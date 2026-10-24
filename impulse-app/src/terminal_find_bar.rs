//! Find in a terminal (the Swift `TerminalFindBar` and
//! `MainWindowController+FindBar`): a 32pt bar over the grid with the
//! query, case / whole-word / regex toggles, the match count, previous and
//! next. Return and Shift+Return step through matches, Escape closes. The
//! query becomes the core's regex through `impulse_kit::terminal_find`.

use gpui_kit::{div, prelude::*, px, AnyElement, Context, Entity, FontWeight, MouseButton, Window};
use impulse_kit::chrome::metrics;
use impulse_kit::terminal_find::TerminalFindQuery;
use impulse_terminal::SearchStats;

use crate::command_editor::{CommandEditor, CommandEditorEvent, EditorKey};
use crate::terminal_view::TerminalView;
use crate::workbench_chrome::{hsla, icon};

pub(crate) struct FindBar {
    editor: Entity<CommandEditor>,
    query: TerminalFindQuery,
    stats: SearchStats,
}

impl FindBar {
    pub fn set_colors(&self, colors: crate::command_editor::EditorColors, cx: &mut gpui_kit::App) {
        self.editor.update(cx, |editor, cx| {
            editor.colors = colors;
            cx.notify();
        });
    }

    /// "3 of 12", "No results", ...
    fn status(&self) -> String {
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
}

impl TerminalView {
    /// Find: show the bar (seeded from a one-line selection), or hide it.
    pub fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() {
            return self.close_find(window, cx);
        }
        let theme = self.theme.clone();
        let editor = cx.new(|cx| {
            let mut editor = CommandEditor::new(true, &theme, cx);
            editor.placeholder = "Find in terminal".into();
            editor.font = gpui_kit::font(crate::workbench::UI_FONT);
            editor.font_size = px(12.5);
            editor
        });
        cx.subscribe_in(
            &editor,
            window,
            |view, editor, event, window, cx| match event {
                CommandEditorEvent::Changed => {
                    let text = editor.read(cx).text().to_string();
                    if let Some(find) = view.find.as_mut() {
                        find.query.text = text;
                    }
                    view.run_find(cx);
                }
                CommandEditorEvent::Submit { shift } => view.step_find(!shift, cx),
                CommandEditorEvent::Key(EditorKey::Escape) => view.close_find(window, cx),
                _ => {}
            },
        )
        .detach();
        let selection = self
            .backend
            .selected_text()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && !t.contains('\n'));
        let mut query = TerminalFindQuery::default();
        if let Some(text) = &self.last_find {
            query = text.clone();
        }
        if let Some(selection) = selection {
            query.text = selection;
        }
        let text = query.text.clone();
        editor.update(cx, |e, cx| e.set_text(text, cx));
        let handle = editor.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
        self.find = Some(FindBar {
            editor,
            query,
            stats: SearchStats::default(),
        });
        self.run_find(cx);
    }

    /// Find `text` (the snapshot's `find=`), showing the bar if it's hidden.
    #[cfg(debug_assertions)]
    pub(crate) fn find_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.toggle_find(window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.query.text = text.to_string();
            let editor = find.editor.clone();
            let text = text.to_string();
            editor.update(cx, |e, cx| e.set_text(text, cx));
        }
        self.run_find(cx);
    }

    /// Whole words only (the snapshot's `find-word`).
    #[cfg(debug_assertions)]
    pub(crate) fn find_whole_word(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.toggle_find(window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.query.whole_word = true;
        }
        self.run_find(cx);
    }

    /// Hide the bar, clear the matches and give the terminal the keyboard.
    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.find.take() {
            self.last_find = Some(find.query);
        }
        self.backend.search_clear();
        self.focus(window, cx);
        cx.notify();
    }

    fn run_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        match find.query.pattern() {
            Some(pattern) => {
                self.backend.search(&pattern);
            }
            None => self.backend.search_clear(),
        }
        find.stats = self.backend.search_stats();
        cx.notify();
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.query.pattern().is_none() {
            return;
        }
        if forward {
            self.backend.search_next();
        } else {
            self.backend.search_prev();
        }
        find.stats = self.backend.search_stats();
        cx.notify();
    }

    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let p = &self.palette;
        let status = find.status();
        let failed = find.stats.invalid || (find.stats.total == 0 && !find.query.text.is_empty());
        let none = find.stats.total == 0;
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
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .h(px(32.))
                .bg(hsla(p.panel))
                .border_b_1()
                .border_color(hsla(p.hairline))
                .font_family(crate::workbench::UI_FONT)
                .child(icon("search", 12., p.text_tertiary))
                .child(div().flex_1().min_w(px(120.)).child(find.editor.clone()))
                .child(
                    toggle("find-case", "Aa", find.query.case_sensitive, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(f) = view.find.as_mut() {
                                f.query.case_sensitive = !f.query.case_sensitive;
                            }
                            view.run_find(cx);
                        }),
                    ),
                )
                .child(
                    toggle("find-word", "ab", find.query.whole_word, true).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(f) = view.find.as_mut() {
                                f.query.whole_word = !f.query.whole_word;
                            }
                            view.run_find(cx);
                        }),
                    ),
                )
                .child(
                    toggle("find-regex", ".*", find.query.regex, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(f) = view.find.as_mut() {
                                f.query.regex = !f.query.regex;
                            }
                            view.run_find(cx);
                        }),
                    ),
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
                    cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        view.step_find(false, cx);
                    }),
                ))
                .child(button("find-next", "chevron-down", !none).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        view.step_find(true, cx);
                    }),
                ))
                .child(button("find-close", "x", true).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, window, cx| {
                        cx.stop_propagation();
                        view.close_find(window, cx);
                    }),
                ))
                .into_any_element(),
        )
    }
}
