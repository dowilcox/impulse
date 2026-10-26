//! Agents in the window's chrome (the Swift `AgentViews` and the agent
//! parts of `TabManager` and `MainWindowController`): the status glyph on
//! titlebar tabs and sidebar rows, the attention dot, the agents inbox in
//! the titlebar, desktop notifications for the window's terminals, Review
//! Last Agent Turn and Next Agent Needing You.

use std::time::Instant;

use gpui_kit::component::popover::Popover;
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, Entity, EntityId, FontWeight, Window,
};
use impulse_kit::agents::state_machine::AgentState;
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};

use crate::agent_tracker::{urgency, AgentSignal};
use crate::settings_store::SettingsStore;
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workbench_chrome::{hsla, icon};
use crate::workspace::{Surface, Tab};

/// The commands `Workbench::run_command` hands to `run_agent_command`.
pub const AGENT_COMMANDS: [&str; 6] = [
    "agent_hooks",
    "agent_mcp_server",
    "review_agent_turn",
    "next_agent",
    "agent_composer",
    "send_selection_to_agent",
];

/// One agent, as the inbox lists it.
#[derive(Clone)]
pub struct AgentSummary {
    pub terminal: Entity<TerminalView>,
    pub agent_name: String,
    pub tab_title: String,
    pub workspace_name: String,
    pub state: AgentState,
    pub since: Instant,
    pub has_turns: bool,
    pub message: Option<String>,
}

pub fn state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle => "Idle",
        AgentState::Working => "Working",
        AgentState::NeedsInput => "Needs your input",
        AgentState::Done => "Finished",
        AgentState::Exited => "Exited",
    }
}

/// "5s ago", "3m ago", "2h ago" (the Swift abbreviated relative style).
fn ago(since: Instant) -> String {
    let seconds = since.elapsed().as_secs();
    match seconds {
        0..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

/// How far round the working ring is: one turn every 900 ms, in twelve
/// steps (the workbench redraws twelve times a second while an agent works,
/// `Workbench::start_spinner`, rather than every frame).
pub(crate) const SPINNER_STEP: std::time::Duration = std::time::Duration::from_millis(75);

pub(crate) fn spinner_turn() -> f32 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    ((elapsed % 900) * 12 / 900) as f32 / 12.
}

/// Swift's `ProgressRing(progress: nil, color: chrome.working)`.
pub(crate) fn working_ring(size: f32, line_width: f32, p: &ChromePalette) -> AnyElement {
    let turn = if crate::display_options::reduce_motion() {
        0.
    } else {
        spinner_turn()
    };
    swift_ring(None, p.working, size, line_width, turn)
}

/// Swift's `ProgressRing` at `size` with `line_width`.
pub(crate) fn swift_ring(
    progress: Option<f32>,
    color: KitRgba,
    size: f32,
    line_width: f32,
    turn: f32,
) -> AnyElement {
    crate::workbench_chrome::progress_ring(progress, color, size, line_width, turn)
        .into_any_element()
}

/// A coding agent's state at a glance: a spinner while it works, a bot
/// with a badge when it wants the user, a quiet bot otherwise. The badges
/// differ in shape as well as color: a solid dot asks for input, a check
/// mark says it finished.
pub fn agent_glyph(state: AgentState, size: f32, p: &ChromePalette) -> AnyElement {
    match state {
        // Swift's ProgressRing with no progress, turning every 0.9 s (still
        // with Reduce Motion on).
        AgentState::Working => div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(size))
            .flex_none()
            .child(working_ring(size - 1., 1.6, p))
            .into_any_element(),
        AgentState::NeedsInput | AgentState::Done => {
            let badge = if state == AgentState::NeedsInput {
                div()
                    .size(px(size * 0.45))
                    .rounded_full()
                    .bg(hsla(p.attention))
                    .into_any_element()
            } else {
                icon("check", size * 0.6, p.success).into_any_element()
            };
            div()
                .relative()
                .size(px(size))
                .flex_none()
                .child(icon("bot", size, p.text_secondary))
                .child(
                    div()
                        .absolute()
                        .top(px(-size * 0.12))
                        .right(px(-size * 0.18))
                        .child(badge),
                )
                .into_any_element()
        }
        AgentState::Idle | AgentState::Exited => {
            icon("bot", size, p.text_tertiary).into_any_element()
        }
    }
}

