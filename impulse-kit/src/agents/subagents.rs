//! The subagents an agent runs (Claude Code's Agent tool), from its hooks:
//! each one's type, what it was asked to do, the worktree it works in when
//! it has its own, whether it's still running and what it reported. And
//! who last edited each file, so agents changing one file in one checkout
//! at the same time can be told.
//!
//! The hooks arrive in this order for each Agent call: PreToolUse (with
//! the call's description), SubagentStart (with the subagent's id), and
//! PostToolUse (with both), which for a subagent in the background comes
//! right away and for one in the foreground once it's done. A subagent
//! with its own worktree may start only after its PostToolUse, once the
//! worktree is made. A Stop lists the subagents still running in the
//! background, and each one that finishes there is reported to the agent
//! in a turn of its own (a UserPromptSubmit with its task-notification).

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Subagent types that only read: they don't change a checkout they share.
pub const READ_ONLY_TYPES: [&str; 3] = ["Explore", "Plan", "claude-code-guide"];

/// The folder Claude Code makes a subagent's own worktree in
/// (`isolation: "worktree"`): `<repo>/.claude/worktrees/<name>`.
pub const CLAUDE_WORKTREES: &str = ".claude/worktrees";

/// The longest result kept for a subagent.
const RESULT_LIMIT: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subagent {
    pub id: String,
    /// "general-purpose", "Explore", a custom agent's name.
    pub agent_type: String,
    /// What it was asked to do, in the Agent call's words ("Fix the parser").
    pub description: Option<String>,
    /// Its own worktree, when it has one.
    pub worktree: Option<String>,
    pub started: Instant,
    /// When it stopped; `None` while it runs.
    pub finished: Option<Instant>,
    /// Its last message, once it stopped.
    pub result: Option<String>,
    /// The last tool it used that Impulse heard of.
    pub last_tool: Option<String>,
    /// It runs in the background: the agent hears that it finished in a
    /// turn of its own.
    pub background: bool,
    /// That turn began.
    pub reported: bool,
}

impl Subagent {
    pub fn is_running(&self) -> bool {
        self.finished.is_none()
    }

    /// It finished in the background, and the agent hasn't been told yet.
    pub fn awaits_report(&self) -> bool {
        self.background && !self.is_running() && !self.reported
    }

    /// Whether it may change the checkout its agent is in.
    pub fn edits_shared_checkout(&self) -> bool {
        self.worktree.is_none() && !is_read_only(&self.agent_type)
    }

    /// Its description, or its type when the call had none.
    pub fn label(&self) -> String {
        self.description
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| self.agent_type.clone())
    }
}

pub fn is_read_only(agent_type: &str) -> bool {
    READ_ONLY_TYPES.contains(&agent_type)
}

/// The worktree a subagent works in, from the folder its hooks report:
/// one Claude Code made for it, or `None` for its agent's own folder.
pub fn worktree_of(cwd: &str) -> Option<String> {
    let at = cwd.find(&format!("/{CLAUDE_WORKTREES}/"))?;
    let rest = &cwd[at + CLAUDE_WORKTREES.len() + 2..];
    let name = rest.split('/').next().filter(|n| !n.is_empty())?;
    Some(format!("{}/{CLAUDE_WORKTREES}/{name}", &cwd[..at]))
}

/// Two agents changing the same file of one checkout at the same time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clash {
    /// The agent edited a file its subagent (named), still running, edited
    /// too.
    WithRunningSubagent(String),
    /// A subagent edited a file someone else changed while it was running:
    /// another subagent (named), or `None` for the agent that started it.
    WhileRunning(Option<String>),
}

impl Clash {
    /// What the agent that just edited `file` is told.
    pub fn note(&self, file: &str) -> String {
        match self {
            Clash::WithRunningSubagent(other) => format!(
                "Impulse: your subagent \"{other}\" also edited {file} and is still running in this checkout. Edits made at the same time overwrite each other: let it finish before changing this file again."
            ),
            Clash::WhileRunning(other) => format!(
                "Impulse: {file} was also edited by {} while you were working, in this same checkout. Read the file again before changing it, so neither of you overwrites the other, and mention it when you report back.",
                match other {
                    Some(other) => format!("another subagent (\"{other}\")"),
                    None => "the agent that started you".into(),
                }
            ),
        }
    }
}

/// Who edited a file: a subagent, by its id, or the agent itself (`None`).
type Editor = Option<String>;

