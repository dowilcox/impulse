//! Peek views (Monaco's peek widget): Peek Definition, and the places Go to
//! References and the other Go to commands find when there is more than
//! one, shown without leaving the file. The selected place is previewed
//! read-only on the left, the places are listed by file on the right; a
//! double click, Enter or Open goes there, Escape closes. Monaco opens the
//! widget under the cursor's line in a view zone; GPUI Kit's editor has
//! none, so it is docked under the editor, as the git peek is.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Editor, EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle, TabSize,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::{div, prelude::*, px, AnyElement, Context, Entity, FontWeight, Window};
use impulse_kit::uri::language_for_path;

use crate::editor_tab::{EditorHooks, EditorTab};

/// The peek under the editor.
pub(crate) struct Peek {
    /// "References", "Definition", "Implementations"…
    title: &'static str,
    items: Vec<PeekItem>,
    selected: usize,
    preview: Entity<EditorState>,
    marks: RangeDecorationCollection,
    /// The file the preview holds.
    previewing: Option<PathBuf>,
    hooks: EditorHooks,
    own_uri: lsp_types::Uri,
}

struct PeekItem {
    location: lsp_types::Location,
    path: PathBuf,
    /// The file's text, shared by its places.
    text: Rc<String>,
    /// The place's line, trimmed, for the list.
    code: String,
}

/// How tall the peek is (Monaco's default is 12 lines of the editor).
const HEIGHT: f32 = 280.;

impl EditorTab {
    /// Show `locations` in a peek titled `title`; the first is selected.
    pub(crate) fn show_peek(
        &mut self,
        title: &'static str,
        locations: Vec<lsp_types::Location>,
        own: (&lsp_types::Uri, &str),
        hooks: EditorHooks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (own_uri, own_text) = own;
        // Each file read once.
        let mut texts: HashMap<String, Option<Rc<String>>> = HashMap::new();
        let items: Vec<PeekItem> = locations
            .into_iter()
            .filter_map(|location| {
                let path = impulse_lsp::workspace_edit::uri_path(&location.uri);
                let text = texts
                    .entry(location.uri.as_str().to_string())
                    .or_insert_with(|| {
                        if &location.uri == own_uri {
                            Some(Rc::new(own_text.to_string()))
                        } else {
                            impulse_kit::text_file::read(&path).map(|c| Rc::new(text_lf(&c.text)))
                        }
                    })
                    .clone()?;
                let code = text
                    .lines()
                    .nth(location.range.start.line as usize)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                Some(PeekItem {
                    location,
                    path,
                    text,
                    code,
                })
            })
            .collect();
        if items.is_empty() {
            return self.say("Nothing to show.", cx);
        }
        let preview = self.peek_state(window, cx);
        let marks = preview.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        self.peek = Some(Peek {
            title,
            items,
            selected: 0,
            preview,
            marks,
            previewing: None,
            hooks,
            own_uri: own_uri.clone(),
        });
        self.select_peek(0, window, cx);
    }

