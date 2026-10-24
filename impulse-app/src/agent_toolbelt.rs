//! Under an agent's terminal while it runs (the Swift `AgentToolbelt` and
//! `TerminalInputHost`): what the agent is doing, and the things done
//! around it: write to it (the composer), review its last turn, step back
//! through its turns, or put the files back to before one. The composer
//! takes the toolbelt's place while it's open.

use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::Sizable as _;
use gpui_kit::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, Window};
use impulse_kit::agents::state_machine::AgentState;
use impulse_kit::chrome::Rgba as KitRgba;

use crate::agent_tracker::AgentSignal;
use crate::terminal_view::TerminalView;
use crate::workbench_chrome::{hsla, icon};

/// "Working · 12s", "Finished 3m ago": the toolbelt's status.
pub fn status_text(state: AgentState, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let elapsed = if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h {}m", seconds / 3600, seconds / 60 % 60)
    };
    match state {
        AgentState::Working => format!("Working · {elapsed}"),
        AgentState::NeedsInput => "Needs your input".into(),
        AgentState::Done => format!("Finished {elapsed} ago"),
        AgentState::Idle => "Idle".into(),
        AgentState::Exited => "Exited".into(),
    }
}

/// When a turn started: the time today, else the day too (turns restored
/// from an earlier session can be from another day).
pub fn started_label(started: DateTime<Local>, now: DateTime<Local>) -> String {
    let time = started.format("%-I:%M %p").to_string();
    let day = started.date_naive();
    if day == now.date_naive() {
        time
    } else if now.date_naive().pred_opt() == Some(day) {
        format!("Yesterday at {time}")
    } else {
        format!("{} at {time}", started.format("%b %-d, %Y"))
    }
}

/// "Turn 3 · 2:14 PM (running)".
pub fn turn_label(index: usize, started: SystemTime, running: bool) -> String {
    let started = started_label(DateTime::<Local>::from(started), Local::now());
    format!(
        "Turn {} · {started}{}",
        index + 1,
        if running { " (running)" } else { "" }
    )
}

