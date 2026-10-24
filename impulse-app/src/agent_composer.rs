//! The agent composer (the Swift `AgentComposerView` and
//! `toggleAgentComposer`): over a program that owns the terminal (an
//! agent's TUI), write a prompt with a real editor (several lines, @file
//! mentions from the workspace) and hand it to the program as a paste.
//! ⌘↩ (Ctrl+Return on Linux) sends and presses Return, ⌥⌘↩ (Ctrl+Alt+
//! Return) only pastes, Esc puts focus back in the program; ↑ in an empty
//! composer brings back earlier messages.

use std::cell::RefCell;
use std::sync::Arc;

use gpui_kit::{
    div, prelude::*, px, Context, Entity, EventEmitter, FontWeight, SharedString, Subscription,
    Task, Window,
};
use impulse_kit::chrome::{ChromePalette, Rgba as KitRgba};
use impulse_kit::theme::ResolvedTheme;

use crate::command_editor::{CommandEditor, CommandEditorEvent, EditorKey};
use crate::settings_store::SettingsStore;
use crate::terminal_view::TerminalView;
use crate::workbench_chrome::{hsla, icon};

/// Messages kept for ↑/↓.
const HISTORY_LIMIT: usize = 50;
/// Files a mention lists.
const MENTION_LIMIT: usize = 6;

/// The word being typed, when it starts with "@" (the last space-separated
/// one, on the last line).
pub fn mention_query(text: &str) -> Option<&str> {
    let last = text.rsplit(' ').next()?;
    (last.starts_with('@') && !last.contains('\n')).then(|| &last[1..])
}

/// `text` with the mention being typed replaced by `@path` and a space.
pub fn accept_mention(text: &str, path: &str) -> Option<String> {
    let query = mention_query(text)?;
    let kept = &text[..text.len() - query.len() - 1];
    Some(format!("{kept}@{path} "))
}

/// Messages sent from the composer, oldest first, with ↑/↓ moving through
/// them (the Swift `composerHistory` and `historyIndex`).
#[derive(Default, Debug, PartialEq)]
pub struct History {
    pub items: Vec<String>,
}

impl History {
    /// Remember a sent message (not twice in a row).
    pub fn push(&mut self, message: &str) {
        if self.items.last().map(String::as_str) != Some(message) {
            self.items.push(message.to_string());
            if self.items.len() > HISTORY_LIMIT {
                self.items.remove(0);
            }
        }
    }

    /// ↑: an earlier message, from an empty composer or while already
    /// going back. The new position and its text.
    pub fn back(&self, at: Option<usize>, empty: bool) -> Option<(usize, String)> {
        if self.items.is_empty() || !(empty || at.is_some()) {
            return None;
        }
        let index = at.unwrap_or(self.items.len()).saturating_sub(1);
        Some((index, self.items[index].clone()))
    }

    /// ↓: a later message, or past the newest an empty composer again.
    pub fn forward(&self, at: Option<usize>) -> Option<(Option<usize>, String)> {
        let index = at?;
        Some(if index + 1 < self.items.len() {
            (Some(index + 1), self.items[index + 1].clone())
        } else {
            (None, String::new())
        })
    }
}

thread_local! {
    /// The draft and the sent messages, kept while the composer is closed.
    static KEPT: RefCell<(String, History)> = RefCell::new(Default::default());
}

pub enum ComposerEvent {
    /// Paste `text` into the program (and press Return with `submit`).
    Send {
        text: String,
        submit: bool,
    },
    Close,
    /// Ctrl+C: interrupt the program.
    Interrupt,
}

impl EventEmitter<ComposerEvent> for AgentComposer {}

