//! The find bar terminals and editors share (the Swift `TerminalFindBar`'s
//! look): a 32pt bar with the query, case / whole-word / regex toggles,
//! the "3 of 12" / "No results" / "Invalid pattern" status, previous, next
//! and close. Return and Shift+Return step through matches, Escape closes.
//! An editor's bar also has Monaco's replace row, behind the chevron at its
//! left (or ⌥⌘F): the replacement, Replace and Replace All.
//!
//! The bar is only the view and the query. Its owner, a `FindHost`, does
//! the searching: the terminal core's regex search (`terminal_find_bar`),
//! or GPUI Kit's editor search (`editor_find`).

use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, Entity, FontWeight, MouseButton, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::terminal_find::TerminalFindQuery;
use impulse_kit::theme::ResolvedTheme;
use impulse_terminal::SearchStats;

use crate::command_editor::{CommandEditor, CommandEditorEvent, EditorColors, EditorKey};
use crate::workbench_chrome::{hsla, icon};

/// The window's theme and chrome colors, for surfaces that aren't handed
/// them (an editor's find bar). Set with GPUI Kit's theme
/// (`kit_theme::apply`).
#[derive(Clone)]
pub(crate) struct ChromeTheme {
    pub theme: ResolvedTheme,
    pub palette: ChromePalette,
}

impl gpui_kit::Global for ChromeTheme {}

/// What a find bar's owner does when the bar asks.
pub(crate) trait FindHost: Sized + 'static {
    fn find_bar_mut(&mut self) -> Option<&mut FindBar>;
    /// The query or a toggle changed: search again.
    fn find_changed(&mut self, window: &mut Window, cx: &mut Context<Self>);
    /// Previous or next match.
    fn find_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>);
    /// Escape or the close button.
    fn find_close(&mut self, window: &mut Window, cx: &mut Context<Self>);
    /// Replace or Replace All, with the replace row's text. Only bars made
    /// with a replace row ask.
    fn find_replace(&mut self, _all: bool, _window: &mut Window, _cx: &mut Context<Self>) {}
}

/// The replace row: its field and whether it's showing.
struct ReplaceRow {
    field: Entity<CommandEditor>,
    shown: bool,
}

pub(crate) struct FindBar {
    field: Entity<CommandEditor>,
    pub query: TerminalFindQuery,
    /// The matches: how many, which one is current (1-based, 0 for none),
    /// and whether the pattern compiles.
    pub stats: SearchStats,
    replace: Option<ReplaceRow>,
}

