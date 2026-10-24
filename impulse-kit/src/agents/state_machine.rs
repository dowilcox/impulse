//! Tracks what a coding agent in a terminal is doing (working, waiting on
//! the user, finished) from the signals a terminal can see. Hooks (when the
//! agent is set up to call Impulse) are authoritative; without them the
//! machine combines progress reports (OSC 9;4), notifications, bells, and
//! how output flows relative to the user's typing. Even with hooks, a turn
//! or question dismissed with Esc never reports that it stopped: an agent
//! that spins its title (Claude Code's ◐/◑) stopped working when the
//! spinner stopped and no report followed; otherwise a long silence ends
//! it. State reported with `impulse status` is left alone.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    /// Running, nothing asked of it yet (or its result was seen).
    Idle,
    Working,
    /// Blocked on the user mid-turn (a permission prompt, a question).
    NeedsInput,
    /// Finished a turn the user hasn't looked at yet.
    Done,
    Exited,
}

impl AgentState {
    /// Whether the user is wanted.
    pub fn wants_user(self) -> bool {
        matches!(self, AgentState::NeedsInput | AgentState::Done)
    }
}

/// What an agent reported through its hooks (Claude Code hooks, Codex
/// notify, the `impulse` CLI).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentHook {
    SessionStarted,
    PromptSubmitted,
    /// A tool finished (named, when the hook says). The answer to a pending
    /// approval, even one given without Return; otherwise no change.
    UsedTool(Option<String>),
    Stopped,
    /// The turn ended on an error (Claude Code's StopFailure: a rate limit,
    /// an overloaded API, ...), described when the hook says.
    Failed(String),
    Notification(String),
    /// What `impulse status` reported, kept until the next report.
    Status(AgentState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// The program wrote to the terminal.
    Output,
    /// The user typed into the program.
    Keystroke,
    /// The user pressed Return (sent a prompt or answered a question).
    Submit,
    /// OSC 9;4 progress shown (true) or cleared (false).
    Progress(bool),
    Title(String),
    /// A desktop-notification request (OSC 9/777/99), title and body joined.
    Notification(String),
    Bell,
    Hook(AgentHook),
    /// The user looked at the pane: a finished turn has been seen.
    Acknowledged,
    Exited,
}

#[derive(Clone, Debug)]
pub struct AgentStateMachine {
    state: AgentState,
    /// When `state` last changed.
    since: Instant,

    /// Output must keep coming this long (a spinner, streaming text) before
    /// it counts as working...
    pub streak_to_work: Duration,
    /// ...and gaps longer than this end a streak.
    pub streak_gap: Duration,
    /// Output right after a keystroke is just echo.
    pub echo_window: Duration,
    /// Working with no output for this long means the turn ended.
    pub silence_to_done: Duration,
    /// With progress reports, wait this long before trusting silence.
    pub silence_with_progress: Duration,
    /// With hooks, which report the end of a turn, this long without output
    /// or a report means the turn was interrupted (Esc sends no Stop).
    pub silence_with_hooks: Duration,
    /// ...and this long means a question was dismissed. Much longer: one
    /// left unanswered while you're away should still say it needs you.
    pub silence_while_asking: Duration,
    /// With hooks, a title spinner that stopped this long ago without a
    /// report means the turn was interrupted. Longer than Claude Code waits
    /// (6 s) before it reports a permission prompt, which stops it too.
    pub silence_after_spinner: Duration,