pub struct AgentComposer {
    pub editor: Entity<CommandEditor>,
    palette: ChromePalette,
    theme: ResolvedTheme,
    /// The agent's name, for "Message Claude Code".
    target: String,
    /// The workspace whose files @ mentions.
    root: String,
    trusted: bool,
    mentions: Vec<String>,
    selected: usize,
    files: Option<(String, Arc<Vec<String>>)>,
    /// Position while browsing sent messages with ↑/↓.
    history_index: Option<usize>,
    _index: Option<Task<()>>,
    _subscription: Subscription,
}

impl AgentComposer {
    pub fn new(
        target: String,
        root: String,
        trusted: bool,
        theme: ResolvedTheme,
        palette: ChromePalette,
        cx: &mut Context<Self>,
    ) -> Self {
        let font_size = (SettingsStore::get(cx).terminal_font_size - 1).max(6) as f32;
        let editor = cx.new(|cx| {
            let mut editor = CommandEditor::new(false, &theme, cx);
            editor.shell_syntax = false;
            editor.return_submits = false;
            editor.placeholder = "Describe the change… (@ mentions a file)".into();
            editor.font = gpui_kit::font(crate::workbench::UI_FONT);
            editor.font_size = px(font_size);
            let draft = KEPT.with(|k| k.borrow().0.clone());
            editor.set_text(draft, cx);
            editor
        });
        let subscription = cx.subscribe(&editor, |composer, _, event, cx| match event {
            CommandEditorEvent::Changed => {
                let text = composer.text(cx);
                KEPT.with(|k| k.borrow_mut().0 = text.clone());
                composer.update_mentions(&text, cx);
            }
            CommandEditorEvent::Submit { .. } => composer.send(true, cx),
            CommandEditorEvent::Key(key) => composer.key(*key, cx),
            _ => {}
        });
        AgentComposer {
            editor,
            palette,
            theme,
            target,
            root,
            trusted,
            mentions: Vec::new(),
            selected: 0,
            files: None,
            history_index: None,
            _index: None,
            _subscription: subscription,
        }
    }

    fn text(&self, cx: &gpui_kit::App) -> String {
        self.editor.read(cx).text().to_string()
    }