impl TerminalView {
    /// What sits between the grid and the input bar for the terminal in
    /// front: the composer while it's open over a program, else the
    /// toolbelt while an agent runs.
    pub(crate) fn agent_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(composer) = self.composer_element(window, cx) {
            return Some(composer);
        }
        if std::mem::take(&mut self.agent.composer_focus_pending) {
            // The composer closed: the keyboard goes back to the program.
            cx.defer_in(window, |view, window, cx| view.focus(window, cx));
        }
        let state = self.agent.state().filter(|s| *s != AgentState::Exited);
        let Some(state) = state.filter(|_| self.has_live_bar()) else {
            self.agent.toolbelt_tick = None;
            return None;
        };
        Some(self.toolbelt(state, cx))
    }

    fn toolbelt(&mut self, state: AgentState, cx: &mut Context<Self>) -> AnyElement {
        // The elapsed time moves on by itself.
        if self.agent.toolbelt_tick.is_none() {
            self.agent.toolbelt_tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, |view, cx| {
                    view.agent.toolbelt_tick = None;
                    cx.notify();
                });
            }));
        }
        let p = self.palette.clone();
        let c = KitRgba::from_hex;
        let name = self
            .agent
            .agent
            .as_ref()
            .map(|a| a.display_name.clone())
            .unwrap_or_default();
        let elapsed = Instant::now().saturating_duration_since(self.agent.machine.since());
        let id = self.agent.id.clone();
        let turns = crate::agent_checkpoints::turns(&id);
        let has_turns = !turns.is_empty();
        let busy = matches!(state, AgentState::Working | AgentState::NeedsInput);
        let compose_hint = impulse_kit::keybindings::display_for(
            "agent_composer",
            &crate::settings_store::SettingsStore::get(cx).keybinding_overrides,
            impulse_kit::keybindings::Platform::current(),
        );
        let pill = |id: &'static str,
                    title: &'static str,
                    glyph: &'static str,
                    hint: Option<String>,
                    enabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .px(px(6.))
                .h(px(22.))
                .rounded(px(5.))
                .bg(hsla(p.raised))
                .when(!enabled, |d| d.opacity(0.5))
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|d| d.bg(hsla(p.hover)))
                })
                .child(icon(glyph, 11., p.text_secondary))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.text_secondary))
                        .child(title),
                )
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(10.))
                        .text_color(hsla(p.text_tertiary))
                        .child(hint)
                }))
        };
        let compose = pill(
            "toolbelt-compose",
            "Compose",
            "message-square-plus",
            compose_hint,
            true,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(AgentSignal::Compose);
            }),
        );
        let review = pill(
            "toolbelt-review",
            "Review Turn",
            "file-diff",
            None,
            has_turns,
        )
        .when(has_turns, |d| {
            d.on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(AgentSignal::ReviewTurn(None));
                }),
            )
        });
        let view = cx.entity().downgrade();
        let count = turns.len();
        let listed: Vec<(usize, String)> = turns
            .iter()
            .enumerate()
            .rev()
            .take(12)
            .map(|(index, turn)| {
                // Only the newest turn of an agent that's mid-turn is
                // running; one restored from an earlier session isn't.
                let running = busy && index + 1 == count && turn.end.is_none();
                (index, turn_label(index, turn.start.date, running))
            })
            .collect();
        let turns_menu = Button::new("toolbelt-turns")
            .ghost()
            .compact()
            .small()
            .icon(gpui_kit::component::Icon::empty().path("icons/history.svg"))
            .label("Turns")
            .tooltip("Turns this agent took: review one, or restore the files to before it")
            .dropdown_menu(move |mut menu, _, _| {
                if listed.is_empty() {
                    return menu.item(PopupMenuItem::new("No turns yet").disabled(true));
                }
                for (n, (index, label)) in listed.iter().enumerate() {
                    if n > 0 {
                        menu = menu.separator();
                    }
                    let (index, review, restore) = (*index, view.clone(), view.clone());
                    menu = menu
                        .item(PopupMenuItem::new(format!("Review {label}")).on_click(
                            move |_, _, cx| {
                                let _ = review.update(cx, |_, cx| {
                                    cx.emit(AgentSignal::ReviewTurn(Some(index)))
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new(format!("Restore Files to Before {label}…"))
                                .on_click(move |_, _, cx| {
                                    let _ = restore.update(cx, |_, cx| {
                                        cx.emit(AgentSignal::RestoreTurn(index))
                                    });
                                }),
                        );
                }
                menu
            });
        div()
            .id("agent-toolbelt")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(32.))
            .bg(hsla(c(&self.theme.bg_dark)))
            .border_t_1()
            .border_color(hsla(c(&self.theme.border)))
            .font_family(crate::workbench::UI_FONT)
            .child(crate::agent_views::agent_glyph(state, 12., &p))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_secondary))
                    .child(status_text(state, elapsed)),
            )
            .children(self.agent.status_message.clone().map(|message| {
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(message)
            }))
            .child(div().flex_1().min_w(px(8.)))
            .child(compose)
            .child(review)
            .child(turns_menu)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    #[test]
    fn statuses_read_like_the_swift_app() {
        let s = Duration::from_secs;
        assert_eq!(status_text(AgentState::Working, s(12)), "Working · 12s");
        assert_eq!(status_text(AgentState::Working, s(185)), "Working · 3m");
        assert_eq!(status_text(AgentState::Done, s(3720)), "Finished 1h 2m ago");
        assert_eq!(
            status_text(AgentState::NeedsInput, s(5)),
            "Needs your input"
        );
        assert_eq!(status_text(AgentState::Idle, s(5)), "Idle");
    }

    #[test]
    fn turns_say_the_day_when_it_isnt_today() {
        let now = Local.with_ymd_and_hms(2026, 10, 24, 15, 0, 0).unwrap();
        let today = Local.with_ymd_and_hms(2026, 10, 24, 9, 5, 0).unwrap();
        let yesterday = Local.with_ymd_and_hms(2026, 10, 23, 21, 30, 0).unwrap();
        let earlier = Local.with_ymd_and_hms(2026, 10, 3, 14, 14, 0).unwrap();
        assert_eq!(started_label(today, now), "9:05 AM");
        assert_eq!(started_label(yesterday, now), "Yesterday at 9:30 PM");
        assert_eq!(started_label(earlier, now), "Oct 3, 2026 at 2:14 PM");
    }
}
