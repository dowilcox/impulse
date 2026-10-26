//! Inlay hints: the language server's parameter names and types drawn in
//! the text (Monaco's `inlayHints`, `editor_inlay_hints`), through
//! Impulse's GPUI Kit patch (`EditorState::set_inlay_hints`). Asked for the
//! lines on screen and 30 either side, as Monaco asks, again a moment after
//! an edit and when the view scrolls past them. The setting's
//! `offUnlessPressed` and `onUnlessPressed` show or hide them while Ctrl+Alt
//! (⌃⌥) is held.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::base::input::InlayHint;
use gpui_kit::{Context, Task};

use crate::editor_tab::EditorTab;

/// Lines asked for beyond those on screen, each way (Monaco's).
const MARGIN: usize = 30;

/// How long after an edit the hints are asked for again.
pub(crate) const EDIT_DELAY: Duration = Duration::from_millis(250);

/// How long after a scroll past the lines asked for.
const SCROLL_DELAY: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(crate) struct InlayState {
    /// `editor_inlay_hints`: on, off, offUnlessPressed or onUnlessPressed.
    mode: String,
    /// Ctrl+Alt held.
    pressed: bool,
    /// The lines the shown hints were asked for.
    rows: Option<Range<usize>>,
    /// A request waiting or under way.
    task: Option<Task<()>>,
}

/// Whether hints show in `mode` with Ctrl+Alt `pressed` or not.
pub(crate) fn shown(mode: &str, pressed: bool) -> bool {
    match mode {
        "off" => false,
        "offUnlessPressed" => pressed,
        "onUnlessPressed" => !pressed,
        _ => true,
    }
}

/// The lines to ask for when `visible` are on screen in a text of `lines`.
pub(crate) fn rows_to_ask(visible: &Range<usize>, lines: usize) -> Range<usize> {
    visible.start.saturating_sub(MARGIN)..(visible.end + MARGIN).min(lines.max(1))
}

impl EditorTab {
    /// Take the setting: hints asked for when they can show, dropped when
    /// they can't.
    pub(crate) fn set_inlay_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        if self.inlay.mode == mode {
            return;
        }
        self.inlay.mode = mode.to_string();
        self.show_inlay_hints(cx);
        if mode == "off" {
            self.inlay.task = None;
            self.inlay.rows = None;
            self.state
                .update(cx, |state, cx| state.set_inlay_hints(Vec::new(), cx));
        } else {
            self.refresh_inlay_hints(Duration::ZERO, cx);
        }
    }

    /// Ctrl+Alt pressed or let go.
    pub(crate) fn inlay_modifiers(&mut self, pressed: bool, cx: &mut Context<Self>) {
        if self.inlay.pressed != pressed {
            self.inlay.pressed = pressed;
            self.show_inlay_hints(cx);
        }
    }

    fn show_inlay_hints(&mut self, cx: &mut Context<Self>) {
        let visible = shown(&self.inlay.mode, self.inlay.pressed);
        self.state
            .update(cx, |state, cx| state.set_inlay_hints_visible(visible, cx));
    }

    /// The view moved: ask again once it shows lines past those asked for.
    pub(crate) fn inlay_view_moved(&mut self, cx: &mut Context<Self>) {
        if self.inlay.mode == "off" || self.inlay.task.is_some() {
            return;
        }
        let Some(asked) = self.inlay.rows.clone() else {
            return;
        };
        let Some(visible) = self.state.read(cx).visible_row_range() else {
            return;
        };
        if visible.start < asked.start || visible.end > asked.end {
            self.refresh_inlay_hints(SCROLL_DELAY, cx);
        }
    }

    /// Ask the server for the hints of the lines around those on screen
    /// after `delay` (an edit sooner than that asks later instead).
    pub(crate) fn refresh_inlay_hints(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.inlay.mode.is_empty() || self.inlay.mode == "off" {
            return;
        }
        if self.inlay_link().is_none() {
            return;
        }
        self.inlay.task = Some(cx.spawn(async move |tab, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(Some((request, rows, version, link))) = tab.update(cx, |tab, cx| {
                let link = tab.inlay_link()?;
                let state = tab.state.read(cx);
                let text = state.text().clone();
                let lines = gpui_kit::component::input::RopeExt::lines_len(&text);
                // Before the first layout, the top of the file.
                let visible = state.visible_row_range().unwrap_or(0..60);
                let rows = rows_to_ask(&visible, lines);
                let request = link.inlay_hints(&text, rows.clone(), cx);
                Some((request, rows, link.version(), link))
            }) else {
                let _ = tab.update(cx, |tab, _| tab.inlay.task = None);
                return;
            };
            let result = request.await;
            let _ = tab.update(cx, |tab, cx| {
                tab.inlay.task = None;
                // Edited since: the next request has the right offsets.
                if link.version() != version {
                    return;
                }
                match result {
                    Ok(hints) => {
                        let hints = hints
                            .into_iter()
                            .map(|hint| {
                                InlayHint::new(hint.offset, hint.label)
                                    .padding_left(hint.padding_left)
                                    .padding_right(hint.padding_right)
                            })
                            .collect();
                        tab.state
                            .update(cx, |state, cx| state.set_inlay_hints(hints, cx));
                        tab.inlay.rows = Some(rows);
                        // The view may have moved on while it was asked.
                        tab.inlay_view_moved(cx);
                    }
                    Err(error) => log::debug!("inlay hints: {error}"),
                }
            });
        }));
    }

    /// A server stopped: its hints go when no other draws them.
    pub(crate) fn inlay_server_gone(&mut self, cx: &mut Context<Self>) {
        if self.inlay_link().is_none() {
            self.inlay.task = None;
            self.inlay.rows = None;
            self.state
                .update(cx, |state, cx| state.set_inlay_hints(Vec::new(), cx));
        }
    }

    /// The server that draws hints: the primary one, else the first that
    /// does.
    fn inlay_link(&self) -> Option<std::rc::Rc<crate::lsp_hub::DocumentLink>> {
        let mut links = self.document_links();
        links.sort_by_key(|(primary, _)| !primary);
        links
            .into_iter()
            .map(|(_, link)| link)
            .find(|link| link.has_inlay_hints())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_and_ctrl_alt_decide_whether_hints_show() {
        assert!(shown("on", false) && shown("on", true));
        assert!(!shown("off", false) && !shown("off", true));
        assert!(!shown("offUnlessPressed", false) && shown("offUnlessPressed", true));
        assert!(shown("onUnlessPressed", false) && !shown("onUnlessPressed", true));
    }

    #[test]
    fn hints_are_asked_for_around_the_lines_on_screen() {
        assert_eq!(rows_to_ask(&(40..80), 500), 10..110);
        assert_eq!(rows_to_ask(&(0..20), 25), 0..25);
        assert_eq!(rows_to_ask(&(0..0), 0), 0..1);
    }
}