    pub(crate) fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        KEPT.with(|k| k.borrow_mut().0 = text.clone());
        self.editor.update(cx, |e, cx| e.set_text(text.clone(), cx));
        self.update_mentions(&text, cx);
    }

    fn key(&mut self, key: EditorKey, cx: &mut Context<Self>) {
        let listing = !self.mentions.is_empty();
        match key {
            EditorKey::AlternateSubmit => self.send(false, cx),
            EditorKey::Escape if listing => self.mentions.clear(),
            EditorKey::Escape => cx.emit(ComposerEvent::Close),
            EditorKey::Up if listing => {
                self.selected = (self.selected + self.mentions.len() - 1) % self.mentions.len()
            }
            EditorKey::Down if listing => self.selected = (self.selected + 1) % self.mentions.len(),
            EditorKey::Up => {
                let empty = self.text(cx).is_empty();
                let back = KEPT.with(|k| k.borrow().1.back(self.history_index, empty));
                if let Some((index, text)) = back {
                    self.history_index = Some(index);
                    self.set_text(text, cx);
                }
            }
            EditorKey::Down => {
                let forward = KEPT.with(|k| k.borrow().1.forward(self.history_index));
                if let Some((index, text)) = forward {
                    self.history_index = index;
                    self.set_text(text, cx);
                }
            }
            EditorKey::Tab => {
                if let Some(path) = self.mentions.get(self.selected).cloned() {
                    self.accept(&path, cx);
                }
            }
            EditorKey::ControlC => cx.emit(ComposerEvent::Interrupt),
            _ => {}
        }
        cx.notify();
    }

    fn send(&mut self, submit: bool, cx: &mut Context<Self>) {
        let message = self.text(cx).trim().to_string();
        if message.is_empty() {
            return;
        }
        KEPT.with(|k| k.borrow_mut().1.push(&message));
        self.history_index = None;
        self.set_text(String::new(), cx);
        cx.emit(ComposerEvent::Send {
            text: message,
            submit,
        });
    }

    // MARK: @ mentions

    fn update_mentions(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(query) = mention_query(text).map(str::to_string) else {
            self.mentions.clear();
            return cx.notify();
        };
        if self.root.is_empty() {
            return;
        }
        let files = match &self.files {
            Some((root, files)) if *root == self.root => files.clone(),
            Some(_) | None => {
                self.files = Some((self.root.clone(), Arc::default()));
                let (root, trusted) = (self.root.clone(), self.trusted);
                let listing = cx.background_executor().spawn(async move {
                    impulse_git::file_index::files(
                        &root,
                        impulse_git::file_index::DEFAULT_LIMIT,
                        trusted,
                    )
                });
                self._index = Some(cx.spawn(async move |this, cx| {
                    let files = listing.await;
                    let _ = this.update(cx, |composer, cx| {
                        composer.files = Some((composer.root.clone(), Arc::new(files)));
                        let text = composer.text(cx);
                        composer.update_mentions(&text, cx);
                    });
                }));
                return;
            }
        };
        self.mentions = impulse_kit::fuzzy::rank(&files, &query, MENTION_LIMIT, true, |f| f)
            .into_iter()
            .map(|(path, _)| path.clone())
            .collect();
        self.selected = 0;
        cx.notify();
    }

    fn accept(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(text) = accept_mention(&self.text(cx), path) {
            self.set_text(text, cx);
        }
        self.mentions.clear();
        cx.notify();
    }

    fn mention_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        div()
            .flex()
            .flex_col()
            .p(px(4.))
            .rounded(px(6.))
            .bg(hsla(p.raised))
            .children(self.mentions.iter().enumerate().map(|(index, path)| {
                let chosen = path.clone();
                div()
                    .id(("mention", index))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .when(index == self.selected, |d| d.bg(hsla(p.selection)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |composer, _, _, cx| composer.accept(&chosen, cx)))
                    .child(icon("file", 11., p.text_tertiary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_family("JetBrains Mono")
                            .text_size(px(11.5))
                            .text_color(hsla(p.text))
                            .child(path.clone()),
                    )
            }))
    }
}

/// The Swift `KeyHint`: a shortcut in a hairline box.
pub(crate) fn key_hint(text: &str, p: &ChromePalette) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(4.))
        .h(px(16.))
        .rounded(px(3.))
        .border_1()
        .border_color(hsla(p.hairline_strong))
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(hsla(p.text_tertiary))
        .child(text.to_string())
}

/// The composer's shortcuts as the platform writes them.
fn hints() -> [&'static str; 3] {
    if cfg!(target_os = "macos") {
        ["⌘↩", "⌥⌘↩", "esc"]
    } else {
        ["Ctrl+↩", "Ctrl+Alt+↩", "Esc"]
    }
}

impl Render for AgentComposer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let c = KitRgba::from_hex;
        let theme = &self.theme;
        let [send, paste, escape] = hints();
        let title: SharedString = if self.target.is_empty() {
            "Message".into()
        } else {
            format!("Message {}", self.target).into()
        };
        let caption = |text: &'static str| {
            div()
                .text_size(px(10.5))
                .text_color(hsla(p.text_tertiary))
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .px(px(12.))
            .py(px(8.))
            .bg(hsla(c(&theme.bg_dark)))
            .border_t_1()
            .border_color(hsla(c(&theme.border)))
            .font_family(crate::workbench::UI_FONT)
            .when(!self.mentions.is_empty(), |d| {
                d.child(self.mention_list(cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("bot", 12., p.accent))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.text_secondary))
                            .child(title),
                    )
                    .child(div().flex_1())
                    .child(key_hint(send, &p))
                    .child(caption("send"))
                    .child(key_hint(paste, &p))
                    .child(caption("paste"))
                    .child(key_hint(escape, &p)),
            )
            .child(
                div()
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(hsla(c(&theme.bg)))
                    .border_1()
                    .border_color(hsla(p.accent.opacity(0.6)))
                    .child(self.editor.clone()),
            )
    }
}

