use anyhow::Result;
use gpui::{App, Context, MouseMoveEvent, Task, Window};
use ropey::Rope;

use crate::input::{EditorMode, HoverPopoverState, InputBaseState, RopeExt};

/// Hover provider
///
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover
pub trait HoverProvider {
    /// textDocument/hover
    ///
    /// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover
    fn hover(
        &self,
        _text: &Rope,
        _offset: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>>;
}

impl InputBaseState<EditorMode> {
    /// Handle hover trigger LSP request.
    pub(super) fn handle_hover_popover(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<InputBaseState<EditorMode>>,
    ) {
        if self.selecting {
            return;
        }

        let Some(provider) = self.extras.lsp.hover_provider.clone() else {
            return;
        };

        if let Some(hover_popover) = self.extras.hover_popover.as_ref() {
            if hover_popover.symbol_range.contains(&offset) {
                return;
            }
        }

        let text = self.text.clone();
        let mut symbol_range = self.text.word_range(offset).unwrap_or(offset..offset);
        let editor = cx.entity();
        let should_delay = self.extras.hover_popover.is_none();
        // Impulse: the host's delay (`set_hover_delay`).
        let delay = self.extras.lsp.hover_delay;
        // Ask the provider only after the delay: every mouse move replaces this
        // task, so positions the pointer merely passes over never reach it.
        self.extras.lsp._hover_task = cx.spawn_in(window, async move |this, cx| {
            if should_delay {
                cx.background_executor().timer(delay).await;
            }

            let task = this.update_in(cx, |_, window, cx| {
                provider.hover(&text, offset, window, cx)
            })?;
            let result = task.await?;

            _ = editor.update_in(cx, |editor, window, cx| {
                // Impulse: drop an answer that comes after the pointer has
                // left the text. The editor hears mouse moves only over
                // itself, so nothing would take its highlight away.
                let pointer = window.mouse_position();
                if !editor
                    .text_bounds()
                    .is_some_and(|bounds| bounds.contains(&pointer))
                {
                    return;
                }
                match result {
                    Some(hover) => {
                        // Impulse: only a range on one line. A YAML server's
                        // hover on a key covers the key's whole block (the
                        // whole file for a top-level key), which highlighted
                        // all of it and kept the hover open while the pointer
                        // was anywhere over it; Monaco's hover stays by the
                        // pointer, so keep the word under it instead.
                        if let Some(range) = hover.range.filter(|r| r.start.line == r.end.line) {
                            let start = editor.text.position_to_offset(&range.start);
                            let end = editor.text.position_to_offset(&range.end);
                            symbol_range = start..end;
                        }
                        editor.extras.hover_popover = Some(HoverPopoverState {
                            symbol_range,
                            hover,
                        });
                    }
                    None => {
                        editor.extras.hover_popover = None;
                    }
                }
                cx.notify();
            });

            Ok(())
        });
    }

    pub(crate) fn handle_mouse_move(
        &mut self,
        offset: usize,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.alt {
            self.clear_hover_state(cx);
            return;
        }
        if event.modifiers.secondary() {
            self.handle_hover_definition(offset, window, cx);
        } else {
            // Repaint only when the definition underline goes away; the hover
            // and definition results notify when they arrive.
            if !self.extras.hover_definition.is_empty() {
                self.extras.hover_definition.clear();
                cx.notify();
            }
            self.handle_hover_popover(offset, window, cx);
        }
    }

