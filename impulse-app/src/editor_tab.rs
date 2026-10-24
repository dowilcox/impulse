//! An editor tab: one file in GPUI Kit's `Editor`, loaded and saved with
//! Impulse's rules (`impulse_kit::text_file`): UTF-8 only, byte-order mark
//! kept, writes through symlinks, files over 5 MB read-only, and a save that
//! would overwrite a change made on disk asks first.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, InputEvent, Position, RopeExt, TabSize};
use gpui_kit::{
    prelude::*, px, App, Context, Entity, EntityInputHandler, EventEmitter, Subscription, Window,
};
use impulse_kit::text_file::{self, Opened, Stamp};
use impulse_kit::uri::language_for_path;

use crate::lsp_hub::{DocumentLink, LspHub};

/// Opens a `window/showDocument` target (go to definition in another file)
/// and says whether it did.
pub type ShowDocument = Rc<dyn Fn(&lsp_types::ShowDocumentParams, &mut Window, &mut App) -> bool>;

/// Applies a server's workspace edit across files (the workbench asks first
/// when it has to).
pub type ApplyEdit = Rc<dyn Fn(crate::workspace_edits::PendingEdit, &mut Window, &mut App)>;

/// A dialog's submit, shared by its button and the Return key.
type Submit = Rc<dyn Fn(&mut Window, &mut App)>;