/// The attention dot (or the dirty dot) at the right of a tab.
pub fn status_dot(size: f32, color: KitRgba) -> AnyElement {
    div()
        .size(px(size))
        .rounded_full()
        .bg(hsla(color))
        .into_any_element()
}

impl Workbench {
    /// Desktop notifications and redraws for a new terminal's agent.
    pub(crate) fn watch_agent(
        &mut self,
        terminal: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            terminal,
            window,
            |workbench, terminal, signal, window, cx| {
                workbench.agent_signal(terminal, signal, window, cx)
            },
        )
        .detach();
    }

    fn agent_signal(
        &mut self,
        terminal: &Entity<TerminalView>,
        signal: &AgentSignal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match signal {
            AgentSignal::Changed => {
                // After this update: it reads every window's workbench.
                gpui_kit::App::defer(cx, crate::badge::update);
                cx.notify()
            }
            AgentSignal::SendToAgent(text) => {
                self.send_to_best_agent(text, Some(terminal.entity_id()), window, cx)
            }
            AgentSignal::ReviewTurn(index) => {
                self.review_agent_turn_at(terminal.clone(), *index, window, cx)
            }
            AgentSignal::RestoreTurn(index) => {
                self.restore_agent_turn(terminal.clone(), *index, window, cx)
            }
            AgentSignal::Compose => {
                self.reveal_surface(&Surface::Terminal(terminal.clone()), cx);
                self.toggle_agent_composer(window, cx)
            }
            AgentSignal::Notify { title, body } => {
                // In front, the tab's dot is enough.
                if crate::agent_notify::app_in_front(cx) {
                    return;
                }
                window.request_attention();
                let id = terminal.entity_id();
                let workspace = self
                    .workspaces
                    .iter()
                    .find(|w| {
                        w.all_surfaces()
                            .filter_map(Surface::terminal)
                            .any(|t| t.entity_id() == id)
                    })
                    .map(|w| w.name());
                let subtitle = workspace
                    .into_iter()
                    .chain([terminal.read(cx).display_title()])
                    .collect::<Vec<_>>()
                    .join(" · ");
                // Clicking it (where the desktop says) shows the terminal.
                let Some(clicked) =
                    crate::agent_notify::post(title, &subtitle, body, Some(id.as_u64()))
                else {
                    return;
                };
                let terminal = terminal.clone();
                cx.spawn_in(window, async move |workbench, cx| {
                    if clicked.recv().await.is_ok() {
                        let _ = workbench.update_in(cx, |workbench, window, cx| {
                            window.activate_window();
                            workbench.reveal_terminal(&terminal, window, cx);
                        });
                    }
                })
                .detach();
            }
        }
    }

    /// The tab's most pressing agent.
    pub(crate) fn tab_agent(tab: &Tab, cx: &App) -> Option<(String, AgentState)> {
        tab.panes
            .values()
            .filter_map(Surface::terminal)
            .filter_map(|t| {
                let tracker = &t.read(cx).agent;
                Some((
                    tracker.agent.as_ref()?.display_name.clone(),
                    tracker.state()?,
                ))
            })
            .max_by_key(|(_, state)| urgency(*state))
    }

    /// A terminal in the tab wants the user.
    pub(crate) fn tab_needs_attention(tab: &Tab, cx: &App) -> bool {
        tab.panes
            .values()
            .filter_map(Surface::terminal)
            .any(|t| t.read(cx).agent.needs_attention)
    }

    /// A tab's icon: its agent's glyph, a terminal's in `color`, a file's
    /// own, else `name` in the text color (the Swift tab's symbol image).
    pub(crate) fn tab_icon(
        &self,
        tab: &Tab,
        name: &str,
        size: f32,
        color: KitRgba,
        cx: &App,
    ) -> AnyElement {
        if let Some((_, state)) = Self::tab_agent(tab, cx) {
            return agent_glyph(state, size, &self.palette);
        }
        let surface = tab.focused_surface();
        if let Some(terminal) = surface.terminal() {
            let t = terminal.read(cx);
            // A program's progress (red for an error, the warning color
            // when paused), else its status dot.
            if let Some((state, percent)) = t.progress {
                use impulse_terminal::osc_scanner::ProgressState;
                let color = match state {
                    ProgressState::Error => self.palette.danger,
                    ProgressState::Paused => self.palette.warning,
                    _ => self.palette.working,
                };
                let fraction = (state != ProgressState::Indeterminate)
                    .then(|| percent.map(|p| p as f32 / 100.))
                    .flatten();
                return div()
                    .size(px(size))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .child(swift_ring(fraction, color, size - 1., 1.6, 0.))
                    .into_any_element();
            }
            if let Some(dot) = t.session_status.dot_color() {
                return div()
                    .size(px(size))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px((size * 0.6).round()))
                            .rounded_full()
                            .bg(crate::workbench_chrome::hsla(KitRgba::from_hex(dot))),
                    )
                    .into_any_element();
            }
        }
        // A terminal a program drives directly gets the filled glyph.
        if let Some(terminal) = surface.terminal() {
            let name = if terminal.read(cx).direct {
                "square-terminal"
            } else {
                "terminal"
            };
            return icon(name, size, color).into_any_element();
        }
        match surface.file_icon(cx) {
            Some(path) => gpui_kit::img(path)
                .size(px(size + 1.))
                .flex_none()
                .into_any_element(),
            None => icon(name, size + 1., self.palette.text).into_any_element(),
        }
    }

    /// Every agent in the window, most urgent first.
    pub(crate) fn agent_summaries(&self, cx: &App) -> Vec<AgentSummary> {
        let mut agents: Vec<AgentSummary> = self
            .workspaces
            .iter()
            .flat_map(|workspace| {
                workspace
                    .all_surfaces()
                    .filter_map(Surface::terminal)
                    .filter_map(|terminal| {
                        let view = terminal.read(cx);
                        let tracker = &view.agent;
                        Some(AgentSummary {
                            terminal: terminal.clone(),
                            agent_name: tracker.agent.as_ref()?.display_name.clone(),
                            tab_title: view.display_title(),
                            workspace_name: workspace.name(),
                            state: tracker.state()?,
                            since: tracker.machine.since(),
                            has_turns: crate::agent_checkpoints::has_turns(&tracker.id),
                            message: tracker.status_message.clone(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        agents.sort_by(|a, b| {
            urgency(b.state)
                .cmp(&urgency(a.state))
                .then(b.since.cmp(&a.since))
        });
        agents
    }

    /// The working spinner and waiting count on a workspace's row.
    pub(crate) fn workspace_agent_counts(&self, index: usize, cx: &App) -> Option<AnyElement> {
        let workspace = self.workspaces.get(index)?;
        let states: Vec<AgentState> = workspace
            .tabs
            .iter()
            .filter_map(|tab| Self::tab_agent(tab, cx).map(|(_, s)| s))
            .collect();
        let working = states.iter().filter(|s| **s == AgentState::Working).count();
        let waiting = states.iter().filter(|s| s.wants_user()).count();
        if working == 0 && waiting == 0 {
            return None;
        }
        let p = &self.palette;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .when(working > 0, |d| d.child(working_ring(10., 1.4, p)))
                .when(waiting > 0, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(icon("bot", 10., p.attention))
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.))
                                    .text_color(hsla(p.attention))
                                    .child(waiting.to_string()),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    // MARK: Commands

    pub(crate) fn run_agent_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(part) = crate::agent_integration::part_for(id) {
            return self.present_agent_integration(part, window, cx);
        }
        match id {
            "review_agent_turn" => {
                let terminal = self.active_terminal().cloned();
                self.review_last_agent_turn(terminal, window, cx)
            }
            "next_agent" => self.reveal_next_waiting_agent(window, cx),
            "agent_composer" => self.toggle_agent_composer(window, cx),
            "send_selection_to_agent" => self.send_editor_selection_to_agent(window, cx),
            _ => {}
        }
    }

    /// The most recent turn of `terminal`'s agent, else the latest in the
    /// focused surface's repository.
    pub(crate) fn last_agent_turn(
        &self,
        terminal: Option<&Entity<TerminalView>>,
        cx: &App,
    ) -> Option<crate::agent_checkpoints::Turn> {
        terminal
            .and_then(|t| crate::agent_checkpoints::last_turn(&t.read(cx).agent.id))
            .or_else(|| {
                let dir = self
                    .focused_surface()
                    .and_then(|s| s.directory(cx))
                    .unwrap_or_else(|| self.workspace().default_directory());
                let root = impulse_git::branches::repo_root(&dir.to_string_lossy())?;
                crate::agent_checkpoints::last_turn_in_repo(&root)
            })
    }

    /// Open Review on the agent's most recent turn.
    pub(crate) fn review_last_agent_turn(
        &mut self,
        terminal: Option<Entity<TerminalView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(turn) = self.last_agent_turn(terminal.as_ref(), cx) else {
            return self.set_notice("No agent turns recorded yet in this repository.".into(), cx);
        };
        // Review opens for the focused surface's repository: the agent's.
        if let Some(terminal) = terminal {
            self.reveal_surface(&Surface::Terminal(terminal), cx);
        }
        self.open_review(Some(turn.scope()), window, cx);
    }

    /// Review one of the terminal's turns, by its index (`None`: the last).
    pub(crate) fn review_agent_turn_at(
        &mut self,
        terminal: Entity<TerminalView>,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = index else {
            return self.review_last_agent_turn(Some(terminal), window, cx);
        };
        let turns = crate::agent_checkpoints::turns(&terminal.read(cx).agent.id);
        let Some(turn) = turns.get(index) else {
            return;
        };
        self.reveal_surface(&Surface::Terminal(terminal), cx);
        self.open_review(Some(turn.scope()), window, cx);
    }

    /// Put the repository's files back to how they were when one of the
    /// terminal's turns started, after asking (a snapshot first, so Undo
    /// brings the newer state back).
    pub(crate) fn restore_agent_turn(
        &mut self,
        terminal: Entity<TerminalView>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let turns = crate::agent_checkpoints::turns(&terminal.read(cx).agent.id);
        let Some(turn) = turns.get(index).cloned() else {
            return;
        };
        let workbench = cx.entity().downgrade();
        crate::tasks::confirm(
            window,
            cx,
            format!("Restore files to before turn {}?", index + 1),
            format!(
                "Every file in the repository goes back to how it was when {} started that turn. \
                 Commits are kept, and you can undo this right after.",
                turn.agent_name
            ),
            "Restore",
            "Cancel",
            true,
            move |yes, window, cx| {
                if yes {
                    let _ =
                        workbench.update(cx, |w, cx| w.restore_turn_files(turn, index, window, cx));
                }
            },
        );
    }

    fn restore_turn_files(
        &mut self,
        turn: crate::agent_checkpoints::Turn,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = crate::repository_state::RepositoryStore::state(&turn.repo_root, cx);
        let start = turn.start.clone();
        let restoring = state.update(cx, |state, cx| {
            state.run(
                Some("Restoring…".into()),
                Some("restore agent checkpoint".into()),
                true,
                move |root, _| impulse_git::safety::restore(&start, &[] as &[&str], root),
                cx,
            )
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let (result, snapshot) = restoring.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                if let Err(error) = result {
                    return workbench.present_git_error(&error, "Couldn't restore", window, cx);
                }
                let message = format!("Restored files to before turn {}", index + 1);
                let Some(snapshot) = snapshot else {
                    return crate::tasks::toast(
                        window,
                        cx,
                        crate::tasks::ToastKind::Success,
                        message,
                        None,
                        None,
                    );
                };
                let (undo_state, undo_workbench) = (state.clone(), cx.entity().downgrade());
                let undo: crate::tasks::Run = std::rc::Rc::new(move |window, cx| {
                    let snapshot = snapshot.clone();
                    let undoing = undo_state.update(cx, |state, cx| {
                        state.run(
                            None,
                            None,
                            false,
                            move |root, _| {
                                impulse_git::safety::restore(&snapshot, &[] as &[&str], root)
                            },
                            cx,
                        )
                    });
                    let workbench = undo_workbench.clone();
                    window
                        .spawn(cx, async move |cx| {
                            let (result, _) = undoing.await;
                            if let Err(error) = result {
                                let _ = cx.update(|window, cx| {
                                    let _ = workbench.update(cx, |w, cx| {
                                        w.present_git_error(
                                            &error,
                                            "Couldn't undo the restore",
                                            window,
                                            cx,
                                        )
                                    });
                                });
                            }
                        })
                        .detach();
                });
                crate::tasks::toast(
                    window,
                    cx,
                    crate::tasks::ToastKind::Success,
                    message,
                    None,
                    Some(("Undo", undo)),
                );
            });
        })
        .detach();
    }

    /// The composer over the program in the focused terminal (an agent's
    /// TUI); closes again when it's open. At a prompt the input bar is the
    /// place to type, so it gets the keyboard instead.
    pub(crate) fn toggle_agent_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.active_terminal().cloned() else {
            return crate::tasks::toast(
                window,
                cx,
                crate::tasks::ToastKind::Info,
                "The composer writes to the program in a terminal.".into(),
                None,
                None,
            );
        };
        let (direct, open, cwd) = {
            let t = terminal.read(cx);
            (t.direct, t.agent.composer.is_some(), t.cwd.clone())
        };
        if !direct {
            return terminal.update(cx, |t, cx| t.focus(window, cx));
        }
        if open {
            return terminal.update(cx, |t, cx| t.close_composer(cx));
        }
        let workspace = self.workspace();
        let root = if workspace.kind == crate::workspace::Kind::Folder {
            workspace.root.to_string_lossy().into_owned()
        } else {
            cwd
        };
        let trusted = self.trust.is_trusted(&root);
        terminal.update(cx, |t, cx| t.open_composer_at(root, trusted, cx));
    }

    /// Show the terminal and give it the keyboard.
    pub(crate) fn reveal_terminal(
        &mut self,
        terminal: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal_surface(&Surface::Terminal(terminal.clone()), cx);
        self.focus_active(window, cx);
    }

    /// Go to the next agent that needs the user.
    fn reveal_next_waiting_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let waiting: Vec<AgentSummary> = self
            .agent_summaries(cx)
            .into_iter()
            .filter(|a| a.state.wants_user())
            .collect();
        if waiting.is_empty() {
            return self.set_notice("No agents are waiting for you.".into(), cx);
        }
        let current: Option<EntityId> = self.active_terminal().map(|t| t.entity_id());
        let start = waiting
            .iter()
            .position(|a| Some(a.terminal.entity_id()) == current)
            .map_or(0, |i| i + 1);
        let next = waiting[start % waiting.len()].terminal.clone();
        self.reveal_terminal(&next, window, cx);
    }

    // MARK: The inbox

    /// The titlebar's way into the inbox: how many agents want the user
    /// and how many are working. Nothing while there are no agents.
    pub(crate) fn agent_inbox_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let agents = self.agent_summaries(cx);
        if agents.is_empty() {
            return None;
        }
        let p = self.palette.clone();
        let waiting = agents.iter().filter(|a| a.state.wants_user()).count();
        let working = agents
            .iter()
            .filter(|a| a.state == AgentState::Working)
            .count();
        let trigger = InboxTrigger(
            div()
                .id("agent-inbox")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .h(px(24.))
                .active(|s| s.opacity(0.7))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Agents (next one that needs you: Next Agent Needing You)",
                    )
                    .build(window, cx)
                })
                .child(if waiting > 0 {
                    icon("bell-dot", 13., p.attention)
                } else {
                    icon("bot", 13., p.text_secondary)
                })
                .when(working > 0, |d| {
                    d.child(working_ring(10., 1.4, &p)).child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(10.5))
                            .text_color(hsla(p.text_secondary))
                            .child(working.to_string()),
                    )
                })
                .when(waiting > 0, |d| {
                    d.child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.attention))
                            .child(format!("{waiting} waiting")),
                    )
                }),
        );
        let workbench = cx.entity().downgrade();
        let hint = impulse_kit::keybindings::display_for(
            "next_agent",
            &SettingsStore::get(cx).keybinding_overrides,
            impulse_kit::keybindings::Platform::current(),
        )
        .unwrap_or_default();
        Some(
            // The panel draws its own frame, as the palette's: GPUI Kit's
            // would put a padded box around it (Swift's NSPopover is one).
            // NSPopover centers under the button, touching its bottom edge.
            Popover::new("agent-inbox-popover")
                .anchor(gpui_kit::Anchor::TopCenter)
                .offset(px(0.))
                .appearance(false)
                .trigger(trigger)
                .content(move |_, _, cx| inbox(&agents, &p, &hint, workbench.clone(), cx))
                .into_any_element(),
        )
    }
}