    last_output: Option<Instant>,
    last_hook: Option<Instant>,
    streak_start: Option<Instant>,
    last_keystroke: Option<Instant>,
    progress_active: bool,
    saw_progress: bool,
    hooks_active: bool,
    /// The state came from `impulse status`, which reports its own changes.
    reported_by_status: bool,
    /// The tool the pending permission request names ("...to use Bash").
    requested_tool: Option<String>,
    /// Whether the title starts with a spinner now...
    title_spins: bool,
    /// ...and when it last stopped, while it's stopped. Output (a redrawn
    /// prompt, the user typing the next one) then says nothing about work.
    spinner_stopped: Option<Instant>,
    /// A timeout ended the turn while the spinner was stopped: the spinner
    /// starting again (a question that sent no report was answered) means
    /// the agent went on.
    ended_by_spinner: bool,
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

/// `later - earlier`, or zero when `later` comes first.
fn elapsed(later: Instant, earlier: Instant) -> Duration {
    later.saturating_duration_since(earlier)
}

impl AgentStateMachine {
    pub fn new(now: Instant) -> Self {
        Self {
            state: AgentState::Idle,
            since: now,
            streak_to_work: seconds(1.5),
            streak_gap: seconds(0.6),
            echo_window: seconds(1.0),
            silence_to_done: seconds(5.0),
            silence_with_progress: seconds(120.0),
            silence_with_hooks: seconds(300.0),
            silence_while_asking: seconds(1800.0),
            silence_after_spinner: seconds(8.0),
            last_output: None,
            last_hook: None,
            streak_start: None,
            last_keystroke: None,
            progress_active: false,
            saw_progress: false,
            hooks_active: false,
            reported_by_status: false,
            requested_tool: None,
            title_spins: false,
            spinner_stopped: None,
            ended_by_spinner: false,
        }
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    /// When `state` last changed.
    pub fn since(&self) -> Instant {
        self.since
    }

    /// Output this soon after a keystroke is its echo.
    fn is_echo(&self, now: Instant) -> bool {
        self.last_keystroke
            .is_some_and(|keystroke| elapsed(now, keystroke) < self.echo_window)
    }

    /// Apply an event. Returns true when `state` changed.
    pub fn handle(&mut self, event: AgentEvent, now: Instant) -> bool {
        if self.state == AgentState::Exited && event != AgentEvent::Exited {
            return false;
        }
        match event {
            AgentEvent::Exited => self.transition(AgentState::Exited, now),

            AgentEvent::Hook(hook) => {
                self.hooks_active = true;
                self.last_hook = Some(now);
                self.reported_by_status = false;
                match hook {
                    AgentHook::SessionStarted => self.transition(AgentState::Idle, now),
                    AgentHook::PromptSubmitted => self.transition(AgentState::Working, now),
                    AgentHook::Stopped | AgentHook::Failed(_) => {
                        self.transition(AgentState::Done, now)
                    }
                    AgentHook::UsedTool(tool) => {
                        // Tools also run in parallel subagents and after Stop
                        // (background tasks): only the one that was waiting
                        // for approval answers it.
                        if self.state != AgentState::NeedsInput {
                            return false;
                        }
                        if let (Some(tool), Some(requested)) = (&tool, &self.requested_tool) {
                            if !requested.eq_ignore_ascii_case(tool) {
                                return false;
                            }
                        }
                        self.transition(AgentState::Working, now)
                    }
                    AgentHook::Notification(text) => {
                        if is_permission_request(&text) {
                            return self.ask(&text, now);
                        }
                        // Other notifications end a turn only while one
                        // runs: Claude Code's "waiting for your input"
                        // reminder comes long after it finished.
                        self.state == AgentState::Working && self.transition(AgentState::Done, now)
                    }
                    AgentHook::Status(reported) => {
                        self.reported_by_status = true;
                        self.requested_tool = None;
                        self.transition(reported, now)
                    }
                }
            }

            AgentEvent::Acknowledged => {
                self.state == AgentState::Done && self.transition(AgentState::Idle, now)
            }

            AgentEvent::Submit => {
                self.last_keystroke = Some(now);
                self.streak_start = None;
                self.transition(AgentState::Working, now)
            }

            AgentEvent::Keystroke => {
                self.last_keystroke = Some(now);
                self.streak_start = None;
                false
            }

            AgentEvent::Progress(active) => {
                self.saw_progress = true;
                self.progress_active = active;
                if self.hooks_active {
                    return false;
                }
                if active {
                    return self.transition(AgentState::Working, now);
                }
                self.state == AgentState::Working && self.transition(AgentState::Done, now)
            }

            AgentEvent::Notification(text) => {
                if is_permission_request(&text) {
                    return self.ask(&text, now);
                }
                if self.hooks_active {
                    return false;
                }
                matches!(self.state, AgentState::Working | AgentState::Idle)
                    && self.transition(AgentState::Done, now)
            }

            AgentEvent::Bell => {
                !self.hooks_active
                    && self.state == AgentState::Working
                    && self.transition(AgentState::Done, now)
            }

            AgentEvent::Title(title) => {
                let spinning = is_spinner(title.chars().next());
                if spinning {
                    self.spinner_stopped = None;
                } else if self.title_spins {
                    self.spinner_stopped = Some(now);
                }
                self.title_spins = spinning;
                if !spinning {
                    return false;
                }
                if self.hooks_active {
                    // Hooks report turns; the spinner tells what they
                    // don't: a question was answered (Claude Code's spinner
                    // stops while it asks).
                    let resumes = self.state == AgentState::NeedsInput
                        || (self.state == AgentState::Idle && self.ended_by_spinner);
                    return !self.reported_by_status
                        && resumes
                        && self.transition(AgentState::Working, now);
                }
                if self.is_echo(now) {
                    return false;
                }
                self.state != AgentState::Working && self.transition(AgentState::Working, now)
            }

            AgentEvent::Output => {
                let changed = self.output(now);
                self.last_output = Some(now);
                changed
            }
        }
    }

    fn output(&mut self, now: Instant) -> bool {
        if self.is_echo(now) {
            self.streak_start = None;
            return false;
        }
        let continues = self.streak_start.is_some()
            && self
                .last_output
                .is_some_and(|last| elapsed(now, last) <= self.streak_gap);
        if !continues {
            self.streak_start = Some(now);
        }
        let Some(streak_start) = self.streak_start else {
            return false;
        };
        if self.hooks_active
            || self.saw_progress
            || matches!(self.state, AgentState::Working | AgentState::NeedsInput)
            || elapsed(now, streak_start) < self.streak_to_work
        {
            return false;
        }
        self.transition(AgentState::Working, now)
    }

    /// Evaluate timeouts. Returns true when `state` changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.next_deadline() else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.progress_active = false;
        // With hooks, a turn that finished would have said so: this one (or
        // its question) was dismissed, by the user, so there's nothing to
        // flag.
        let next = if self.hooks_active {
            AgentState::Idle
        } else {
            AgentState::Done
        };
        let changed = self.transition(next, now);
        self.ended_by_spinner = self.hooks_active && self.spinner_stopped.is_some();
        changed
    }

