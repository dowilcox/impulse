//! Text for agents (the Swift `TerminalTab.sendToAgent` and `paste`, and
//! the window's `sendToAgent`, `sendToBestAgent` and
//! `sendEditorSelectionToAgent`): a prompt typed into an agent's prompt as
//! a paste, so its newlines don't submit, and left there for the user to
//! read and send. While the agent is mid-turn it waits, and goes in when
//! the turn ends. Problems, review comments, conflicts, command blocks and
//! the editor's selection get here, each to an agent the user picks (or
//! the one most likely meant).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{App, Context, Entity, EntityId, Window};
use impulse_kit::agents::state_machine::{AgentEvent, AgentState};
use impulse_kit::shell::quoting::{is_control, without_control_characters};
use impulse_terminal::TerminalMode;

use crate::agent_tracker::AgentSignal;
use crate::agent_views::AgentSummary;
use crate::tasks::{toast, ToastKind};
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::Kind;

/// What goes to a program as a paste: in bracketed-paste marks when it
/// asks for them, without control characters but tab and line ends (so
/// the end mark can't appear inside); unbracketed, a control character is
/// a key press (a `\r` in captured progress output would submit early), so
/// only tab and newline stay.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let body: String = text
            .chars()
            .filter(|c| matches!(c, '\t' | '\n' | '\r') || !is_control(*c))
            .collect();
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        without_control_characters(text).into_bytes()
    }
}

/// A command block as an agent reads it: what ran where, how it ended,
/// and its output (the last 200 lines).
pub fn block_description(
    command: Option<&str>,
    output: &str,
    cwd: Option<&str>,
    exit_code: Option<i32>,
) -> String {
    let command = command.map(str::trim).unwrap_or("");
    let mut lines: Vec<&str> = output.trim_matches('\n').split('\n').collect();
    let mut note = String::new();
    if lines.len() > 200 {
        note = format!("(last 200 of {} lines)\n", lines.len());
        lines = lines.split_off(lines.len() - 200);
    }
    let mut text = format!(
        "I ran `{}`",
        if command.is_empty() {
            "a command"
        } else {
            command
        }
    );
    if let Some(cwd) = cwd.filter(|c| !c.is_empty()) {
        text += &format!(" in {cwd}");
    }
    match exit_code {
        Some(0) => text += " (it succeeded)",
        Some(code) => text += &format!(" (exit status {code})"),
        None => {}
    }
    text + &format!(". Output:\n{note}```\n{}\n```\n", lines.join("\n"))
}

/// The editor's selection as a prompt: where it is, then the code.
pub fn selection_prompt(path: &str, start: u32, end: u32, language: &str, text: &str) -> String {
    let lines = if start == end {
        format!("line {start}")
    } else {
        format!("lines {start}–{end}")
    };
    format!("In @{path}, {lines}:\n```{language}\n{text}\n```\n")
}

/// The toast after sending: queued while the agent is busy, else typed in.
pub fn sent_message(agent: &str, busy: bool) -> String {
    if busy {
        format!("Queued for {agent}; it's sent when the current turn ends.")
    } else {
        format!("Sent to {agent}. Review it in its prompt, then press Return.")
    }
}

impl TerminalView {
    /// Type `text` into the agent's prompt as a paste, without pressing
    /// Return. While the agent is mid-turn it waits until the turn ends.
    /// False when there's no agent.
    pub(crate) fn send_to_agent(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.agent.state() else {
            return false;
        };
        if matches!(state, AgentState::Working | AgentState::NeedsInput) {
            self.agent.pending_text.push(text.to_string());
        } else {
            self.paste_to_program(text);
        }
        cx.notify();
        true
    }