impl FindBar {
    /// A bar showing `query`, its field saying `placeholder` while empty,
    /// with a replace row (hidden until asked for) when `replace`.
    pub fn new<T: FindHost>(
        theme: &ResolvedTheme,
        placeholder: &'static str,
        query: TerminalFindQuery,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let make = |placeholder: &'static str, cx: &mut Context<T>| {
            cx.new(|cx| {
                let mut editor = CommandEditor::new(true, theme, cx);
                editor.placeholder = placeholder.into();
                editor.font = gpui_kit::font(crate::workbench::UI_FONT);
                editor.font_size = px(12.5);
                editor
            })
        };
        let field = make(placeholder, cx);
        cx.subscribe_in(&field, window, |host: &mut T, field, event, window, cx| {
            match event {
                CommandEditorEvent::Changed => {
                    let text = field.read(cx).text().to_string();
                    if let Some(find) = host.find_bar_mut() {
                        find.query.text = text;
                    }
                    host.find_changed(window, cx);
                }
                CommandEditorEvent::Submit { shift } => host.find_step(!shift, window, cx),
                CommandEditorEvent::Key(EditorKey::Escape) => host.find_close(window, cx),
                // Tab goes to the replace field, as in Monaco.
                CommandEditorEvent::Key(EditorKey::Tab) => {
                    if let Some(replace) = host.find_bar_mut().and_then(|f| f.replace.as_ref()) {
                        if replace.shown {
                            let handle = replace.field.read(cx).focus_handle().clone();
                            window.focus(&handle, cx);
                        }
                    }
                }
                _ => {}
            }
        })
        .detach();
        let text = query.text.clone();
        field.update(cx, |e, cx| e.set_text(text, cx));
        let replace = replace.then(|| {
            let field = make("Replace", cx);
            cx.subscribe_in(
                &field,
                window,
                |host: &mut T, _, event, window, cx| match event {
                    CommandEditorEvent::Submit { .. } => host.find_replace(false, window, cx),
                    CommandEditorEvent::Key(EditorKey::Escape) => host.find_close(window, cx),
                    CommandEditorEvent::Key(EditorKey::Tab) => {
                        if let Some(find) = host.find_bar_mut() {
                            let handle = find.field.read(cx).focus_handle().clone();
                            window.focus(&handle, cx);
                        }
                    }
                    _ => {}
                },
            )
            .detach();
            ReplaceRow {
                field,
                shown: false,
            }
        });
        FindBar {
            field,
            query,
            stats: SearchStats::default(),
            replace,
        }
    }

    pub fn set_colors(&self, colors: EditorColors, cx: &mut App) {
        let fields = std::iter::once(&self.field).chain(self.replace.as_ref().map(|r| &r.field));
        for field in fields {
            let colors = colors.clone();
            field.update(cx, |editor, cx| {
                editor.colors = colors;
                cx.notify();
            });
        }
    }

    /// Give the query field the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let handle = self.field.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    /// Give the query field the keyboard with its text selected, as a
    /// second ⌘F does in Monaco.
    pub fn focus_selected(&self, window: &mut Window, cx: &mut App) {
        let handle = self.field.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
        handle.dispatch_action(&crate::command_editor::SelectAll, window, cx);
    }

    /// Whether the query field has the keyboard.
    pub fn field_focused(&self, window: &Window, cx: &App) -> bool {
        self.field.read(cx).focus_handle().is_focused(window)
    }

    /// Whether either field has the keyboard.
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.field_focused(window, cx)
            || self
                .replace
                .as_ref()
                .is_some_and(|r| r.field.read(cx).focus_handle().is_focused(window))
    }

    /// Show the query in the field (not as typing: no `Changed`).
    pub fn set_query_text(&mut self, text: &str, cx: &mut App) {
        self.query.text = text.to_string();
        let text = text.to_string();
        self.field.update(cx, |e, cx| e.set_text(text, cx));
    }

    /// The replace row is showing.
    pub fn replace_shown(&self) -> bool {
        self.replace.as_ref().is_some_and(|r| r.shown)
    }

    /// Show or hide the replace row (a bar without one stays as it is).
    /// Showing it gives its field the keyboard when `focus`.
    pub fn show_replace(&mut self, shown: bool, focus: bool, window: &mut Window, cx: &mut App) {
        let Some(replace) = self.replace.as_mut() else {
            return;
        };
        replace.shown = shown;
        if shown && focus {
            let handle = replace.field.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        } else if !shown && replace.field.read(cx).focus_handle().is_focused(window) {
            self.focus(window, cx);
        }
    }

    /// Whether the replace row's field has the keyboard.
    pub fn replace_focused(&self, window: &Window, cx: &App) -> bool {
        self.replace
            .as_ref()
            .is_some_and(|r| r.shown && r.field.read(cx).focus_handle().is_focused(window))
    }

    /// The replace row's text.
    pub fn replacement(&self, cx: &App) -> String {
        self.replace
            .as_ref()
            .map(|r| r.field.read(cx).text().to_string())
            .unwrap_or_default()
    }

    /// "3 of 12", "No results", ...
    pub fn status(&self) -> String {
        if self.query.text.is_empty() {
            return String::new();
        }
        if self.stats.invalid {
            return "Invalid pattern".into();
        }
        if self.stats.total == 0 {
            return "No results".into();
        }
        let count = if self.stats.capped {
            format!("{}+", self.stats.total)
        } else {
            self.stats.total.to_string()
        };
        if self.stats.current > 0 {
            format!("{} of {count}", self.stats.current)
        } else {
            format!("{count} found")
        }
    }

    /// The bar, its clicks going to `T`.
    pub fn render<T: FindHost>(&self, p: &ChromePalette, cx: &mut Context<T>) -> AnyElement {
        let status = self.status();
        let failed = self.stats.invalid || (self.stats.total == 0 && !self.query.text.is_empty());
        let none = self.stats.total == 0;
        let toggle = |id: &'static str, label: &'static str, on: bool, underline: bool| {
            div()
                .id(id)
                .w(px(24.))
                .h(px(20.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .border_1()
                .border_color(if on {
                    hsla(p.accent.opacity(0.5))
                } else {
                    gpui_kit::transparent_black()
                })
                .when(on, |d| d.bg(hsla(p.accent.opacity(0.16))))
                .font_family("JetBrains Mono")
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hsla(if on { p.accent } else { p.text_secondary }))
                .when(underline, |d| d.underline())
                .cursor_pointer()
                .child(label)
        };
        let button = |id: &'static str, name: &'static str, enabled: bool| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|d| d.bg(hsla(p.hover)))
                })
                .child(icon(
                    name,
                    13.,
                    if enabled {
                        p.text_secondary
                    } else {
                        p.text_tertiary
                    },
                ))
        };
        // A toggle's click: flip the option, search again.
        let flip = |set: fn(&mut TerminalFindQuery)| {
            cx.listener(move |host: &mut T, _, window, cx| {
                cx.stop_propagation();
                if let Some(find) = host.find_bar_mut() {
                    set(&mut find.query);
                }
                host.find_changed(window, cx);
            })
        };
        let row = || {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .h(px(32.))
        };
        let replace_shown = self.replace_shown();
        // Monaco's chevron, left of the query, showing the replace row.
        let chevron = self.replace.as_ref().map(|_| {
            button(
                "find-replace-toggle",
                if replace_shown {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                true,
            )
            .w(px(16.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|host: &mut T, _, window, cx| {
                    cx.stop_propagation();
                    if let Some(find) = host.find_bar_mut() {
                        let shown = !find.replace_shown();
                        find.show_replace(shown, shown, window, cx);
                    }
                    cx.notify();
                }),
            )
        });
        let find_row = row()
            .children(chevron)
            .child(icon("search", 12., p.text_tertiary))
            .child(div().flex_1().min_w(px(120.)).child(self.field.clone()))
            .child(
                toggle("find-case", "Aa", self.query.case_sensitive, false).on_mouse_down(
                    MouseButton::Left,
                    flip(|q| q.case_sensitive = !q.case_sensitive),
                ),
            )
            .child(
                toggle("find-word", "ab", self.query.whole_word, true)
                    .on_mouse_down(MouseButton::Left, flip(|q| q.whole_word = !q.whole_word)),
            )
            .child(
                toggle("find-regex", ".*", self.query.regex, false)
                    .on_mouse_down(MouseButton::Left, flip(|q| q.regex = !q.regex)),
            )
            .child(
                div()
                    .min_w(px(84.))
                    .flex()
                    .flex_none()
                    .justify_end()
                    .text_size(px(11.5))
                    .text_color(hsla(if failed { p.danger } else { p.text_secondary }))
                    .child(status),
            )
            .child(button("find-previous", "chevron-up", !none).on_mouse_down(
                MouseButton::Left,
                cx.listener(|host: &mut T, _, window, cx| {
                    cx.stop_propagation();
                    host.find_step(false, window, cx);
                }),
            ))
            .child(button("find-next", "chevron-down", !none).on_mouse_down(
                MouseButton::Left,
                cx.listener(|host: &mut T, _, window, cx| {
                    cx.stop_propagation();
                    host.find_step(true, window, cx);
                }),
            ))
            .child(button("find-close", "x", true).on_mouse_down(
                MouseButton::Left,
                cx.listener(|host: &mut T, _, window, cx| {
                    cx.stop_propagation();
                    host.find_close(window, cx);
                }),
            ));
        // Replace and Replace All: text buttons the toggles' size, since
        // the icon set has no replace glyphs.
        let text_button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .h(px(20.))
                .px(px(6.))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .text_size(px(11.5))
                .text_color(hsla(if enabled {
                    p.text_secondary
                } else {
                    p.text_tertiary
                }))
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|d| d.bg(hsla(p.hover)))
                })
                .child(label)
        };
        let replace_row = self.replace.as_ref().filter(|r| r.shown).map(|replace| {
            row()
                // Under the chevron and the search icon, so the two fields
                // line up.
                .child(div().w(px(16.)).flex_none())
                .child(div().w(px(12.)).flex_none())
                .child(div().flex_1().min_w(px(120.)).child(replace.field.clone()))
                .child(
                    text_button("find-replace-one", "Replace", !none).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|host: &mut T, _, window, cx| {
                            cx.stop_propagation();
                            host.find_replace(false, window, cx);
                        }),
                    ),
                )
                .child(
                    text_button("find-replace-all", "Replace All", !none).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|host: &mut T, _, window, cx| {
                            cx.stop_propagation();
                            host.find_replace(true, window, cx);
                        }),
                    ),
                )
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .font_family(crate::workbench::UI_FONT)
            .child(find_row)
            .children(replace_row)
            .into_any_element()
    }
}