    /// When `tick` next needs to run, if ever.
    pub fn next_deadline(&self) -> Option<Instant> {
        match self.state {
            AgentState::Working if self.reported_by_status => None,
            AgentState::Working => {
                if let Some(stopped) = self.spinner_stopped {
                    let stopped_since = stopped
                        .max(self.last_hook.unwrap_or(self.since))
                        .max(self.since);
                    let limit = if self.hooks_active {
                        self.silence_after_spinner
                    } else {
                        self.silence_to_done
                    };
                    return Some(stopped_since + limit);
                }
                let limit = if self.hooks_active {
                    self.silence_with_hooks
                } else if self.progress_active || self.saw_progress {
                    self.silence_with_progress
                } else {
                    self.silence_to_done
                };
                Some(self.quiet_since() + limit)
            }
            AgentState::NeedsInput if self.hooks_active && !self.reported_by_status => {
                Some(self.quiet_since() + self.silence_while_asking)
            }
            _ => None,
        }
    }

    /// Silence counts from the last output or hook report, or from when work
    /// started if that's later.
    fn quiet_since(&self) -> Instant {
        self.last_output
            .unwrap_or(self.since)
            .max(self.last_hook.unwrap_or(self.since))
            .max(self.since)
    }

    fn ask(&mut self, text: &str, now: Instant) -> bool {
        self.requested_tool = requested_tool(text);
        self.transition(AgentState::NeedsInput, now)
    }