impl TerminalView {
    /// Open the composer over the program (an agent's TUI), unless one is
    /// open already. `root` is the workspace @ mentions list files from.
    pub(crate) fn open_composer_at(&mut self, root: String, trusted: bool, cx: &mut Context<Self>) {
        if self.agent.composer.is_some() {
            self.agent.composer_focus_pending = true;
            return cx.notify();
        }
        let target = self
            .agent
            .agent
            .as_ref()
            .map(|a| a.display_name.clone())
            .unwrap_or_default();
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let composer = cx.new(|cx| AgentComposer::new(target, root, trusted, theme, palette, cx));
        cx.subscribe(
            &composer,
            |view, _, event: &ComposerEvent, cx| match event {
                ComposerEvent::Send { text, submit } => {
                    view.paste_into_program(text, *submit, cx);
                    if *submit {
                        view.close_composer(cx);
                    }
                }
                ComposerEvent::Close => view.close_composer(cx),
                ComposerEvent::Interrupt => view.backend.write(&[0x03]),
            },
        )
        .detach();
        self.agent.composer = Some(composer);
        self.agent.composer_focus_pending = true;
        cx.notify();
    }

    /// Open the composer at the terminal's own folder (the agent asked for
    /// input and the setting says to).
    pub(crate) fn open_composer(&mut self, cx: &mut Context<Self>) {
        if self.direct {
            let root = self.cwd.clone();
            self.open_composer_at(root, false, cx);
        }
    }

    /// Close it; the program gets the keyboard back at the next frame.
    pub(crate) fn close_composer(&mut self, cx: &mut Context<Self>) {
        if self.agent.composer.take().is_some() {
            self.agent.composer_focus_pending = true;
            cx.notify();
        }
    }

    /// The composer's element, giving it the keyboard when it just opened.
    /// Only over a program that owns the terminal: at a prompt the input
    /// bar is the place to type.
    pub(crate) fn composer_element(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let composer = self.agent.composer.clone().filter(|_| self.direct)?;
        if std::mem::take(&mut self.agent.composer_focus_pending) {
            let handle = composer.read(cx).editor.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        }
        Some(composer.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_the_last_word() {
        assert_eq!(mention_query("look at @src/ma"), Some("src/ma"));
        assert_eq!(mention_query("@"), Some(""));
        assert_eq!(mention_query("look at src"), None);
        assert_eq!(mention_query("a\n@b"), None);
        assert_eq!(mention_query("@a done "), None);
        assert_eq!(
            accept_mention("fix @ma", "src/main.rs").as_deref(),
            Some("fix @src/main.rs ")
        );
        assert_eq!(accept_mention("fix", "src/main.rs"), None);
    }

    #[test]
    fn history_goes_back_from_an_empty_composer() {
        let mut history = History::default();
        assert_eq!(history.back(None, true), None);
        history.push("one");
        history.push("two");
        history.push("two");
        assert_eq!(history.items, vec!["one", "two"]);
        // Typing something: ↑ moves the cursor, not through history.
        assert_eq!(history.back(None, false), None);
        assert_eq!(history.back(None, true), Some((1, "two".into())));
        assert_eq!(history.back(Some(1), false), Some((0, "one".into())));
        assert_eq!(history.back(Some(0), false), Some((0, "one".into())));
        assert_eq!(history.forward(Some(0)), Some((Some(1), "two".into())));
        assert_eq!(history.forward(Some(1)), Some((None, String::new())));
        assert_eq!(history.forward(None), None);
        for n in 0..60 {
            history.push(&n.to_string());
        }
        assert_eq!(history.items.len(), HISTORY_LIMIT);
        assert_eq!(history.items.last().map(String::as_str), Some("59"));
    }
}