/// The inbox button's face. Swift's `AgentInboxButton` has no background,
/// hovered, pressed or while its popover is open; it only dims while
/// pressed (`ChromePressStyle`). So the popover's open state, which
/// GPUI Kit shows as a ghost button's selected fill, changes nothing here.
#[derive(IntoElement)]
struct InboxTrigger(gpui_kit::Stateful<gpui_kit::Div>);

impl gpui_kit::base::Selectable for InboxTrigger {
    fn selected(self, _: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        false
    }
}

impl RenderOnce for InboxTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}

/// Every agent in the window, most urgent first; choosing one shows its
/// pane.
fn inbox(
    agents: &[AgentSummary],
    p: &ChromePalette,
    hint: &str,
    workbench: gpui_kit::WeakEntity<Workbench>,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    let mut list = div()
        .id("agent-inbox-list")
        .flex()
        .flex_col()
        .gap(px(1.))
        .py(px(4.))
        .max_h(px(320.))
        .overflow_y_scroll();
    for (index, agent) in agents.iter().enumerate() {
        let terminal = agent.terminal.clone();
        let (reveal_workbench, review_workbench) = (workbench.clone(), workbench.clone());
        let review_terminal = terminal.clone();
        let details = [
            agent.workspace_name.clone(),
            state_label(agent.state).to_string(),
            agent.message.clone().unwrap_or_default(),
            ago(agent.since),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        list = list.child(
            div()
                .id(("agent-row", index))
                .group("agent-row")
                .relative()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(40.))
                .px(px(12.))
                // Swift's rowBackground: inset and rounded, as the sidebar's.
                .child(crate::workbench_chrome::row_background(
                    false,
                    "agent-row",
                    p,
                ))
                .on_click(cx.listener(move |state, _, window, cx| {
                    state.dismiss(window, cx);
                    let terminal = terminal.clone();
                    let _ = reveal_workbench
                        .update(cx, |w, cx| w.reveal_terminal(&terminal, window, cx));
                }))
                .child(div().w(px(18.)).flex().justify_center().child(agent_glyph(
                    agent.state,
                    14.,
                    p,
                )))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(1.))
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis_middle()
                                .text_size(px(12.))
                                .font_weight(if agent.state.wants_user() {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(hsla(p.text))
                                .child(format!("{} · {}", agent.agent_name, agent.tab_title)),
                        )
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(10.5))
                                .text_color(hsla(if agent.state == AgentState::NeedsInput {
                                    p.attention
                                } else {
                                    p.text_tertiary
                                }))
                                .child(details),
                        ),
                )
                .when(agent.has_turns, |d| {
                    let done = agent.state == AgentState::Done;
                    d.child(
                        div()
                            .flex_none()
                            .when(!done, |d| {
                                d.invisible().group_hover("agent-row", |s| s.visible())
                            })
                            .child(
                                crate::workbench_chrome::chrome_button(
                                    ("agent-review", index),
                                    "Review",
                                    Some("file-diff"),
                                    crate::workbench_chrome::ButtonKind::Secondary,
                                    false,
                                    p,
                                )
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Review the last turn's changes",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(
                                    move |state, _, window, cx| {
                                        cx.stop_propagation();
                                        state.dismiss(window, cx);
                                        let terminal = review_terminal.clone();
                                        let _ = review_workbench.update(cx, |w, cx| {
                                            w.review_last_agent_turn(Some(terminal), window, cx)
                                        });
                                    },
                                )),
                            ),
                    )
                }),
        );
    }
    let hooks_workbench = workbench.clone();
    // NSPopover's frame: a 13pt margin of the same color round Swift's
    // 340-wide view, large rounded corners and a shadow, no border.
    div()
        .relative()
        .p(px(13.))
        .bg(hsla(p.overlay))
        .rounded(px(metrics::RADIUS_LARGE + 4.))
        .shadow_lg()
        .overflow_hidden()
        .text_color(hsla(p.text))
        .font_family(crate::workbench::UI_FONT)
        .child(crate::overlays::probe())
        .child(inbox_body(agents, list, p, hint, hooks_workbench, cx))
        .into_any_element()
}