    /// Paste text into the program now, optionally pressing Return after
    /// it (a moment later, so the program has taken the paste).
    pub(crate) fn paste_into_program(&mut self, text: &str, submit: bool, cx: &mut Context<Self>) {
        self.paste_to_program(text);
        if !submit {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let _ = this.update(cx, |view, cx| {
                view.backend.write(b"\r");
                view.agent_event(AgentEvent::Submit, cx);
            });
        })
        .detach();
    }

    fn paste_to_program(&mut self, text: &str) {
        let bracketed = self.backend.mode().contains(TerminalMode::BRACKETED_PASTE);
        self.backend.write(&paste_bytes(text, bracketed));
    }

    /// Deliver text queued while the agent was busy.
    pub(crate) fn flush_pending_agent_text(&mut self) {
        if self.agent.pending_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.agent.pending_text).join("\n\n");
        self.paste_to_program(&text);
    }

    /// A command block as a prompt.
    pub(crate) fn block_for_agent(&self, id: u64) -> Option<String> {
        let block = self.backend.command_block(id)?;
        let cwd = block
            .cwd
            .clone()
            .or_else(|| (!self.cwd.is_empty()).then(|| self.cwd.clone()));
        Some(block_description(
            block.command.as_deref(),
            &block.output,
            cwd.as_deref(),
            block.exit_code,
        ))
    }

    /// Send to Agent on command blocks: the window picks the agent (not
    /// this terminal's own).
    pub(crate) fn send_blocks_to_agent(&mut self, ids: &[u64], cx: &mut Context<Self>) {
        let texts: Vec<String> = ids
            .iter()
            .filter_map(|id| self.block_for_agent(*id))
            .collect();
        if !texts.is_empty() {
            cx.emit(AgentSignal::SendToAgent(texts.join("\n")));
        }
    }
}

impl Workbench {
    /// Agents that could take a prompt.
    pub(crate) fn agent_targets(&self, cx: &App) -> Vec<AgentSummary> {
        self.agent_summaries(cx)
            .into_iter()
            .filter(|a| a.state != AgentState::Exited)
            .collect()
    }

    /// Hand `text` to the agent in `terminal`, saying so with a toast that
    /// can show it.
    pub(crate) fn send_to_agent(
        &mut self,
        text: &str,
        terminal: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (name, state) = {
            let tracker = &terminal.read(cx).agent;
            (
                tracker.agent.as_ref().map(|a| a.display_name.clone()),
                tracker.state(),
            )
        };
        let Some(name) = name else {
            return toast(
                window,
                cx,
                ToastKind::Warning,
                "That agent isn't running anymore.".into(),
                None,
                None,
            );
        };
        let busy = matches!(state, Some(AgentState::Working | AgentState::NeedsInput));
        terminal.update(cx, |t, cx| t.send_to_agent(text, cx));
        let show: crate::tasks::Run = {
            let (workbench, terminal) = (cx.entity().downgrade(), terminal.clone());
            Rc::new(move |window, cx| {
                window.activate_window();
                let _ = workbench.update(cx, |w, cx| w.reveal_terminal(&terminal, window, cx));
            })
        };
        toast(
            window,
            cx,
            ToastKind::Success,
            sent_message(&name, busy),
            None,
            Some(("Show", show)),
        );
    }

    /// Send text to the agent most likely meant: one waiting on the user in
    /// the active workspace first, then any other there, then any agent.
    pub(crate) fn send_to_best_agent(
        &mut self,
        text: &str,
        excluding: Option<EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace().name();
        let candidates: Vec<AgentSummary> = self
            .agent_targets(cx)
            .into_iter()
            .filter(|a| Some(a.terminal.entity_id()) != excluding)
            .collect();
        let target = candidates
            .iter()
            .find(|a| a.workspace_name == workspace && a.state.wants_user())
            .or_else(|| candidates.iter().find(|a| a.workspace_name == workspace))
            .or(candidates.first())
            .map(|a| a.terminal.clone());
        match target {
            Some(terminal) => self.send_to_agent(text, &terminal, window, cx),
            None => toast(
                window,
                cx,
                ToastKind::Info,
                "No agent is running in this window.".into(),
                None,
                None,
            ),
        }
    }

