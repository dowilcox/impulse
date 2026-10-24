//! An editor tab: one file in GPUI Kit's `Editor`, loaded and saved with
//! Impulse's rules (`impulse_kit::text_file`): UTF-8 only, byte-order mark
//! kept, writes through symlinks, files over 5 MB read-only, and a save that
//! would overwrite a change made on disk asks first.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, InputEvent, Position, RopeExt, TabSize};
use gpui_kit::{prelude::*, px, App, Context, Entity, EventEmitter, Subscription, Window};
use impulse_kit::text_file::{self, Opened, Stamp};
use impulse_kit::uri::language_for_path;

use crate::lsp_hub::{DocumentLink, LspHub};

/// Opens a `window/showDocument` target (go to definition in another file)
/// and says whether it did.
pub type ShowDocument = Rc<dyn Fn(&lsp_types::ShowDocumentParams, &mut Window, &mut App) -> bool>;

/// The tab's title or state changed (dirty mark, notice).
pub struct EditorTabChanged;

pub struct EditorTab {
    pub path: PathBuf,
    pub title: String,
    /// Impulse's language id (`language_from_uri`), shown in the status bar.
    pub language: String,
    pub dirty: bool,
    pub read_only: bool,
    pub tab_size: usize,
    /// A message for the status bar ("changed on disk", a failed save).
    pub notice: Option<String>,
    state: Entity<EditorState>,
    bom: bool,
    stamp: Option<Stamp>,
    /// The next save overwrites a change made on disk.
    overwrite_confirmed: bool,
    /// The document's language server, once it's running.
    link: Option<Rc<DocumentLink>>,
    _subscription: Subscription,
    _lsp_start: Option<gpui_kit::Task<()>>,
}

impl EventEmitter<EditorTabChanged> for EditorTab {}

/// GPUI Kit's grammar name for Impulse's language id.
fn grammar_for(language: &str) -> &str {
    match language {
        "shellscript" => "bash",
        "typescriptreact" => "tsx",
        "javascriptreact" => "javascript",
        "md" => "markdown",
        "yml" => "yaml",
        other => other,
    }
}

impl EditorTab {
    /// A tab for `path`, read with `text_file::open` (which refuses what the
    /// editor can't show).
    pub fn new(
        path: &Path,
        opened: Opened,
        lsp: Option<(&LspHub, ShowDocument)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = language_for_path(path);
        let tab_size = 4;
        let text = opened.contents.text;
        let grammar = grammar_for(&language).to_string();
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(grammar)
                .line_number(true)
                .folding(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: false,
                })
                .default_value(text);
            if opened.large {
                state.set_readonly(true, cx);
            }
            state
        });
        let subscription = cx.subscribe(&state, |tab, state, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if let Some(link) = &tab.link {
                link.sync(state.read(cx).text());
            }
            if !tab.dirty {
                tab.dirty = true;
                cx.emit(EditorTabChanged);
                cx.notify();
            }
        });
        // Start (or reuse) the language's server, then hand the editor its
        // providers.
        let lsp_start = lsp.and_then(|(hub, show_document)| {
            let client = hub.client_for(&language, cx)?;
            Some(
                cx.spawn_in(window, async move |tab, cx| match client.await {
                    Ok(client) => {
                        let _ = tab.update(cx, |tab, cx| tab.attach(client, show_document, cx));
                    }
                    Err(error) => log::warn!("language server: {error}"),
                }),
            )
        });
        Self {
            title: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_path_buf(),
            language,
            dirty: false,
            read_only: opened.large,
            tab_size,
            notice: opened
                .large
                .then(|| "Larger than 5 MB: opened read-only.".to_string()),
            state,
            bom: opened.contents.bom,
            stamp: opened.stamp,
            overwrite_confirmed: false,
            link: None,
            _subscription: subscription,
            _lsp_start: lsp_start,
        }
    }

    fn attach(
        &mut self,
        client: std::sync::Arc<impulse_lsp::LspClient>,
        show_document: ShowDocument,
        cx: &mut Context<Self>,
    ) {
        let text = self.state.read(cx).text().clone();
        let Some(link) = DocumentLink::open(client, &self.path, &self.language, &text) else {
            return;
        };
        self.state.update(cx, |state, _| {
            let lsp = state.lsp_mut();
            lsp.completion_provider = Some(link.clone());
            lsp.hover_provider = Some(link.clone());
            lsp.definition_provider = Some(link.clone());
            lsp.show_document = Some(show_document);
        });
        self.link = Some(link);
    }

    /// Whether `uri` is this tab's document.
    pub fn is_document(&self, uri: &lsp_types::Uri) -> bool {
        self.link.as_ref().is_some_and(|link| &link.uri == uri)
    }

    /// The server's diagnostics for this file, replacing the last ones.
    pub fn set_diagnostics(
        &mut self,
        diagnostics: Vec<lsp_types::Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        let utf32 = self
            .link
            .as_ref()
            .is_some_and(|link| link.client.uses_utf32());
        self.state.update(cx, |state, cx| {
            let text = state.text().clone();
            let to_chars = |p: lsp_types::Position| -> lsp_types::Position {
                if utf32 || p.line as usize >= text.lines_len() {
                    return p;
                }
                // UTF-16 columns to characters, which GPUI Kit's ranges use.
                let line = text.slice_line(p.line as usize);
                let byte = line.utf16_to_byte_idx((p.character as usize).min(line.len_utf16()));
                lsp_types::Position::new(p.line, line.byte_to_char_idx(byte) as u32)
            };
            if let Some(set) = state.diagnostics_mut() {
                set.clear();
                set.extend(diagnostics.into_iter().map(|mut d| {
                    d.range.start = to_chars(d.range.start);
                    d.range.end = to_chars(d.range.end);
                    gpui_kit::component::highlighter::Diagnostic::from(d)
                }));
            }
            cx.notify();
        });
    }

    /// Put the cursor at a 0-based line and character.
    pub fn go_to(&self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.set_cursor_position(position, window, cx)
        });
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.focus(window, cx));
    }

    /// 1-based line and column of the cursor, as the status bar shows them.
    pub fn cursor(&self, cx: &gpui_kit::App) -> (u32, u32) {
        let position = self.state.read(cx).cursor_position();
        (position.line + 1, position.character + 1)
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        // Something else rewrote the file since it was loaded: say so, and
        // let a second save overwrite it.
        let on_disk = text_file::stamp(&self.path);
        if self.stamp.is_some() && on_disk != self.stamp && !self.overwrite_confirmed {
            self.overwrite_confirmed = true;
            self.notice = Some(format!(
                "{} changed on disk. Save again to replace it with your version.",
                self.title
            ));
            cx.emit(EditorTabChanged);
            cx.notify();
            return;
        }
        let text = self.state.read(cx).value();
        match text_file::write(&text, self.bom, &self.path) {
            Ok(()) => {
                if let Some(link) = &self.link {
                    link.saved();
                }
                self.dirty = false;
                self.overwrite_confirmed = false;
                self.notice = None;
                self.stamp = text_file::stamp(&self.path);
            }
            Err(error) => self.notice = Some(format!("Couldn't save {}: {error}", self.title)),
        }
        cx.emit(EditorTabChanged);
        cx.notify();
    }
}

impl Drop for EditorTab {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.closed();
        }
    }
}

impl Render for EditorTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Editor::new(&self.state)
            .size_full()
            .font_family("JetBrains Mono")
            .text_size(px(14.))
            .bordered(false)
    }
}