/// What an editor's language features call back into the workbench for.
#[derive(Clone)]
pub struct EditorHooks {
    pub show_document: ShowDocument,
    pub apply_edit: ApplyEdit,
}

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
    /// Indent with spaces (`use_spaces`, or the file type's override).
    pub use_spaces: bool,
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
    /// Set once the workbench starts language servers for this tab.
    hooks: Option<EditorHooks>,
    /// ⌘D: the selection it started from and where the last one it added
    /// ends (GPUI Kit keeps the first selection active).
    occurrences: Option<(std::ops::Range<usize>, usize)>,
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
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let (tab_size, use_spaces) = settings.indentation(&path.to_string_lossy());
        let tab_size = tab_size.max(1) as usize;
        let text = opened.contents.text;
        let grammar = grammar_for(&language).to_string();
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(grammar)
                .line_number(settings.show_line_numbers)
                .folding(settings.folding)
                .indent_guides(settings.indent_guides)
                .soft_wrap(settings.word_wrap)
                .show_whitespaces(settings.render_whitespace == "all")
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: !use_spaces,
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
            use_spaces,
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
            hooks: None,
            occurrences: None,
            _subscription: subscription,
            _lsp_start: Vec::new(),
        }
    }

    /// Take changed settings (settings.json edited, or the Settings tab).
    pub fn apply_settings(
        &mut self,
        settings: &impulse_kit::settings::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (tab_size, use_spaces) = settings.indentation(&self.path.to_string_lossy());
        self.tab_size = tab_size.max(1) as usize;
        self.use_spaces = use_spaces;
        let tab = TabSize {
            tab_size: self.tab_size,
            hard_tabs: !use_spaces,
        };
        self.state.update(cx, |state, cx| {
            state.set_line_number(settings.show_line_numbers, window, cx);
            state.set_folding(settings.folding, window, cx);
            state.set_indent_guides(settings.indent_guides, window, cx);
            state.set_soft_wrap(settings.word_wrap, window, cx);
            state.set_show_whitespaces(settings.render_whitespace == "all", window, cx);
            state.set_tab_size(tab, cx);
        });
        cx.notify();
    }

    /// Start (or reuse) the language's servers, then hand the editor the
    /// first one's providers. Only for files in a trusted folder.
    pub fn start_lsp(
        &mut self,
        hub: &LspHub,
        hooks: EditorHooks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self._lsp_start.is_empty() {
            return;
        }
        self.hooks = Some(hooks.clone());
        let servers = hub.servers_for(&self.path, &self.language, cx);
        self.primary = servers.first().map(|server| server.id.clone());
        for server in servers {
            let hooks = hooks.clone();
            self._lsp_start.push(cx.spawn_in(
                window,
                async move |tab, cx| match server.client.await {
                    Ok(client) => {
                        let _ = tab.update(cx, |tab, cx| tab.attach(server.id, client, hooks, cx));
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
        hooks: EditorHooks,
        cx: &mut Context<Self>,
    ) {
        let text = self.state.read(cx).text().clone();
        let Some(link) = DocumentLink::open(client, &self.path, &self.language, &text) else {
            return;
        };
        *link.apply_edit.borrow_mut() = Some(hooks.apply_edit.clone());
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
                lsp.show_document = Some(hooks.show_document);
            });
        }
        self.links.push((id, link));
    }

    fn primary_link(&self) -> Option<Rc<DocumentLink>> {
        self.links
            .iter()
            .find(|(id, _)| Some(id) == self.primary.as_ref())
            .map(|(_, link)| link.clone())
    }

    /// The primary server's symbols in this file; `None` without a server.
    pub fn document_symbols(
        &self,
        cx: &gpui_kit::App,
    ) -> Option<gpui_kit::Task<Result<Vec<impulse_kit::document_symbols::OutlineSymbol>, String>>>
    {
        let link = self.primary_link()?;
        Some(link.document_symbols(self.state.read(cx).text(), cx))
    }

    /// The primary server's project symbols matching `query`; `None`
    /// without a server.
    #[allow(clippy::type_complexity)]
    pub fn workspace_symbols(
        &self,
        query: String,
        cx: &gpui_kit::App,
    ) -> Option<
        gpui_kit::Task<Result<Vec<(impulse_kit::document_symbols::OutlineSymbol, String)>, String>>,
    > {
        Some(self.primary_link()?.workspace_symbols(query, cx))
    }

    fn say(&mut self, notice: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(notice.into());
        cx.notify();
    }

    /// Rename Symbol (F2): ask the server where the symbol is, ask for the
    /// new name, then apply the server's edit across files.
    pub fn rename_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(link), Some(hooks)) = (self.primary_link(), self.hooks.clone()) else {
            return self.say("No language server renames in this file.", cx);
        };
        let (text, offset) = {
            let state = self.state.read(cx);
            (state.text().clone(), state.cursor())
        };
        let prepare = link.prepare_rename(&text, offset, cx);
        let word = text
            .word_range(offset)
            .map(|range| text.slice(range).to_string())
            .unwrap_or_default();
        cx.spawn_in(window, async move |tab, cx| {
            let current = match prepare.await {
                Ok(Some(lsp_types::PrepareRenameResponse::RangeWithPlaceholder {
                    placeholder,
                    ..
                })) => placeholder,
                Ok(_) => word,
                Err(error) => {
                    let _ = tab.update(cx, |tab, cx| {
                        tab.say(format!("Can't rename this: {error}"), cx)
                    });
                    return;
                }
            };
            if current.is_empty() {
                let _ = tab.update(cx, |tab, cx| {
                    tab.say("Put the cursor on a symbol to rename it.", cx)
                });
                return;
            }
            let _ = tab.update_in(cx, |tab, window, cx| {
                tab.ask_new_name(link, hooks, text, offset, current, window, cx)
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn ask_new_name(
        &mut self,
        link: Rc<DocumentLink>,
        hooks: EditorHooks,
        text: gpui_kit::component::input::Rope,
        offset: usize,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use gpui_kit::component::input::{Input, InputState};
        use gpui_kit::component::WindowExt as _;

        let input = cx.new(|cx| InputState::new(window, cx).default_value(current.clone()));
        // The dialog focuses itself when it opens: the field takes the focus
        // (with the name selected) the first time the dialog is built.
        let initial_focus = std::cell::Cell::new(true);
        let tab = cx.entity().downgrade();
        let submit: Submit = Rc::new({
            let (input, tab) = (input.clone(), tab.clone());
            move |window, cx| {
                let new_name = input.read(cx).value().trim().to_string();
                window.close_dialog(cx);
                if new_name.is_empty() || new_name == current {
                    return;
                }
                let task = link.rename(&text, offset, new_name.clone(), cx);
                let (link, hooks, tab) = (link.clone(), hooks.clone(), tab.clone());
                window
                    .spawn(cx, async move |cx| match task.await {
                        Ok(Some(edit)) => {
                            let pending = crate::workspace_edits::PendingEdit {
                                edit: impulse_lsp::workspace_edit::WorkspaceEdit::from(&edit),
                                utf32: link.client.uses_utf32(),
                                origin: link.path.clone(),
                                server_root: link.client.root.clone(),
                                title: format!("Rename to {new_name}"),
                            };
                            let _ = cx.update(|window, cx| (hooks.apply_edit)(pending, window, cx));
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let _ = tab.update(cx, |tab, cx| {
                                tab.say(format!("Couldn't rename: {error}"), cx)
                            });
                        }
                    })
                    .detach();
            }
        });
        let on_enter = submit.clone();
        cx.subscribe_in(
            &input,
            window,
            move |_, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    on_enter(window, cx);
                }
            },
        )
        .detach();
        let field = input.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                field.update(cx, |field, cx| {
                    field.focus(window, cx);
                    field.select_all(window, cx);
                });
            }
            let submit = submit.clone();
            dialog
                .title("Rename Symbol")
                .w(px(420.))
                .close_button(false)
                .child(Input::new(&input))
                .footer(
                    gpui_kit::div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("rename-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("rename-ok")
                                .primary()
                                .label("Rename")
                                .on_click(move |_, window, cx| submit(window, cx)),
                        ),
                )
        });
    }

    /// Find References (⇧F12): every place the symbol is used, each with its
    /// line; picking one goes there.
    pub fn find_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(link), Some(hooks)) = (self.primary_link(), self.hooks.clone()) else {
            return self.say("No language server finds references in this file.", cx);
        };
        let (text, offset) = {
            let state = self.state.read(cx);
            (state.text().clone(), state.cursor())
        };
        let task = link.references(&text, offset, cx);
        let own_text = text.to_string();
        let own_uri = link.uri.clone();
        cx.spawn_in(window, async move |tab, cx| {
            let locations = match task.await {
                Ok(locations) => locations,
                Err(error) => {
                    let _ = tab.update(cx, |tab, cx| {
                        tab.say(format!("Couldn't find references: {error}"), cx)
                    });
                    return;
                }
            };
            let _ = tab.update_in(cx, |tab, window, cx| {
                if locations.is_empty() {
                    return tab.say("No references found.", cx);
                }
                tab.show_references(locations, &own_uri, &own_text, hooks, window, cx)
            });
        })
        .detach();
    }

    fn show_references(
        &mut self,
        locations: Vec<lsp_types::Location>,
        own_uri: &lsp_types::Uri,
        own_text: &str,
        hooks: EditorHooks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;

        // Each location's file, line number and line of code.
        // By URI (as text: `Uri` caches its parse internally).
        let mut texts: std::collections::HashMap<String, Option<String>> = Default::default();
        let rows: Vec<(String, String, lsp_types::Location)> = locations
            .into_iter()
            .map(|location| {
                let text = texts
                    .entry(location.uri.as_str().to_string())
                    .or_insert_with(|| {
                        if &location.uri == own_uri {
                            Some(own_text.to_string())
                        } else {
                            impulse_kit::text_file::read(&impulse_lsp::workspace_edit::uri_path(
                                &location.uri,
                            ))
                            .map(|c| c.text)
                        }
                    });
                let line = location.range.start.line as usize;
                let code = text
                    .as_deref()
                    .and_then(|t| t.lines().nth(line))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let path = impulse_lsp::workspace_edit::uri_path(&location.uri);
                let shown = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (format!("{shown}:{}", line + 1), code, location)
            })
            .collect();
        let count = rows.len();
        let state = self.state.clone();
        let own_uri = own_uri.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows = rows.clone();
            let (hooks, state, own_uri) = (hooks.clone(), state.clone(), own_uri.clone());
            dialog
                .title(format!("{count} references"))
                .w(px(640.))
                .child(
                    gpui_kit::div()
                        .id("references")
                        .flex()
                        .flex_col()
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .children(rows.into_iter().enumerate().map(
                            move |(index, (place, code, location))| {
                                let (hooks, state, own_uri) =
                                    (hooks.clone(), state.clone(), own_uri.clone());
                                gpui_kit::div()
                                    .id(("reference", index))
                                    .flex()
                                    .gap(px(12.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(gpui_kit::black().opacity(0.15)))
                                    .font_family("JetBrains Mono")
                                    .text_size(px(12.))
                                    .child(gpui_kit::div().flex_none().w(px(160.)).child(place))
                                    .child(gpui_kit::div().truncate().child(code))
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        if location.uri == own_uri {
                                            state.update(cx, |state, cx| {
                                                state.set_cursor_position(
                                                    location.range.start,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        } else {
                                            let params = lsp_types::ShowDocumentParams {
                                                uri: location.uri.clone(),
                                                external: Some(false),
                                                take_focus: Some(true),
                                                selection: Some(location.range),
                                            };
                                            (hooks.show_document)(&params, window, cx);
                                        }
                                    })
                            },
                        )),
                )
        });
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
        let task = link.format(&text, self.tab_size, self.use_spaces, cx);
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

    /// The version of this document a server last saw, for `uri`.
    pub fn document_version(&self, uri: &lsp_types::Uri) -> Option<i32> {
        self.links
            .iter()
            .find(|(_, link)| &link.uri == uri)
            .map(|(_, link)| link.version())
    }

    /// A server's edits to this file, as one undo step.
    pub fn apply_server_edits(
        &mut self,
        edits: &[lsp_types::TextEdit],
        utf32: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let old = self.state.read(cx).value().to_string();
        if impulse_lsp::edits::apply_text_edits(&old, edits, utf32).is_none() {
            return Err(format!("The changes to {} overlap.", self.title));
        }
        crate::lsp_hub::apply_edits(&self.state, edits, utf32, window, cx);
        Ok(())
    }

    /// Apply one of Monaco's line commands (`impulse_kit::line_edits`) as one
    /// undo step, then put the selection where it says.
    fn line_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&str, std::ops::Range<usize>) -> Option<impulse_kit::line_edits::LineEdit>,
    ) {
        if self.read_only {
            return;
        }
        self.state.update(cx, |state, cx| {
            let text = state.value().to_string();
            let Some(change) = edit(&text, state.selected_range()) else {
                return;
            };
            let utf16 = |byte: usize| text[..byte].encode_utf16().count();
            state.replace_text_in_range(
                Some(utf16(change.range.start)..utf16(change.range.end)),
                &change.text,
                window,
                cx,
            );
            state.set_selected_range(change.selection, cx);
        });
    }

    fn toggle_line_comment(
        &mut self,
        _: &crate::editor_keys::ToggleLineComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prefix) = impulse_kit::line_edits::line_comment(&self.language) else {
            return;
        };
        self.line_edit(window, cx, |text, selection| {
            Some(impulse_kit::line_edits::toggle_comment(
                text, selection, prefix,
            ))
        });
    }

    fn move_lines_up(
        &mut self,
        _: &crate::editor_keys::MoveLinesUp,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            impulse_kit::line_edits::move_lines(t, s, true)
        });
    }

    fn move_lines_down(
        &mut self,
        _: &crate::editor_keys::MoveLinesDown,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            impulse_kit::line_edits::move_lines(t, s, false)
        });
    }

    fn copy_lines_up(
        &mut self,
        _: &crate::editor_keys::CopyLinesUp,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            Some(impulse_kit::line_edits::copy_lines(t, s, true))
        });
    }

    fn copy_lines_down(
        &mut self,
        _: &crate::editor_keys::CopyLinesDown,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            Some(impulse_kit::line_edits::copy_lines(t, s, false))
        });
    }

    fn delete_lines(
        &mut self,
        _: &crate::editor_keys::DeleteLines,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            Some(impulse_kit::line_edits::delete_lines(t, s))
        });
    }

    fn insert_line_below(
        &mut self,
        _: &crate::editor_keys::InsertLineBelow,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            Some(impulse_kit::line_edits::insert_line(t, s, false))
        });
    }

    fn insert_line_above(
        &mut self,
        _: &crate::editor_keys::InsertLineAbove,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(w, cx, |t, s| {
            Some(impulse_kit::line_edits::insert_line(t, s, true))
        });
    }

    /// ⌘D: the word at the cursor, then each next place it appears, as
    /// another cursor.
    fn add_next_occurrence(
        &mut self,
        _: &crate::editor_keys::AddNextOccurrence,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, selection) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        // Carry on from the last one added while the selection it started
        // from is still the active one.
        let after = match &self.occurrences {
            Some((start, after)) if *start == selection => *after,
            _ => selection.end,
        };
        let Some(next) = impulse_kit::line_edits::next_occurrence(&text, selection.clone(), after)
        else {
            return;
        };
        self.state.update(cx, |state, cx| {
            if selection.is_empty() {
                state.set_selected_range(next.clone(), cx);
            } else {
                state.add_selection(next.clone(), cx);
            }
        });
        let start = if selection.is_empty() {
            next.clone()
        } else {
            selection
        };
        self.occurrences = Some((start, next.end));
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

    pub fn focus_handle(&self, cx: &gpui_kit::App) -> gpui_kit::FocusHandle {
        use gpui_kit::Focusable as _;
        self.state.read(cx).focus_handle(cx)
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = crate::settings_store::SettingsStore::get(cx);
        let (family, size) = (settings.font_family.clone(), settings.font_size as f32);
        let line_height = settings.editor_line_height;
        let mut editor = Editor::new(&self.state)
            .size_full()
            .font_family(family)
            .text_size(px(size))
            .bordered(false);
        if line_height > 0 {
            editor = editor.line_height(px(line_height as f32));
        }
        gpui_kit::div()
            .key_context(crate::editor_keys::CONTEXT)
            .size_full()
            .on_action(cx.listener(Self::toggle_line_comment))
            .on_action(cx.listener(Self::move_lines_up))
            .on_action(cx.listener(Self::move_lines_down))
            .on_action(cx.listener(Self::copy_lines_up))
            .on_action(cx.listener(Self::copy_lines_down))
            .on_action(cx.listener(Self::delete_lines))
            .on_action(cx.listener(Self::insert_line_below))
            .on_action(cx.listener(Self::insert_line_above))
            .on_action(cx.listener(Self::add_next_occurrence))
            .child(editor)
    }
}
