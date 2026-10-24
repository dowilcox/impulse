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
    /// The document's language servers that are running, by server id.
    links: Vec<(String, Rc<DocumentLink>)>,
    /// The server that answers completion, hover and definition (the first
    /// the config names).
    primary: Option<String>,
    /// Each server's last diagnostics for this file, shown together.
    diagnostics: std::collections::BTreeMap<String, Vec<lsp_types::Diagnostic>>,
    _subscription: Subscription,
    _lsp_start: Vec<gpui_kit::Task<()>>,
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
    pub fn new(path: &Path, opened: Opened, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            for (_, link) in &tab.links {
                link.sync(state.read(cx).text());
            }
            if !tab.dirty {
                tab.dirty = true;
                cx.emit(EditorTabChanged);
                cx.notify();
            }
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
            links: Vec::new(),
            primary: None,
            diagnostics: Default::default(),
            _subscription: subscription,
            _lsp_start: Vec::new(),
        }
    }

    /// Start (or reuse) the language's servers, then hand the editor the
    /// first one's providers. Only for files in a trusted folder.
    pub fn start_lsp(
        &mut self,
        hub: &LspHub,
        show_document: ShowDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self._lsp_start.is_empty() {
            return;
        }
        let servers = hub.servers_for(&self.path, &self.language, cx);
        self.primary = servers.first().map(|server| server.id.clone());
        for server in servers {
            let show_document = show_document.clone();
            self._lsp_start.push(cx.spawn_in(
                window,
                async move |tab, cx| match server.client.await {
                    Ok(client) => {
                        let _ = tab.update(cx, |tab, cx| {
                            tab.attach(server.id, client, show_document, cx)
                        });
                    }
                    Err(error) => log::warn!("{}: {error}", server.id),
                },
            ));
        }
    }

    fn attach(
        &mut self,
        id: String,
        client: std::sync::Arc<impulse_lsp::LspClient>,
        show_document: ShowDocument,
        cx: &mut Context<Self>,
    ) {
        let text = self.state.read(cx).text().clone();
        let Some(link) = DocumentLink::open(client, &self.path, &self.language, &text) else {
            return;
        };
        let primary = self.primary.as_deref() == Some(id.as_str());
        self.state.update(cx, |state, _| {
            // Every server's fixes (eslint's beside typescript's).
            state.lsp_mut().code_action_providers.push(link.clone());
        });
        if primary {
            self.state.update(cx, |state, _| {
                let lsp = state.lsp_mut();
                lsp.completion_provider = Some(link.clone());
                lsp.hover_provider = Some(link.clone());
                lsp.definition_provider = Some(link.clone());
                lsp.show_document = Some(show_document);
            });
        }
        self.links.push((id, link));
    }

    /// Format Document: the primary server's formatting, as one edit.
    pub fn format_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self
            .links
            .iter()
            .find(|(id, _)| Some(id) == self.primary.as_ref())
            .map(|(_, link)| link.clone())
        else {
            self.notice = Some("No language server formats this file.".into());
            cx.notify();
            return;
        };
        let text = self.state.read(cx).text().clone();
        let task = link.format(&text, self.tab_size, cx);
        let state = self.state.clone();
        let utf32 = link.client.uses_utf32();
        cx.spawn_in(window, async move |tab, cx| match task.await {
            Ok(edits) => {
                let _ = cx.update(|window, cx| {
                    crate::lsp_hub::apply_edits(&state, &edits, utf32, window, cx)
                });
            }
            Err(error) => {
                let _ = tab.update(cx, |tab, cx| {
                    tab.notice = Some(format!("Couldn't format: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The servers running for this file, for the status bar.
    pub fn server_names(&self) -> Vec<String> {
        self.links.iter().map(|(id, _)| id.clone()).collect()
    }

    /// Whether `uri` is this tab's document.
    pub fn is_document(&self, uri: &lsp_types::Uri) -> bool {
        self.links.iter().any(|(_, link)| &link.uri == uri)
    }

    /// One server's diagnostics for this file, replacing its last ones; the
    /// editor shows every server's together.
    pub fn set_diagnostics(
        &mut self,
        server: String,
        diagnostics: Vec<lsp_types::Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        let utf32: std::collections::HashMap<String, bool> = self
            .links
            .iter()
            .map(|(id, link)| (id.clone(), link.client.uses_utf32()))
            .collect();
        self.diagnostics.insert(server, diagnostics);
        let all: Vec<(bool, lsp_types::Diagnostic)> = self
            .diagnostics
            .iter()
            .flat_map(|(id, list)| {
                let utf32 = utf32.get(id).copied().unwrap_or(false);
                list.iter().cloned().map(move |d| (utf32, d))
            })
            .collect();
        self.state.update(cx, |state, cx| {
            let text = state.text().clone();
            let to_chars = |utf32: bool, p: lsp_types::Position| -> lsp_types::Position {
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
                set.extend(all.into_iter().map(|(utf32, mut d)| {
                    d.range.start = to_chars(utf32, d.range.start);
                    d.range.end = to_chars(utf32, d.range.end);
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
                for (_, link) in &self.links {
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
        for (_, link) in &self.links {
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