    pub fn clear_hover_state(&mut self, cx: &mut Context<Self>) {
        let changed =
            !self.extras.hover_definition.is_empty() || self.extras.hover_popover.is_some();
        self.extras.hover_definition.clear();
        self.extras.hover_popover = None;
        self.extras.lsp._hover_task = Task::ready(Ok(()));
        if changed {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{rc::Rc, time::Duration};

    use gpui::{
        AppContext as _, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
        Styled as _, Task, TestAppContext, VisualTestContext, Window, div, point, px,
    };
    use ropey::Rope;

    use super::HoverProvider;
    use crate::input::{EditorState, RopeExt as _};

    /// Impulse: a YAML server's hover on a key covers the key's whole block.
    struct WholeText;

    impl HoverProvider for WholeText {
        fn hover(
            &self,
            text: &Rope,
            _offset: usize,
            _window: &mut Window,
            _cx: &mut gpui::App,
        ) -> Task<anyhow::Result<Option<lsp_types::Hover>>> {
            let end = text.offset_to_position(text.len());
            Task::ready(Ok(Some(lsp_types::Hover {
                contents: lsp_types::HoverContents::Scalar(lsp_types::MarkedString::String(
                    "docs".into(),
                )),
                range: Some(lsp_types::Range::new(lsp_types::Position::new(0, 0), end)),
            })))
        }
    }

    /// The editor under a 100 px band (as a tab strip sits above it).
    struct Root(Entity<EditorState>);

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().h(px(100.)).flex_none())
                .child(div().h(px(200.)).flex_none().child(self.0.clone()))
        }
    }

    fn editor(cx: &mut TestAppContext) -> (Entity<EditorState>, VisualTestContext, usize) {
        let text = (0..200)
            .map(|i| format!("  key{i}: value"))
            .collect::<Vec<_>>()
            .join("\n");
        let len = text.len();
        let mut state = None;
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.set_global(crate::theme::Theme::default());
                crate::input::init(cx);
                let editor = cx.new(|cx| {
                    let mut editor = EditorState::new(window, cx).default_value(text);
                    editor.extras.lsp.hover_provider = Some(Rc::new(WholeText));
                    editor.set_hover_delay(Duration::from_millis(50));
                    editor
                });
                state = Some(editor.clone());
                cx.new(|_| Root(editor))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (state.unwrap(), cx, len)
    }

    #[gpui::test]
    fn a_range_past_the_last_visible_line_has_visible_bounds(cx: &mut TestAppContext) {
        let (state, mut cx, len) = editor(cx);
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert!(state.range_to_bounds(&(0..len)).is_none());
            let first = state.range_to_bounds(&(0..0)).unwrap();
            let bounds = state.visible_range_bounds(&(0..len)).unwrap();
            assert_eq!(bounds.top(), first.top());
            assert!(bounds.bottom() > first.bottom());
            assert_eq!(bounds.bottom(), state.text_bounds().unwrap().bottom());
            assert!(state.visible_range_bounds(&(len..len)).is_none());
        });
    }

    #[gpui::test]
    fn a_range_over_many_lines_keeps_to_the_word_under_the_pointer(cx: &mut TestAppContext) {
        let (state, mut cx, _) = editor(cx);
        let inside = cx.update(|_, cx| state.read(cx).range_to_bounds(&(4..4)).unwrap().center());
        cx.simulate_mouse_move(inside, None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        let range = cx.update(|_, cx| state.read(cx).hover_popover().unwrap().symbol_range.clone());
        // "  key0: value": the word "key0", not the whole text.
        assert_eq!(range, 2..6);
    }

    #[gpui::test]
    fn a_hover_answered_after_the_pointer_left_is_dropped(cx: &mut TestAppContext) {
        let (state, mut cx, _) = editor(cx);
        let inside = cx.update(|_, cx| state.read(cx).range_to_bounds(&(4..4)).unwrap().center());
        let outside = point(px(20.), px(40.));

        // The pointer rests on a key: the answer comes and stays.
        cx.simulate_mouse_move(inside, None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(cx.update(|_, cx| state.read(cx).hover_popover().is_some()));

        // The pointer stops on a key and leaves for the band above before
        // the answer comes.
        cx.update(|_, cx| state.update(cx, |state, cx| state.clear_hover_state(cx)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_move(inside, None, Modifiers::none());
        cx.simulate_mouse_move(outside, None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(cx.update(|_, cx| state.read(cx).hover_popover().is_none()));
    }
}