    fn transition(&mut self, next: AgentState, now: Instant) -> bool {
        if next == self.state {
            return false;
        }
        self.state = next;
        self.since = now;
        self.ended_by_spinner = false;
        if next != AgentState::Working {
            self.streak_start = None;
        }
        true
    }
}

/// "Claude needs your permission to use Bash", "Allow edit?", ...
pub fn is_permission_request(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "permission",
        "approve",
        "approval",
        "allow ",
        "confirm",
        "needs your input",
        "question",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// The tool a permission request names: "Claude needs your permission to
/// use Bash" gives "Bash".
pub fn requested_tool(text: &str) -> Option<String> {
    let start = text.to_ascii_lowercase().find("to use ")? + "to use ".len();
    let name: String = text[start..]
        .chars()
        .take_while(|&c| !c.is_whitespace() && !".,?!:;".contains(c))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Spinner glyphs agents put at the front of the title while they work:
/// braille dots, or Claude Code's ◐ ◓ ◑ ◒ (✳ when it's not working).
pub fn is_spinner(c: Option<char>) -> bool {
    c.is_some_and(|c| {
        ('\u{2801}'..='\u{28FF}').contains(&c) || ('\u{25D0}'..='\u{25D3}').contains(&c)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::hook_events;

    struct Clock(Instant);

    impl Clock {
        fn at(&self, seconds: f64) -> Instant {
            self.0 + Duration::from_millis((seconds * 1000.0).round() as u64)
        }
    }

    fn clock() -> Clock {
        Clock(Instant::now())
    }

    /// Output events every 100 ms over a span, both ends included.
    fn stream(machine: &mut AgentStateMachine, clock: &Clock, from: f64, to: f64) {
        let (start, end) = ((from * 10.0).round() as u64, (to * 10.0).round() as u64);
        for tenth in start..=end {
            machine.handle(AgentEvent::Output, clock.at(tenth as f64 / 10.0));
        }
    }

    fn hook(machine: &mut AgentStateMachine, hook: AgentHook, at: Instant) -> bool {
        machine.handle(AgentEvent::Hook(hook), at)
    }

    fn notification(text: &str) -> AgentHook {
        AgentHook::Notification(text.into())
    }

    fn title(text: &str) -> AgentEvent {
        AgentEvent::Title(text.into())
    }

    #[test]
    fn submitting_a_prompt_starts_work_and_silence_ends_it() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        assert_eq!(machine.state(), AgentState::Idle);
        machine.handle(AgentEvent::Submit, c.at(1.0));
        assert_eq!(machine.state(), AgentState::Working);
        stream(&mut machine, &c, 1.2, 6.0);
        assert!(!machine.tick(c.at(8.0)));
        assert_eq!(machine.next_deadline(), Some(c.at(11.0)));
        assert!(machine.tick(c.at(11.1)));
        assert_eq!(machine.state(), AgentState::Done);
        assert_eq!(machine.since(), c.at(11.1));
        // Looking at it clears "done".
        machine.handle(AgentEvent::Acknowledged, c.at(12.0));
        assert_eq!(machine.state(), AgentState::Idle);
    }

    #[test]
    fn sustained_output_without_typing_means_working() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        stream(&mut machine, &c, 1.0, 2.0);
        assert_eq!(
            machine.state(),
            AgentState::Idle,
            "a short burst isn't work"
        );
        stream(&mut machine, &c, 2.1, 3.0);
        assert_eq!(machine.state(), AgentState::Working);
    }

    #[test]
    fn echo_of_typing_is_not_work() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        let mut t = 1.0;
        while t < 5.0 {
            machine.handle(AgentEvent::Keystroke, c.at(t));
            machine.handle(AgentEvent::Output, c.at(t + 0.05));
            t += 0.3;
        }
        assert_eq!(machine.state(), AgentState::Idle);
    }

    #[test]
    fn progress_reports_are_trusted_over_silence() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        machine.handle(AgentEvent::Progress(true), c.at(1.0));
        assert_eq!(machine.state(), AgentState::Working);
        assert!(!machine.tick(c.at(30.0)), "long thinking without output");
        machine.handle(AgentEvent::Progress(false), c.at(40.0));
        assert_eq!(machine.state(), AgentState::Done);
        // Later output alone doesn't restart work once progress is known.
        stream(&mut machine, &c, 41.0, 45.0);
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn permission_notifications_need_input_others_finish() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        machine.handle(AgentEvent::Submit, c.at(1.0));
        machine.handle(
            AgentEvent::Notification("Claude needs your permission to use Bash".into()),
            c.at(2.0),
        );
        assert_eq!(machine.state(), AgentState::NeedsInput);
        // Output while waiting (the prompt redraws) keeps it waiting.
        stream(&mut machine, &c, 2.5, 5.0);
        assert_eq!(machine.state(), AgentState::NeedsInput);
        machine.handle(AgentEvent::Submit, c.at(6.0));
        assert_eq!(machine.state(), AgentState::Working);
        machine.handle(
            AgentEvent::Notification("Claude is waiting for your input".into()),
            c.at(9.0),
        );
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn bell_ends_a_turn() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        machine.handle(AgentEvent::Bell, c.at(1.0));
        assert_eq!(machine.state(), AgentState::Idle);
        machine.handle(AgentEvent::Submit, c.at(2.0));
        machine.handle(AgentEvent::Bell, c.at(3.0));
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn spinner_titles_mean_working() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        machine.handle(title("✳ Claude Code"), c.at(1.0));
        assert_eq!(machine.state(), AgentState::Idle);
        machine.handle(title("⠋ Refactoring parser"), c.at(2.0));
        assert_eq!(machine.state(), AgentState::Working);

        // Claude Code's own spinner; it stopping ends the turn even while
        // the user types the next prompt.
        let mut claude = AgentStateMachine::new(c.at(0.0));
        claude.handle(title("◐ Refactoring parser"), c.at(1.0));
        assert_eq!(claude.state(), AgentState::Working);
        claude.handle(title("✳ Refactoring parser"), c.at(3.0));
        claude.handle(AgentEvent::Keystroke, c.at(4.0));
        claude.handle(AgentEvent::Output, c.at(4.05));
        assert_eq!(claude.next_deadline(), Some(c.at(8.0)));
        claude.tick(c.at(8.0));
        assert_eq!(claude.state(), AgentState::Done);
    }

    #[test]
    fn hooks_override_heuristics() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::SessionStarted, c.at(0.5));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        assert_eq!(machine.state(), AgentState::Working);
        // Heuristics are ignored once hooks report.
        machine.handle(AgentEvent::Bell, c.at(2.0));
        assert!(!machine.tick(c.at(100.0)));
        assert_eq!(
            machine.next_deadline(),
            Some(c.at(301.0)),
            "only a long silence ends a turn"
        );
        assert_eq!(machine.state(), AgentState::Working);
        hook(
            &mut machine,
            notification("Claude needs your permission to use Edit"),
            c.at(101.0),
        );
        assert_eq!(machine.state(), AgentState::NeedsInput);
        hook(&mut machine, AgentHook::Stopped, c.at(120.0));
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn an_api_error_ends_the_turn() {
        assert_eq!(
            hook_events::hook(
                Some("claude"),
                "StopFailure",
                None,
                Some("API Error: Rate limit reached")
            ),
            Some(AgentHook::Failed("API Error: Rate limit reached".into()))
        );
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        hook(
            &mut machine,
            AgentHook::Failed("API Error: Rate limit reached".into()),
            c.at(5.0),
        );
        assert_eq!(
            machine.state(),
            AgentState::Done,
            "flagged like a finished turn"
        );
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn opencode_plugin_events_drive_the_state() {
        let event = |name: &str, tool: Option<&str>, message: Option<&str>| {
            hook_events::hook(Some("opencode"), name, tool, message)
        };
        assert_eq!(event("start", None, None), Some(AgentHook::SessionStarted));
        assert_eq!(
            event("session.start", None, None),
            None,
            "the summary request changes nothing"
        );
        assert_eq!(
            event("session.execution.started", None, None),
            Some(AgentHook::PromptSubmitted)
        );
        assert_eq!(event("tool.execute.before", Some("shell"), None), None);
        assert_eq!(
            event("tool.execute.after", Some("shell"), None),
            Some(AgentHook::UsedTool(Some("shell".into())))
        );
        assert_eq!(
            event("session.execution.succeeded", None, None),
            Some(AgentHook::Stopped)
        );
        assert_eq!(
            event("session.execution.interrupted", None, None),
            Some(AgentHook::Stopped)
        );
        assert_eq!(
            event("session.execution.failed", None, Some("Rate limited")),
            Some(AgentHook::Failed("Rate limited".into()))
        );
        // opencode's names mean nothing from another agent; Claude Code's
        // events mean the same from any.
        assert_eq!(
            hook_events::hook(Some("codex"), "session.execution.succeeded", None, None),
            None
        );
        assert_eq!(
            hook_events::hook(Some("gemini"), "Stop", None, None),
            Some(AgentHook::Stopped)
        );

        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        let turn = [
            ("start", None),
            ("session.execution.started", None),
            ("permission.asked", Some("shell")),
            ("permission.replied", None),
            ("tool.execute.after", Some("shell")),
            ("form.created", None),
        ];
        let mut states = Vec::new();
        for (offset, (name, tool)) in turn.into_iter().enumerate() {
            hook(
                &mut machine,
                event(name, tool, None).unwrap(),
                c.at(offset as f64),
            );
            states.push(machine.state());
        }
        use AgentState::*;
        assert_eq!(
            states,
            [Idle, Working, NeedsInput, Working, Working, NeedsInput]
        );
        hook(
            &mut machine,
            event("form.cancelled", None, None).unwrap(),
            c.at(10.0),
        );
        assert_eq!(machine.state(), Working);
        hook(
            &mut machine,
            event("session.execution.succeeded", None, None).unwrap(),
            c.at(11.0),
        );
        assert_eq!(machine.state(), Done);
        // An edit's permission is answered by its reply, not by the tool
        // (opencode asks for "edit" whichever tool writes).
        hook(
            &mut machine,
            event("session.execution.started", None, None).unwrap(),
            c.at(12.0),
        );
        hook(
            &mut machine,
            event("permission.asked", Some("edit"), None).unwrap(),
            c.at(13.0),
        );
        hook(
            &mut machine,
            event("tool.execute.after", Some("write"), None).unwrap(),
            c.at(14.0),
        );
        assert_eq!(machine.state(), NeedsInput);
        hook(
            &mut machine,
            event("permission.replied", None, None).unwrap(),
            c.at(15.0),
        );
        assert_eq!(machine.state(), Working);
    }

    #[test]
    fn a_tool_running_answers_its_own_approval() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        hook(
            &mut machine,
            notification("Claude needs your permission to use Bash"),
            c.at(3.0),
        );
        assert_eq!(machine.state(), AgentState::NeedsInput);
        // A parallel subagent's edit doesn't answer it.
        assert!(!hook(
            &mut machine,
            AgentHook::UsedTool(Some("Edit".into())),
            c.at(4.0)
        ));
        assert_eq!(machine.state(), AgentState::NeedsInput);
        // Approved with a number key, no Return: the command runs, and says
        // so.
        machine.handle(AgentEvent::Keystroke, c.at(5.0));
        assert_eq!(machine.state(), AgentState::NeedsInput);
        hook(
            &mut machine,
            AgentHook::UsedTool(Some("Bash".into())),
            c.at(6.0),
        );
        assert_eq!(machine.state(), AgentState::Working);
        // A tool the hook doesn't name answers any approval.
        hook(
            &mut machine,
            notification("Claude needs your permission to use Write"),
            c.at(7.0),
        );
        hook(&mut machine, AgentHook::UsedTool(None), c.at(8.0));
        assert_eq!(machine.state(), AgentState::Working);

        assert_eq!(
            requested_tool("Claude needs your permission to use Bash").as_deref(),
            Some("Bash")
        );
        assert_eq!(
            requested_tool("Allow mcp__docs__read? It wants to use mcp__docs__read.").as_deref(),
            Some("mcp__docs__read")
        );
        assert_eq!(requested_tool("Claude needs your input"), None);
    }