/// The inbox's content, Swift's 340-wide `AgentInboxView`.
fn inbox_body(
    agents: &[AgentSummary],
    list: gpui_kit::Stateful<gpui_kit::Div>,
    p: &ChromePalette,
    hint: &str,
    hooks_workbench: gpui_kit::WeakEntity<Workbench>,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    let hairline = || div().h(px(1.)).bg(hsla(p.hairline));
    div()
        .flex()
        .flex_col()
        .w(px(340.))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(30.))
                .px(px(12.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Agents"),
                )
                .when(!hint.is_empty(), |d| {
                    d.child(
                        // A key chip, as Swift's KeyHint.
                        div()
                            .px(px(5.))
                            .py(px(1.))
                            .rounded(px(metrics::RADIUS_SMALL))
                            .border_1()
                            .border_color(hsla(p.hairline_strong))
                            .text_size(px(10.5))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(hsla(p.text_secondary))
                            .child(hint.to_string()),
                    )
                }),
        )
        .child(hairline())
        .child(if agents.is_empty() {
            div()
                .p(px(12.))
                .text_size(px(11.5))
                .text_color(hsla(p.text_tertiary))
                .child("No agents running. Start Claude Code, Codex, opencode, Gemini or another agent in any terminal.")
                .into_any_element()
        } else {
            list.into_any_element()
        })
        .child(hairline())
        .child(
            div()
                .id("agent-inbox-hooks")
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(28.))
                .px(px(12.))
                .text_size(px(11.))
                .text_color(hsla(p.text_tertiary))
                .hover(|s| s.text_color(hsla(p.text_secondary)))
                .on_click(cx.listener(move |state, _, window, cx| {
                    state.dismiss(window, cx);
                    let _ = hooks_workbench.update(cx, |w, cx| {
                        w.present_agent_integration(crate::agent_integration::Part::Hooks, window, cx)
                    });
                }))
                .child(icon("plug", 11., p.text_tertiary))
                .child("Agent hooks give exact status…"),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_swift_app() {
        assert_eq!(state_label(AgentState::NeedsInput), "Needs your input");
        assert_eq!(state_label(AgentState::Done), "Finished");
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(ago(Instant::now()), "0s ago");
    }
}

impl crate::workbench::Workbench {
    /// Redraw twelve times a second while an agent in this window works,
    /// for its turning ring; four times a second just to look otherwise.
    pub(crate) fn start_spinner(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        cx.spawn_in(window, async move |workbench, cx| loop {
            let working = workbench.update(cx, |workbench, cx| {
                let working = workbench
                    .workspaces
                    .iter()
                    .flat_map(|w| w.all_surfaces())
                    .any(|s| match s {
                        crate::workspace::Surface::Terminal(t) => {
                            t.read(cx).agent.machine.state() == AgentState::Working
                        }
                        _ => false,
                    });
                if working && !crate::display_options::reduce_motion() {
                    cx.notify();
                }
                working
            });
            let Ok(working) = working else {
                break;
            };
            let wait = if working {
                SPINNER_STEP
            } else {
                std::time::Duration::from_millis(250)
            };
            cx.background_executor().timer(wait).await;
        })
        .detach();
    }
}