#[derive(Clone, Debug, Default)]
pub struct Subagents {
    /// The latest batch: the ones running and the ones that finished since
    /// none were.
    list: Vec<Subagent>,
    /// The descriptions of Agent calls whose subagent hasn't started yet.
    pending: VecDeque<Option<String>>,
    /// Calls that returned before their subagent started (its worktree
    /// being made): its description and whether it runs in the
    /// background, by its id.
    launched_early: HashMap<String, (Option<String>, bool)>,
    /// Each file's last edit in this batch (absolute paths).
    edits: HashMap<String, (Editor, Instant)>,
    /// This batch's agent was told about sharing its checkout.
    told: bool,
}

impl Subagents {
    pub fn all(&self) -> &[Subagent] {
        &self.list
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn running(&self) -> impl Iterator<Item = &Subagent> {
        self.list.iter().filter(|s| s.is_running())
    }

    pub fn running_count(&self) -> usize {
        self.running().count()
    }

    fn find(&self, id: &str) -> Option<&Subagent> {
        self.list.iter().find(|s| s.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Subagent> {
        self.list.iter_mut().find(|s| s.id == id)
    }

    /// A new session: nothing from the last one carries over.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A turn began or ended: calls that never started a subagent (denied,
    /// or failed) are forgotten.
    pub fn new_turn(&mut self) {
        self.pending.clear();
    }

    /// The agent was told these subagents finished (a turn Claude Code
    /// starts for itself).
    pub fn reported(&mut self, ids: &[&str]) {
        for subagent in &mut self.list {
            if ids.contains(&subagent.id.as_str()) {
                subagent.reported = true;
            }
        }
    }

    /// Subagents that finished in the background but whose report the
    /// agent hasn't had: it takes another turn for each.
    pub fn awaiting_reports(&self) -> usize {
        self.list.iter().filter(|s| s.awaits_report()).count()
    }

    /// The agent is about to make an Agent call (PreToolUse).
    pub fn launching(&mut self, description: Option<&str>) {
        self.pending
            .push_back(description.map(str::to_string).filter(|d| !d.is_empty()));
    }

    /// A subagent started (SubagentStart), in `cwd`. One starting with
    /// none running, nor any finished one still to be reported, begins a
    /// new batch.
    pub fn started(&mut self, id: &str, agent_type: &str, cwd: Option<&str>, now: Instant) {
        if self.running_count() == 0 && self.awaiting_reports() == 0 && self.find(id).is_none() {
            self.list.clear();
            self.edits.clear();
            self.told = false;
        }
        let (description, background) = match self.launched_early.remove(id) {
            Some(early) => early,
            None => (self.pending.pop_front().flatten(), false),
        };
        if let Some(existing) = self.find_mut(id) {
            // Resumed (SendMessage to a subagent that finished).
            existing.finished = None;
            existing.result = None;
            existing.reported = false;
            existing.started = now;
            return;
        }
        self.list.push(Subagent {
            id: id.to_string(),
            agent_type: agent_type.to_string(),
            description,
            worktree: cwd.and_then(worktree_of),
            started: now,
            finished: None,
            result: None,
            last_tool: None,
            background,
            reported: false,
        });
    }

    /// The Agent call returned (PostToolUse) with the subagent's id, its
    /// description (for certain) and whether it went on in the background.
    pub fn launched(&mut self, id: &str, description: Option<&str>, background: bool) {
        let description = description.filter(|d| !d.is_empty()).map(str::to_string);
        if let Some(subagent) = self.find_mut(id) {
            subagent.background |= background;
            if description.is_some() {
                subagent.description = description;
            }
            return;
        }
        // It hasn't started yet: its description is no other's to take.
        if let Some(at) = self.pending.iter().position(|d| *d == description) {
            self.pending.remove(at);
        }
        self.launched_early
            .insert(id.to_string(), (description, background));
    }

    /// A subagent stopped (SubagentStop), with its last message.
    pub fn stopped(&mut self, id: &str, result: Option<&str>, now: Instant) {
        if let Some(subagent) = self.find_mut(id) {
            subagent.finished = Some(now);
            subagent.result = result.map(|r| {
                let mut text: String = r.trim().chars().take(RESULT_LIMIT).collect();
                if r.trim().chars().count() > RESULT_LIMIT {
                    text.push('…');
                }
                text
            });
        }
    }

    /// A subagent used `tool`.
    pub fn used_tool(&mut self, id: &str, tool: &str) {
        if let Some(subagent) = self.find_mut(id) {
            subagent.last_tool = Some(tool.to_string());
        }
    }

    /// A hook of subagent `id` ran in `cwd`: its own worktree, when
    /// SubagentStart didn't name one and `cwd` is in a worktree other than
    /// the one its agent works in (`agent_cwd`).
    pub fn worked_in(&mut self, id: &str, cwd: &str, agent_cwd: &str) {
        let worktree = worktree_of(cwd).filter(|w| worktree_of(agent_cwd).as_ref() != Some(w));
        if let Some(subagent) = self.find_mut(id).filter(|s| s.worktree.is_none()) {
            subagent.worktree = worktree;
        }
    }

    /// The agent's turn ended (Stop) with these subagents (id, type,
    /// description) still running in the background: any other one that
    /// seems to be running has stopped without saying so.
    pub fn still_running(&mut self, running: &[(String, String, String)], now: Instant) {
        for subagent in &mut self.list {
            let still = running.iter().any(|(id, _, _)| *id == subagent.id);
            subagent.background |= still;
            if subagent.is_running() && !still {
                subagent.finished = Some(now);
            }
        }
        for (id, agent_type, description) in running {
            if self.find(id).is_none() {
                self.list.push(Subagent {
                    id: id.clone(),
                    agent_type: agent_type.clone(),
                    description: Some(description.clone()).filter(|d| !d.is_empty()),
                    worktree: None,
                    started: now,
                    finished: None,
                    result: None,
                    last_tool: None,
                    background: true,
                    reported: false,
                });
            }
        }
        self.pending.clear();
    }

    /// The running subagents, other than `excluding`, that may change the
    /// agent's checkout.
    fn shared_checkout_writers(&self, excluding: Option<&str>) -> Vec<&Subagent> {
        self.running()
            .filter(|s| s.edits_shared_checkout() && Some(s.id.as_str()) != excluding)
            .collect()
    }

    /// What to tell the agent as it makes an Agent call (PreToolUse) that
    /// gives the subagent no folder of its own (`isolated`: its own
    /// worktree or `cwd`), while others that edit run in the same checkout.
    /// Once per batch.
    pub fn launch_note(&mut self, agent_type: Option<&str>, isolated: bool) -> Option<String> {
        if isolated || self.told || is_read_only(agent_type.unwrap_or("general-purpose")) {
            return None;
        }
        let others = self.shared_checkout_writers(None);
        if others.is_empty() {
            return None;
        }
        let note = format!(
            "Impulse: this subagent shares the checkout with {} other subagent{} still running ({}). Subagents editing one checkout at the same time can overwrite each other's changes. Give subagents that edit files their own worktree with isolation: \"worktree\" on the Agent call: Impulse lists those worktrees as tasks the user can review and merge.",
            others.len(),
            if others.len() == 1 { "" } else { "s" },
            quoted_list(&others),
        );
        self.told = true;
        Some(note)
    }

    /// What to tell the subagent `id` as it starts (SubagentStart), when it
    /// may edit a checkout others are editing too.
    pub fn start_note(&self, id: &str) -> Option<String> {
        if !self.find(id)?.edits_shared_checkout() {
            return None;
        }
        let others = self.shared_checkout_writers(Some(id));
        if others.is_empty() {
            return None;
        }
        Some(format!(
            "Impulse: other subagents are working in this same checkout at the same time ({}). Change only the files your task needs, and read a file again right before editing it: another agent may have changed it.",
            quoted_list(&others)
        ))
    }

    /// `editor` (a subagent's id, or `None` for the agent) edited `file`
    /// (an absolute path); the clash it makes, if any.
    pub fn edited(&mut self, editor: Option<&str>, file: &str, now: Instant) -> Option<Clash> {
        let editor: Editor = editor.map(str::to_string);
        let (other, at) = self.edits.insert(file.to_string(), (editor.clone(), now))?;
        if other == editor {
            return None;
        }
        match &editor {
            None => {
                let subagent = self.find(other.as_deref()?)?;
                (subagent.is_running() && subagent.worktree.is_none())
                    .then(|| Clash::WithRunningSubagent(subagent.label()))
            }
            Some(id) => {
                // Changed before it started: it read the file as it is.
                if at < self.find(id)?.started {
                    return None;
                }
                let other = match other {
                    Some(other) => Some(self.find(&other)?.label()),
                    None => None,
                };
                Some(Clash::WhileRunning(other))
            }
        }
    }

    /// "3 subagents running" for the inbox; `None` when none run.
    pub fn summary(&self) -> Option<String> {
        match self.running_count() {
            0 => None,
            1 => Some("1 subagent running".into()),
            n => Some(format!("{n} subagents running")),
        }
    }
}

/// `"write b", "write c"`.
fn quoted_list(subagents: &[&Subagent]) -> String {
    subagents
        .iter()
        .map(|s| format!("\"{}\"", s.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    /// Two subagents started the way Claude Code starts them, from one
    /// message: each call's PreToolUse, then its SubagentStart.
    fn two(start: Instant) -> Subagents {
        let mut list = Subagents::default();
        list.launching(Some("write b"));
        list.started("b", "general-purpose", Some("/r/app"), start);
        list.launching(Some("write c"));
        list.started("c", "general-purpose", Some("/r/app"), start);
        list
    }

    #[test]
    fn calls_and_starts_pair_up_in_order() {
        let start = Instant::now();
        let mut list = Subagents::default();
        list.launching(Some("look around"));
        list.launching(Some("fix it"));
        list.started("x", "Explore", Some("/r/app"), start);
        list.started(
            "y",
            "general-purpose",
            Some("/r/app/.claude/worktrees/agent-y/src"),
            start,
        );
        let labels: Vec<String> = list.all().iter().map(Subagent::label).collect();
        assert_eq!(labels, ["look around", "fix it"]);
        assert_eq!(
            list.all()[1].worktree.as_deref(),
            Some("/r/app/.claude/worktrees/agent-y")
        );
        // The call's own answer has the last word.
        list.launched("y", Some("fix the parser"), false);
        assert_eq!(list.all()[1].label(), "fix the parser");
        assert_eq!(list.summary().as_deref(), Some("2 subagents running"));
    }

    #[test]
    fn a_worktree_shows_in_later_hooks() {
        let start = Instant::now();
        let mut list = Subagents::default();
        list.started("w", "general-purpose", Some("/r/app"), start);
        list.started("v", "general-purpose", Some("/r/app"), start);
        list.worked_in("w", "/r/app/.claude/worktrees/agent-w/src", "/r/app");
        assert_eq!(
            list.all()[0].worktree.as_deref(),
            Some("/r/app/.claude/worktrees/agent-w")
        );
        assert!(!list.all()[0].edits_shared_checkout());
        // The agent's own worktree isn't the subagent's.
        let own = "/r/app/.claude/worktrees/session";
        list.worked_in("v", own, own);
        assert_eq!(list.all()[1].worktree, None);
        // The first worktree it was seen in stays.
        list.worked_in("w", "/r/app/.claude/worktrees/other", "/r/app");
        assert_eq!(
            list.all()[0].worktree.as_deref(),
            Some("/r/app/.claude/worktrees/agent-w")
        );
    }

    #[test]
    fn a_new_batch_starts_when_none_run() {
        let start = Instant::now();
        let mut list = two(start);
        list.stopped("b", Some("  done\n"), at(start, 5));
        assert_eq!(list.all()[0].result.as_deref(), Some("done"));
        // Another while one runs joins the batch...
        list.launching(None);
        list.started("d", "general-purpose", None, at(start, 6));
        assert_eq!(list.all().len(), 3);
        assert_eq!(list.all()[2].label(), "general-purpose");
        list.stopped("c", None, at(start, 7));
        list.stopped("d", None, at(start, 7));
        assert_eq!(list.summary(), None);
        // ...one after all finished starts over.
        list.launching(Some("next"));
        list.started("e", "general-purpose", None, at(start, 9));
        assert_eq!(list.all().len(), 1);
    }

    #[test]
    fn a_call_that_returns_before_its_subagent_starts_keeps_its_name() {
        let start = Instant::now();
        let mut list = Subagents::default();
        // The worktree one returns first, and the next call's subagent
        // starts before it does.
        list.launching(Some("own tree"));
        list.launched("w", Some("own tree"), true);
        list.launching(Some("shared"));
        list.started("s", "general-purpose", Some("/r/app"), start);
        list.started(
            "w",
            "general-purpose",
            Some("/r/app/.claude/worktrees/agent-w"),
            start,
        );
        let labels: Vec<(String, bool)> = list
            .all()
            .iter()
            .map(|s| (s.label(), s.background))
            .collect();
        assert_eq!(
            labels,
            [("shared".into(), false), ("own tree".into(), true)]
        );
    }

    #[test]
    fn finishing_in_the_background_awaits_its_report() {
        let start = Instant::now();
        let mut list = two(start);
        list.launched("b", Some("write b"), true);
        list.launched("c", Some("write c"), true);
        list.stopped("b", None, at(start, 2));
        list.stopped("c", None, at(start, 3));
        assert_eq!(list.awaiting_reports(), 2);
        // A subagent started before they're reported joins their batch.
        list.launching(Some("more"));
        list.started("d", "Explore", None, at(start, 4));
        assert_eq!(list.all().len(), 3);
        list.reported(&["b"]);
        assert_eq!(list.awaiting_reports(), 1);
        list.reported(&["c"]);
        assert_eq!(list.awaiting_reports(), 0);
        // Resumed, it will be reported again.
        list.started("b", "general-purpose", None, at(start, 5));
        list.stopped("b", None, at(start, 6));
        assert_eq!(list.awaiting_reports(), 1);
    }

    #[test]
    fn a_stop_says_which_are_still_running() {
        let start = Instant::now();
        let mut list = two(start);
        list.still_running(
            &[
                ("c".into(), "general-purpose".into(), "write c".into()),
                ("z".into(), "Plan".into(), "plan it".into()),
            ],
            at(start, 3),
        );
        let running: Vec<&str> = list.running().map(|s| s.id.as_str()).collect();
        assert_eq!(running, ["c", "z"]);
        assert_eq!(list.all()[2].label(), "plan it");
    }

    #[test]
    fn worktrees_are_claude_codes_folders() {
        assert_eq!(
            worktree_of("/r/app/.claude/worktrees/agent-a1"),
            Some("/r/app/.claude/worktrees/agent-a1".into())
        );
        assert_eq!(worktree_of("/r/app/.claude/worktrees/"), None);
        assert_eq!(worktree_of("/r/app"), None);
    }

    #[test]
    fn edits_at_the_same_time_clash() {
        let start = Instant::now();
        let mut list = two(start);
        assert_eq!(list.edited(Some("b"), "/r/app/x.rs", at(start, 1)), None);
        // The same agent again is no clash.
        assert_eq!(list.edited(Some("b"), "/r/app/x.rs", at(start, 2)), None);
        assert_eq!(
            list.edited(Some("c"), "/r/app/x.rs", at(start, 3)),
            Some(Clash::WhileRunning(Some("write b".into())))
        );
        // The agent itself, while c still runs, and c after it.
        assert_eq!(
            list.edited(None, "/r/app/x.rs", at(start, 4)),
            Some(Clash::WithRunningSubagent("write c".into()))
        );
        assert_eq!(
            list.edited(Some("c"), "/r/app/x.rs", at(start, 5)),
            Some(Clash::WhileRunning(None))
        );
        // Once c is done, the agent taking its work over is no clash.
        list.edited(Some("c"), "/r/app/y.rs", at(start, 7));
        list.stopped("c", None, at(start, 8));
        assert_eq!(list.edited(None, "/r/app/y.rs", at(start, 9)), None);
    }

    #[test]
    fn edits_before_a_subagent_started_dont_clash() {
        let start = Instant::now();
        let mut list = Subagents::default();
        list.edited(None, "/r/app/x.rs", start);
        list.launching(Some("write b"));
        list.started("b", "general-purpose", None, at(start, 2));
        // The batch starts afresh, and the file was changed before b began.
        assert_eq!(list.edited(Some("b"), "/r/app/x.rs", at(start, 3)), None);
    }

    #[test]
    fn launching_beside_editors_says_so_once() {
        let start = Instant::now();
        let mut list = Subagents::default();
        list.launching(Some("look"));
        list.started("x", "Explore", None, start);
        // Readers don't count.
        assert_eq!(list.launch_note(None, false), None);
        list.launching(Some("write b"));
        list.started("b", "general-purpose", None, start);
        assert_eq!(list.launch_note(Some("Explore"), false), None);
        assert_eq!(list.launch_note(None, true), None);
        let note = list.launch_note(None, false).unwrap();
        assert!(note.starts_with(
            "Impulse: this subagent shares the checkout with 1 other subagent still running (\"write b\")."
        ));
        assert_eq!(list.launch_note(None, false), None);
    }

    #[test]
    fn subagents_starting_beside_editors_are_told() {
        let start = Instant::now();
        let mut list = two(start);
        assert_eq!(list.start_note("b").as_deref(), Some("Impulse: other subagents are working in this same checkout at the same time (\"write c\"). Change only the files your task needs, and read a file again right before editing it: another agent may have changed it."));
        list.launching(Some("own tree"));
        list.started(
            "w",
            "general-purpose",
            Some("/r/app/.claude/worktrees/w"),
            start,
        );
        assert_eq!(list.start_note("w"), None);
    }

    #[test]
    fn notes_name_who_else_edited() {
        assert_eq!(
            Clash::WhileRunning(None).note("src/x.rs"),
            "Impulse: src/x.rs was also edited by the agent that started you while you were working, in this same checkout. Read the file again before changing it, so neither of you overwrites the other, and mention it when you report back."
        );
        assert!(Clash::WithRunningSubagent("write c".into())
            .note("x.rs")
            .starts_with("Impulse: your subagent \"write c\" also edited x.rs"));
    }
}