    fn peek_state(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<EditorState> {
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let tab_size = self.tab_size;
        cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .line_number(settings.show_line_numbers)
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: false,
                });
            state.set_readonly(true, cx);
            state
        })
    }

    /// Preview place `index`: its file, its range marked, in the middle.
    fn select_peek(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.as_mut() else {
            return;
        };
        let Some(item) = peek.items.get(index) else {
            return;
        };
        peek.selected = index;
        if peek.previewing.as_ref() != Some(&item.path) {
            // Another file: a fresh editor in its language.
            let language = language_for_path(&item.path);
            let grammar = crate::base_rules::editor_grammar(crate::editor_tab::grammar_for(
                &language, &item.path,
            ))
            .to_string();
            let text = item.text.to_string();
            peek.preview.update(cx, |state, cx| {
                state.set_highlighter(grammar, cx);
                state.set_value(text, window, cx);
            });
            peek.previewing = Some(item.path.clone());
        }
        let start = impulse_lsp::edits::offset_of(&item.text, item.location.range.start, false);
        let end = impulse_lsp::edits::offset_of(&item.text, item.location.range.end, false);
        let color = cx.theme().warning.opacity(0.25);
        let mark = RangeDecoration::new(start..end.max(start))
            .with_style(RangeDecorationStyle::Fill)
            .with_color(color);
        let (marks, preview) = (peek.marks.clone(), peek.preview.clone());
        marks.set(vec![mark], cx);
        let row = item.location.range.start.line as usize;
        crate::editor_diff::center(&preview, row, window, cx);
        cx.notify();
    }

    /// Go to the selected place and close the peek.
    fn open_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.take() else {
            return;
        };
        let Some(item) = peek.items.get(peek.selected) else {
            return;
        };
        if item.location.uri == peek.own_uri {
            let start = item.location.range.start;
            self.state
                .update(cx, |state, cx| state.set_cursor_position(start, window, cx));
        } else {
            let params = lsp_types::ShowDocumentParams {
                uri: item.location.uri.clone(),
                external: Some(false),
                take_focus: Some(true),
                selection: Some(item.location.range),
            };
            // The workbench reads every editor tab, this one included.
            let show = peek.hooks.show_document.clone();
            window.defer(cx, move |window, cx| {
                show(&params, window, cx);
            });
        }
        cx.notify();
    }

    /// Escape: close the peek and go back to typing.
    pub(crate) fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.peek.take().is_none() {
            return false;
        }
        let state = self.state.clone();
        state.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
        true
    }

    /// ↑ ↓ in the list.
    fn step_peek(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(peek) = self.peek.as_ref() else {
            return;
        };
        let count = peek.items.len() as isize;
        let next = (peek.selected as isize + step).rem_euclid(count) as usize;
        self.select_peek(next, window, cx);
    }

    pub(crate) fn peek_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let theme = cx.theme();
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let selected = peek.items.get(peek.selected)?;
        let root = self.git.root.as_deref();
        let (name, dir) = name_and_dir(&selected.path, root);
        let mut editor = Editor::new(&peek.preview)
            .size_full()
            .font_family(settings.font_family.clone())
            .text_size(px(settings.font_size as f32))
            .bordered(false);
        if settings.editor_line_height > 0 {
            editor = editor.line_height(px(settings.editor_line_height as f32));
        }
        let count = peek.items.len();
        let heading = match (peek.title, count) {
            ("References", 1) => "1 reference".to_string(),
            ("References", n) => format!("{n} references"),
            (title, 1) => title.to_string(),
            (title, n) => format!("{title} ({n})"),
        };
        // The list, a header row per file.
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last: Option<&PathBuf> = None;
        for (index, item) in peek.items.iter().enumerate() {
            if last != Some(&item.path) {
                last = Some(&item.path);
                let (file, folder) = name_and_dir(&item.path, root);
                let in_file = peek.items.iter().filter(|i| i.path == item.path).count();
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(2.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(file))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(folder),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(in_file.to_string()),
                        )
                        .into_any_element(),
                );
            }
            let is_selected = index == peek.selected;
            rows.push(
                div()
                    .id(("peek-item", index))
                    .flex()
                    .gap(px(8.))
                    .pl(px(20.))
                    .pr(px(8.))
                    .py(px(1.))
                    .when(is_selected, |d| d.bg(theme.list_active))
                    .hover(|d| d.bg(theme.foreground.opacity(0.06)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(settings.font_family.clone())
                            .child(item.code.clone()),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child((item.location.range.start.line + 1).to_string()),
                    )
                    .on_click(
                        cx.listener(move |tab, event: &gpui_kit::ClickEvent, window, cx| {
                            tab.select_peek(index, window, cx);
                            if event.click_count() >= 2 {
                                tab.open_peek(window, cx);
                            }
                        }),
                    )
                    .into_any_element(),
            );
        }
        let button = |id: &'static str, label: &'static str| Button::new(id).label(label).xsmall();
        Some(
            div()
                .id("peek")
                .flex()
                .flex_col()
                .flex_none()
                .h(px(HEIGHT))
                .border_t_1()
                .border_color(theme.info)
                .bg(theme.secondary)
                .text_size(px(12.))
                .on_key_down(
                    cx.listener(|tab, event: &gpui_kit::KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                tab.close_peek(window, cx);
                            }
                            "enter" => tab.open_peek(window, cx),
                            "up" if event.keystroke.modifiers.alt => tab.step_peek(-1, window, cx),
                            "down" if event.keystroke.modifiers.alt => tab.step_peek(1, window, cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(4.))
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(dir),
                        )
                        .child(div().text_color(theme.muted_foreground).child(heading))
                        .child(
                            button("peek-open", "Open").on_click(
                                cx.listener(|tab, _, window, cx| tab.open_peek(window, cx)),
                            ),
                        )
                        .child(
                            Button::new("peek-places-close")
                                .label("✕")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|tab, _, window, cx| {
                                    tab.close_peek(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                // The theme leaves code editors' background to us.
                                .when_some(
                                    cx.try_global::<crate::kit_theme::EditorBackdrop>(),
                                    |d, b| d.bg(b.background),
                                )
                                .child(editor),
                        )
                        .child(div().w(px(1.)).h_full().bg(theme.border))
                        .child(
                            div()
                                .id("peek-list")
                                .w(px(320.))
                                .flex_none()
                                .h_full()
                                .overflow_y_scroll()
                                .py(px(4.))
                                .children(rows),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// A file's name, and its folder: inside the repository relative to it,
/// elsewhere with the home folder as `~`.
fn name_and_dir(path: &std::path::Path, root: Option<&str>) -> (String, String) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(parent) = path.parent() else {
        return (name, String::new());
    };
    let dir = match root.and_then(|root| parent.strip_prefix(root).ok()) {
        Some(relative) => relative.to_string_lossy().into_owned(),
        None => crate::workbench_chrome::abbreviate_home(&parent.to_string_lossy()),
    };
    (name, dir)
}

/// Text as the editor holds it (`\n` line ends).
fn text_lf(text: &str) -> String {
    impulse_kit::text_file::LineEnding::normalize(text)
}