    /// Send Selection to Agent: the editor's selection, with where it came
    /// from.
    pub(crate) fn send_editor_selection_to_agent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.active_editor().and_then(|editor| {
            let editor = editor.read(cx);
            let (text, start, end) = editor.selected_text(cx)?;
            Some((
                text,
                start,
                end,
                editor.language.clone(),
                editor.path.to_string_lossy().into_owned(),
            ))
        });
        let Some((text, start, end, language, path)) = selection else {
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Select some code in an editor first.".into(),
                None,
                None,
            );
        };
        let path = self.relative_to_workspace(&path);
        let prompt = selection_prompt(&path, start, end, &language, &text);
        self.send_to_best_agent(&prompt, None, window, cx);
    }

    /// `path` relative to the active workspace (or its repository).
    pub(crate) fn relative_to_workspace(&self, path: &str) -> String {
        let workspace = self.workspace();
        let roots = [
            (workspace.kind == Kind::Folder).then(|| workspace.root.to_string_lossy().into_owned()),
            impulse_git::branches::repo_root(path).map(|r| r.trim_end_matches('/').to_string()),
        ];
        for root in roots.into_iter().flatten() {
            if let Some(rest) = path.strip_prefix(&format!("{root}/")) {
                return rest.to_string();
            }
        }
        path.to_string()
    }
}

/// The window's workbench, from a menu (which has only the window).
pub(crate) fn window_workbench(window: &Window, cx: &App) -> Option<Entity<Workbench>> {
    let id = window.window_handle().window_id();
    crate::session::workbenches(cx)
        .into_iter()
        .find(|(_, handle)| handle.window_id() == id)
        .map(|(workbench, _)| workbench)
}

/// What a menu item sends: worked out when it's chosen.
pub(crate) type Prompt = Rc<dyn Fn(&mut Window, &mut App) -> Option<String>>;

/// What runs after a menu item sent its prompt.
pub(crate) type Sent = Rc<dyn Fn(&mut App)>;

/// "{label} {agent} · {tab}" for each agent in the window, then a
/// separator (nothing without agents). `sent` runs after a send.
pub(crate) fn agent_menu_items(
    mut menu: PopupMenu,
    label: &str,
    enabled: bool,
    prompt: Prompt,
    sent: Option<Sent>,
    window: &Window,
    cx: &App,
) -> PopupMenu {
    let Some(workbench) = window_workbench(window, cx) else {
        return menu;
    };
    let agents = workbench.read(cx).agent_targets(cx);
    if agents.is_empty() {
        return menu;
    }
    for agent in agents {
        let (prompt, sent, workbench) = (prompt.clone(), sent.clone(), workbench.downgrade());
        let terminal = agent.terminal.clone();
        let name = format!("{} · {}", agent.agent_name, agent.tab_title);
        menu = menu.item(
            PopupMenuItem::new(if label.is_empty() {
                name
            } else {
                format!("{label} {name}")
            })
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                let Some(text) = prompt(window, cx).filter(|t| !t.is_empty()) else {
                    return;
                };
                let _ = workbench.update(cx, |w, cx| w.send_to_agent(&text, &terminal, window, cx));
                if let Some(sent) = &sent {
                    sent(cx);
                }
            }),
        );
    }
    menu.separator()
}

/// What's in progress, as a conflict prompt names it.
fn operation_name(operation: Option<&impulse_git::RepoOperation>) -> Option<&'static str> {
    use impulse_git::RepoOperation::*;
    match operation? {
        Merge => Some("merge"),
        Rebase { .. } => Some("rebase"),
        CherryPick => Some("cherry-pick"),
        Revert => Some("revert"),
        _ => None,
    }
}

/// The prompt asking an agent to resolve the conflict blocks in `paths`
/// (every conflicted file when empty) of the repository at `root`; `None`
/// when no conflict markers are left.
fn conflict_prompt(
    repository: &crate::repository_state::RepositoryState,
    paths: &[String],
) -> Option<String> {
    let snapshot = repository.snapshot.as_ref();
    let paths: Vec<String> = if paths.is_empty() {
        snapshot
            .map(|s| s.conflicted.iter().map(|c| c.path.clone()).collect())
            .unwrap_or_default()
    } else {
        paths.to_vec()
    };
    let files: Vec<(String, String)> = paths
        .into_iter()
        .filter_map(|path| {
            let text =
                std::fs::read_to_string(std::path::Path::new(&repository.root).join(&path)).ok()?;
            Some((path, text))
        })
        .collect();
    if !files
        .iter()
        .any(|(_, text)| !impulse_kit::git::conflict_prompt::blocks(text).is_empty())
    {
        return None;
    }
    let operation = operation_name(snapshot.and_then(|s| s.operation.as_ref()));
    Some(impulse_kit::git::conflict_prompt::make(
        &files,
        operation,
        impulse_kit::git::conflict_prompt::DEFAULT_LIMIT,
    ))
}

