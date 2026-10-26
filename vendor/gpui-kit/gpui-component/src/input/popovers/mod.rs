mod caret_anchored;
mod code_action_menu;
mod completion_menu;
mod diagnostic_popover;
mod hover_popover;

pub(crate) use caret_anchored::*;
pub(crate) use code_action_menu::*;
pub(crate) use completion_menu::*;
// Impulse: public, for `crate::input::CompletionMenuSettings` and its icons.
pub use completion_menu::{CompletionIconAssets, CompletionMenuSettings};
pub(crate) use diagnostic_popover::*;
pub(crate) use hover_popover::*;

use gpui::{
    App, Div, ElementId, Empty, Hsla, InteractiveElement as _, SharedString, Stateful,
    StyleRefinement, Styled as _, Window, div, rems,
};

use crate::{
    ActiveTheme, ThemeStyled as _,
    text::{TextView, TextViewStyle},
};

pub(super) fn render_markdown(
    id: impl Into<ElementId>,
    markdown: impl Into<SharedString>,
    _: &mut Window,
    cx: &mut App,
) -> TextView {
    TextView::markdown(id, markdown)
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.5))
                .heading_font_size(|level, rem_size| match level {
                    1..=3 => rem_size * 1,
                    4 => rem_size * 0.9,
                    _ => rem_size * 0.8,
                })
                .code_block(
                    StyleRefinement::default()
                        .bg(cx.theme().transparent)
                        .p_0()
                        // Impulse: the editor's font size, as the prose
                        // around it (hover_popover.rs).
                        .text_size(cx.theme().mono_font_size),
                ),
        )
        // Impulse: no language label in the corner of a code block. The
        // popovers' code blocks have no top padding to clear it (`p_0` above),
        // so it covered the first line, and Monaco's hovers show none. An empty
        // actions element takes its place; the fence's language still picks
        // the syntax colors.
        .code_block_actions(|_, _, _| Empty)
        .selectable(true)
}

/// Impulse: the background of the editor's popovers (Monaco's
/// `editorSuggestWidget.background` and `editorHoverWidget.background`):
/// the highlight theme's `editor.popover.background`, or the theme's popover
/// color when it sets none.
pub(super) fn editor_popover_background(cx: &App) -> Hsla {
    cx.theme()
        .highlight_theme
        .style
        .editor_popover_background
        .unwrap_or(cx.theme().popover)
}

pub(super) fn editor_popover(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .occlude()
        .popover_style(cx)
        // Impulse: the editor's widget background, see above.
        .bg(editor_popover_background(cx))
        .shadow_md()
        .text_xs()
        .p_1()
}