    #[test]
    fn tools_after_the_turn_change_nothing() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        hook(&mut machine, AgentHook::Stopped, c.at(10.0));
        // A background task's tool finishing after Stop.
        assert!(!hook(
            &mut machine,
            AgentHook::UsedTool(Some("Bash".into())),
            c.at(12.0)
        ));
        assert_eq!(machine.state(), AgentState::Done, "still unseen");
        machine.handle(AgentEvent::Acknowledged, c.at(13.0));
        assert!(!hook(
            &mut machine,
            AgentHook::UsedTool(Some("Bash".into())),
            c.at(14.0)
        ));
        assert_eq!(machine.state(), AgentState::Idle);
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn a_dismissed_question_ends_after_a_while() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        hook(
            &mut machine,
            notification("Claude needs your permission to use Bash"),
            c.at(2.0),
        );
        // Esc on the prompt sends nothing; someone away is still asked for a
        // good while.
        assert!(!machine.tick(c.at(600.0)));
        assert_eq!(machine.state(), AgentState::NeedsInput);
        assert_eq!(machine.next_deadline(), Some(c.at(1802.0)));
        assert!(machine.tick(c.at(1900.0)));
        assert_eq!(machine.state(), AgentState::Idle);
    }

    #[test]
    fn impulse_status_reports_are_kept() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(
            &mut machine,
            AgentHook::Status(AgentState::Working),
            c.at(1.0),
        );
        assert_eq!(machine.state(), AgentState::Working);
        assert_eq!(
            machine.next_deadline(),
            None,
            "a silent wrapper keeps working"
        );
        assert!(!machine.tick(c.at(5000.0)));
        hook(
            &mut machine,
            AgentHook::Status(AgentState::NeedsInput),
            c.at(5001.0),
        );
        assert_eq!(machine.state(), AgentState::NeedsInput);
        assert_eq!(machine.next_deadline(), None);
        hook(
            &mut machine,
            AgentHook::Status(AgentState::Done),
            c.at(5002.0),
        );
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn idle_reminders_dont_finish_a_turn() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        hook(&mut machine, AgentHook::Stopped, c.at(10.0));
        machine.handle(AgentEvent::Acknowledged, c.at(11.0));
        assert_eq!(machine.state(), AgentState::Idle);
        // Claude Code's reminder a minute after the turn.
        assert!(!hook(
            &mut machine,
            notification("Claude is waiting for your input"),
            c.at(71.0)
        ));
        assert_eq!(machine.state(), AgentState::Idle);
        // Mid-turn, a notification that asks nothing still ends the turn.
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(80.0));
        hook(
            &mut machine,
            notification("Claude is waiting for your input"),
            c.at(90.0),
        );
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn a_long_silence_ends_an_interrupted_hook_turn() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        stream(&mut machine, &c, 1.0, 20.0);
        // Esc mid-turn: no Stop comes, and the spinner stops.
        assert!(!machine.tick(c.at(200.0)));
        assert!(machine.tick(c.at(330.0)));
        assert_eq!(
            machine.state(),
            AgentState::Idle,
            "interrupted, not finished: nothing to flag"
        );

        // Silence from before the turn started doesn't count.
        let mut quiet = AgentStateMachine::new(c.at(0.0));
        stream(&mut quiet, &c, 0.0, 1.0);
        hook(&mut quiet, AgentHook::PromptSubmitted, c.at(600.0));
        assert!(!quiet.tick(c.at(601.0)));
        assert_eq!(quiet.next_deadline(), Some(c.at(900.0)));
    }

    #[test]
    fn a_stopped_spinner_ends_an_interrupted_hook_turn() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        machine.handle(title("◐ Fix the parser"), c.at(1.1));
        machine.handle(title("◑ Fix the parser"), c.at(2.0));
        stream(&mut machine, &c, 2.0, 10.0);
        assert!(
            machine.next_deadline().unwrap() > c.at(300.0),
            "spinning: only a long silence ends it"
        );
        // Esc: the spinner stops and no Stop comes, while the prompt
        // redraws and the user types the next one.
        machine.handle(title("✳ Fix the parser"), c.at(10.0));
        let mut t = 10.5;
        while t < 20.0 {
            machine.handle(AgentEvent::Keystroke, c.at(t));
            machine.handle(AgentEvent::Output, c.at(t + 0.05));
            t += 0.5;
        }
        assert_eq!(machine.next_deadline(), Some(c.at(18.0)));
        assert!(!machine.tick(c.at(17.0)));
        assert!(machine.tick(c.at(18.0)));
        assert_eq!(
            machine.state(),
            AgentState::Idle,
            "interrupted, not finished: nothing to flag"
        );
    }

    #[test]
    fn a_stopped_spinner_waits_for_the_hooks() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        machine.handle(title("◐ Fix the parser"), c.at(1.1));
        // A permission prompt stops the spinner; Claude Code reports it 6 s
        // later.
        machine.handle(title("✳ Fix the parser"), c.at(10.0));
        assert!(!machine.tick(c.at(16.0)));
        hook(
            &mut machine,
            notification("Claude needs your permission to use WebFetch"),
            c.at(16.2),
        );
        assert_eq!(machine.state(), AgentState::NeedsInput);
        assert_eq!(
            machine.next_deadline(),
            Some(c.at(16.2) + Duration::from_secs(1800))
        );
        // Approved: the spinner starts again (WebFetch reports no
        // PostToolUse).
        machine.handle(title("◑ Fix the parser"), c.at(20.0));
        assert_eq!(machine.state(), AgentState::Working);
        // A turn that ends reports Stop around when the spinner stops.
        machine.handle(title("✳ Fix the parser"), c.at(30.0));
        hook(&mut machine, AgentHook::Stopped, c.at(30.2));
        assert_eq!(machine.state(), AgentState::Done);
        assert_eq!(machine.next_deadline(), None);
        // Background tasks spin it after Stop: still done.
        assert!(!machine.handle(title("◐ Fix the parser"), c.at(31.0)));
        assert_eq!(machine.state(), AgentState::Done);
    }

    #[test]
    fn a_spinner_after_a_guessed_end_resumes_the_turn() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        hook(&mut machine, AgentHook::PromptSubmitted, c.at(1.0));
        machine.handle(title("◐ Ask about the schema"), c.at(1.1));
        // A question that reports nothing.
        machine.handle(title("✳ Ask about the schema"), c.at(5.0));
        machine.tick(c.at(13.0));
        assert_eq!(machine.state(), AgentState::Idle);
        // Answered: it goes on.
        machine.handle(title("◑ Ask about the schema"), c.at(40.0));
        assert_eq!(machine.state(), AgentState::Working);
        hook(&mut machine, AgentHook::Stopped, c.at(60.0));
        machine.handle(AgentEvent::Acknowledged, c.at(61.0));
        // An idle the hooks reported isn't resumed by a spinner.
        assert!(!machine.handle(title("◐ Ask about the schema"), c.at(62.0)));
        assert_eq!(machine.state(), AgentState::Idle);

        assert!(is_spinner(Some('◒')));
        assert!(!is_spinner(Some('✳')));
    }

    #[test]
    fn exit_is_final() {
        let c = clock();
        let mut machine = AgentStateMachine::new(c.at(0.0));
        machine.handle(AgentEvent::Submit, c.at(1.0));
        assert!(machine.handle(AgentEvent::Exited, c.at(2.0)));
        assert!(!machine.handle(AgentEvent::Submit, c.at(3.0)));
        assert_eq!(machine.state(), AgentState::Exited);
        assert!(AgentState::Done.wants_user() && AgentState::NeedsInput.wants_user());
        assert!(!AgentState::Working.wants_user());
    }
}