/// Ask Agent to Resolve: an item per agent, then Copy as Prompt, for the
/// conflicts in `paths` (every conflicted file when empty).
pub(crate) fn conflict_menu_items(
    menu: PopupMenu,
    label: &str,
    repository: Entity<crate::repository_state::RepositoryState>,
    paths: Vec<String>,
    window: &Window,
    cx: &App,
) -> PopupMenu {
    let prompt: Prompt = {
        let (repository, paths) = (repository.clone(), paths.clone());
        Rc::new(move |window, cx| {
            let prompt = conflict_prompt(repository.read(cx), &paths);
            if prompt.is_none() {
                toast(
                    window,
                    cx,
                    ToastKind::Info,
                    "No conflict markers left. Mark the files resolved.".into(),
                    None,
                    None,
                );
            }
            prompt
        })
    };
    let copy = prompt.clone();
    agent_menu_items(menu, label, true, prompt, None, window, cx).item(
        PopupMenuItem::new("Copy as Prompt").on_click(move |_, window, cx| {
            if let Some(prompt) = copy(window, cx) {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(prompt));
                toast(
                    window,
                    cx,
                    ToastKind::Success,
                    "Copied a prompt to resolve the conflicts".into(),
                    None,
                    None,
                );
            }
        }),
    )
}

/// When each repository was last marked reviewed from here.
static REVIEWED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// Review comments went to an agent: mark the repository reviewed up to
/// now (Review's "Since my last review"), at most every 10 seconds.
pub(crate) fn mark_reviewed(root: &str) {
    {
        let mut reviewed = REVIEWED.lock().unwrap_or_else(|e| e.into_inner());
        if reviewed
            .get(root)
            .is_some_and(|last| last.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        reviewed.insert(root.to_string(), Instant::now());
    }
    let root = root.to_string();
    std::thread::spawn(move || {
        use impulse_git::safety;
        let trusted = crate::workbench_trust::is_trusted(&root);
        if safety::create_with("reviewed", &root, safety::REVIEW_PREFIX, trusted).is_ok() {
            safety::prune(&root, safety::REVIEW_PREFIX, 20, safety::DEFAULT_MAX_AGE);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pastes_keep_line_ends_only_when_bracketed() {
        assert_eq!(
            paste_bytes("fix\x1b[201~ this\r\nnow\x07", true),
            b"\x1b[200~fix[201~ this\r\nnow\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes("a\tb\r\nc\x03", false), b"a\tb\nc".to_vec());
    }

    #[test]
    fn blocks_read_like_the_swift_app() {
        assert_eq!(
            block_description(Some(" cargo test "), "\nok\n", Some("/repo"), Some(0)),
            "I ran `cargo test` in /repo (it succeeded). Output:\n```\nok\n```\n"
        );
        assert_eq!(
            block_description(None, "boom", None, Some(2)),
            "I ran `a command` (exit status 2). Output:\n```\nboom\n```\n"
        );
        let long: Vec<String> = (1..=250).map(|n| n.to_string()).collect();
        let text = block_description(Some("seq"), &long.join("\n"), None, None);
        assert!(text.starts_with("I ran `seq`. Output:\n(last 200 of 250 lines)\n```\n51\n"));
        assert!(text.ends_with("250\n```\n"));
    }

    #[test]
    fn selections_say_where_they_are() {
        assert_eq!(
            selection_prompt("src/main.rs", 3, 3, "rust", "fn main() {}"),
            "In @src/main.rs, line 3:\n```rust\nfn main() {}\n```\n"
        );
        assert!(selection_prompt("a.py", 1, 4, "python", "x").starts_with("In @a.py, lines 1–4:"));
    }

    #[test]
    fn the_toast_says_whether_it_waits() {
        assert_eq!(
            sent_message("Claude Code", true),
            "Queued for Claude Code; it's sent when the current turn ends."
        );
        assert_eq!(
            sent_message("Codex", false),
            "Sent to Codex. Review it in its prompt, then press Return."
        );
    }
}
