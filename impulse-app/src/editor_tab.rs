//! An editor tab: one file in GPUI Kit's `Editor`, loaded and saved with
//! Impulse's rules (`impulse_kit::text_file`): UTF-8 only, byte-order mark
//! kept, writes through symlinks, files over 5 MB read-only, and a save that
//! would overwrite a change made on disk asks first.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{
    CompletionProvider, Editor, EditorState, InputEvent, Position, RangeDecoration,
    RangeDecorationCollection, RangeDecorationStyle, RopeExt, TabSize,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    prelude::*, px, App, Context, Entity, EntityInputHandler, EventEmitter, Subscription, Window,
};
use impulse_kit::save_command::RewriteOutcome;
use impulse_kit::text_file::{self, LineEnding, Opened, Stamp};
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

/// Rename Symbol's field, and where the symbol starts.
struct RenameBox {
    input: Entity<gpui_kit::component::input::InputState>,
    at: usize,
    _subscription: Subscription,
}

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
    /// A new file not saved yet (New File): the folder Save As starts in.
    /// `path` is empty meanwhile.
    pub untitled: Option<PathBuf>,
    pub title: String,
    /// Impulse's language id (`language_from_uri`), shown in the status bar.
    pub language: String,
    pub dirty: bool,
    pub read_only: bool,
    pub tab_size: usize,
    /// Indent with spaces (`use_spaces`, or the file type's override).
    pub use_spaces: bool,
    /// The next change is a reload from disk, not an edit.
    reloading: bool,
    /// A Markdown preview: instead of the editor (false) or beside it (true).
    pub preview: Option<bool>,
    /// The previewed text and its blocks (`markdown_view`).
    markdown: std::cell::RefCell<
        Option<(
            String,
            std::rc::Rc<Vec<impulse_kit::markdown_blocks::Block>>,
        )>,
    >,
    /// A message for the status bar ("changed on disk", a failed save).
    pub notice: Option<String>,
    pub(crate) state: Entity<EditorState>,
    bom: bool,
    /// The file's line ending, put back on write.
    pub line_ending: LineEnding,
    stamp: Option<Stamp>,
    /// Formatters and commands on save rewriting the file now.
    rewriting: u32,
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
    /// Cursor Undo's and Shrink Selection's earlier selections.
    pub(crate) selections: crate::editor_actions::SelectionHistory,
    /// Other places the symbol at the cursor appears, and other matches of
    /// the selection (Monaco's occurrence and selection highlights).
    marks: Option<RangeDecorationCollection>,
    /// Diff View: the file's index version beside the editor, changed lines
    /// tinted.
    pub(crate) diff: Option<crate::editor_diff::DiffView>,
    /// Diff View's last layout: inline rather than side by side.
    pub(crate) diff_inline: bool,
    /// Places a Go to or Peek command found, under the editor
    /// (`editor_peek`).
    pub(crate) peek: Option<crate::editor_peek::Peek>,
    /// The call's signature over the cursor (Monaco's parameter hints).
    signature: Option<lsp_types::SignatureHelp>,
    /// Rename Symbol's box over the editor (`rename_box`).
    rename: Option<RenameBox>,
    _signature_request: Option<gpui_kit::Task<()>>,
    /// The selection the marks were worked out for.
    marked_at: Option<std::ops::Range<usize>>,
    /// The text has the keyboard: Monaco marks occurrences only then.
    text_focused: bool,
    /// Vim mode, with `editor_vim_mode` on (`editor_vim`).
    pub(crate) vim: Option<crate::editor_vim::Attached>,
    /// Change marks, the peek and conflicts (`editor_git`).
    pub(crate) git: crate::editor_git::GitLayer,
    /// The pointer over the minimap (`editor_minimap`).
    pub(crate) minimap: crate::editor_minimap::MinimapState,
    /// The find bar at the top, while it's open (`editor_find`).
    pub(crate) find: Option<crate::find_bar::FindBar>,
    /// The last find's query and toggles, for the next ⌘F and F3.
    pub(crate) last_find: impulse_kit::terminal_find::TerminalFindQuery,
    _marking: Option<gpui_kit::Task<()>>,
    _subscription: Subscription,
    _observe_state: Subscription,
    /// Linked tag names (`impulse_kit::linked_tags`): the pair the cursor
    /// is in, and the edit that follows the other one.
    pub(crate) linked: crate::editor_actions::LinkedTags,
    _linked_edits: Subscription,
    _lsp_start: Vec<gpui_kit::Task<()>>,
    /// Servers being started for this file (`_lsp_start`), not attached yet.
    lsp_pending: std::collections::HashSet<String>,
    /// The file grew past the editor's limit and the pager was offered.
    too_large_told: bool,
    /// It's settings.json, checked against its schema as it changes.
    settings_file: bool,
    /// The Markdown preview's scroll, for its `#anchor` links.
    preview_scroll: gpui_kit::ScrollHandle,
    /// Where `go_to` put the cursor before the editor's first layout: its
    /// line is centered once there is one.
    pending_center: Option<Position>,
}

impl EventEmitter<EditorTabChanged> for EditorTab {}

/// GPUI Kit's grammar name for Impulse's language id, by the path's
/// extension where the id covers two grammars (TypeScript and TSX).
pub(crate) fn grammar_for<'a>(language: &'a str, path: &Path) -> &'a str {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("tsx" | "jsx")) {
        return "tsx";
    }
    match language {
        "shell" | "shellscript" => "bash",
        "less" => "css",
        "typescriptreact" => "tsx",
        "javascriptreact" => "javascript",
        "md" => "markdown",
        "yml" => "yaml",
        other => other,
    }
}

/// What Monaco is given in `editor.js` that GPUI Kit's editor takes through
/// Impulse's patches (`vendor/gpui-kit/README.md`): the caret's style and
/// blinking, whitespace markers, auto-closing brackets, fold chevrons only
/// while the pointer is over the gutter, the theme's line number colors and
/// a selection that stays (dimmer) when the window is inactive, one scroll
/// axis per trackpad gesture, the I-beam kept while ⌥ is held, a 300 ms
/// hover and a scrollbar that stays shown. The Diff View's sides take them
/// too.
pub(crate) fn apply_editor_options(
    state: &mut EditorState,
    settings: &impulse_kit::settings::Settings,
    cx: &mut Context<EditorState>,
) {
    use gpui_kit::base::input::{
        AutoCloseBrackets, CaretStyle, CursorBlinking, EditorColors, FoldingControls,
        RenderWhitespace,
    };
    use gpui_kit::component::ActiveTheme as _;
    let colors = {
        let theme = cx.theme();
        let style = &theme.highlight_theme.style;
        EditorColors {
            line_number: style.editor_line_number,
            active_line_number: style.editor_active_line_number,
            // Monaco's `editor.inactiveSelectionBackground`: the selection
            // at half its opacity.
            inactive_selection: Some(theme.selection.opacity(0.5)),
            // `editorCursor.background`: the editor's background.
            block_caret_text: Some(theme.background),
        }
    };
    state.set_editor_colors(colors, cx);
    let caret = match settings.editor_cursor_style.as_str() {
        "block" => CaretStyle::Block,
        "underline" => CaretStyle::Underline,
        "line-thin" => CaretStyle::LineThin,
        "block-outline" => CaretStyle::BlockOutline,
        "underline-thin" => CaretStyle::UnderlineThin,
        _ => CaretStyle::Line,
    };
    state.set_caret_style(caret, cx);
    let blinking = match settings.editor_cursor_blinking.as_str() {
        "blink" => CursorBlinking::Blink,
        "phase" => CursorBlinking::Phase,
        "expand" => CursorBlinking::Expand,
        "solid" => CursorBlinking::Solid,
        _ => CursorBlinking::Smooth,
    };
    state.set_cursor_blinking(blinking, cx);
    let whitespace = match settings.render_whitespace.as_str() {
        "none" => RenderWhitespace::None,
        "all" => RenderWhitespace::All,
        "boundary" => RenderWhitespace::Boundary,
        "trailing" => RenderWhitespace::Trailing,
        _ => RenderWhitespace::Selection,
    };
    state.set_render_whitespace(whitespace, cx);
    // "never" turns auto-closing off (`set_auto_close`).
    let brackets = match settings.editor_auto_closing_brackets.as_str() {
        "always" => AutoCloseBrackets::Always,
        "beforeWhitespace" => AutoCloseBrackets::BeforeWhitespace,
        _ => AutoCloseBrackets::LanguageDefined,
    };
    state.set_auto_close_brackets(brackets, cx);
    state.set_folding_controls(FoldingControls::Mouseover, cx);
    state.set_scroll_predominant_axis(true);
    state.set_hide_column_select_pointer(true, cx);
    state.set_hover_delay(std::time::Duration::from_millis(300));
}

impl EditorTab {
    /// A tab for `path`, read with `text_file::open` (which refuses what the
    /// editor can't show).
    pub fn new(path: &Path, opened: Opened, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = language_for_path(path);
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let (tab_size, use_spaces) = settings.indentation(&path.to_string_lossy());
        let tab_size = tab_size.max(1) as usize;
        // The editor works on `\n`; a CRLF file gets its CRLF back on write.
        let line_ending = LineEnding::detect(&opened.contents.text);
        let text = LineEnding::normalize(&opened.contents.text);
        let grammar = crate::base_rules::editor_grammar(grammar_for(&language, path)).to_string();
        let tab = cx.entity().downgrade();
        cx.default_global::<OpenDocuments>().0.push(tab);
        let state = cx.new(|cx| {
            // Find is Impulse's bar (`editor_find`), not GPUI Kit's panel.
            let mut state = EditorState::new(window, cx)
                .searchable(false)
                .language(grammar)
                .line_number(settings.show_line_numbers)
                .folding(settings.folding)
                .indent_guides(settings.indent_guides)
                .soft_wrap(settings.word_wrap)
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: !use_spaces,
                })
                .default_value(text);
            if opened.large {
                state.set_readonly(true, cx);
            }
            // Links open on Cmd+click (Ctrl+click) before a server answers.
            state.lsp_mut().definition_provider =
                Some(Rc::new(crate::editor_links::LinkDefinitions {
                    server: None,
                }));
            state
        });
        let subscription = cx.subscribe(&state, |tab, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Blur) {
                tab.hide_signature(cx);
                // Save when focus leaves the editor (`auto_save`).
                let auto_save = crate::settings_store::SettingsStore::get(cx).auto_save;
                if auto_save && tab.dirty && tab.untitled.is_none() && !tab.read_only {
                    cx.emit(crate::editor_git::EditorGitAction {
                        action: "save".into(),
                        line: 0,
                    });
                }
            }
            if !matches!(event, InputEvent::Change) {
                return;
            }
            for (_, link) in &tab.links {
                link.sync(state.read(cx).text());
            }
            tab.git_text_changed(cx);
            tab.find_text_changed(cx);
            if tab.settings_file {
                tab.recheck_settings_file(cx);
            }
            // A reload from disk isn't an edit of the user's.
            if tab.reloading {
                tab.reloading = false;
                cx.notify();
                return;
            }
            // A preview beside the editor follows every edit, as does the
            // Diff View.
            if tab.preview.is_some() {
                cx.notify();
            }
            if tab.diff.is_some() {
                tab.refresh_diff(crate::editor_diff::DIFF_DELAY, cx);
            }
            tab.typed_for_signature(cx);
            if !tab.dirty {
                tab.dirty = true;
                cx.emit(EditorTabChanged);
                cx.notify();
            }
        });
        let linked_edits =
            cx.subscribe_in(&state, window, |tab, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    tab.follow_linked_tag(window, cx);
                }
            });
        let observe_state = cx.observe(&state, |tab, _, cx| {
            tab.selection_moved(cx);
            // Laid out at last: render centers `go_to`'s line.
            if tab.pending_center.is_some() {
                cx.notify();
            }
        });
        let vim = crate::editor_vim::Attached::new(settings.editor_vim_mode, &state, cx);
        let mut tab = Self {
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
            preview: None,
            markdown: Default::default(),
            untitled: None,
            reloading: false,
            notice: opened
                .large
                .then(|| "Larger than 5 MB: opened read-only.".to_string()),
            state,
            bom: opened.contents.bom,
            line_ending,
            stamp: opened.stamp,
            rewriting: 0,
            links: Vec::new(),
            primary: None,
            diagnostics: Default::default(),
            hooks: None,
            occurrences: None,
            selections: Default::default(),
            marks: None,
            diff: None,
            signature: None,
            rename: None,
            _signature_request: None,
            marked_at: None,
            text_focused: false,
            vim,
            peek: None,
            diff_inline: false,
            git: Default::default(),
            minimap: Default::default(),
            find: None,
            last_find: Default::default(),
            _marking: None,
            _subscription: subscription,
            _observe_state: observe_state,
            linked: Default::default(),
            _linked_edits: linked_edits,
            _lsp_start: Vec::new(),
            lsp_pending: Default::default(),
            too_large_told: false,
            settings_file: false,
            preview_scroll: gpui_kit::ScrollHandle::new(),
            pending_center: None,
        };
        tab.apply_settings(&settings, window, cx);
        tab.refresh_git(cx);
        if crate::settings_json::is_settings_file(&tab.path) {
            tab.check_settings_file(cx);
        }
        tab
    }

    /// settings.json: the schema's completions and hover, and its problems
    /// as diagnostics (`settings_json`).
    fn check_settings_file(&mut self, cx: &mut Context<Self>) {
        self.settings_file = true;
        let provider = Rc::new(crate::settings_json::SchemaProvider);
        self.state.update(cx, |state, _| {
            let lsp = state.lsp_mut();
            lsp.completion_provider = Some(provider.clone());
            lsp.hover_provider = Some(provider);
        });
        self.recheck_settings_file(cx);
    }

    fn recheck_settings_file(&mut self, cx: &mut Context<Self>) {
        let problems = crate::settings_json::diagnostics(self.state.read(cx).text());
        self.set_diagnostics(crate::settings_json::SOURCE.into(), problems, cx);
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
            apply_editor_options(state, settings, cx);
            state.set_tab_size(tab, cx);
            state.set_scroll_beyond_last_line(
                if settings.scroll_beyond_last_line {
                    None
                } else {
                    Some(0)
                },
                window,
                cx,
            );
            state.set_cursor_surrounding_lines(
                Some(settings.editor_cursor_surrounding_lines.max(0) as usize),
                window,
                cx,
            );
            state.set_auto_close(settings.editor_auto_closing_brackets != "never", window, cx);
        });
        // The theme may have changed with the settings.
        self.find_retheme(cx);
        // Without a language server, the document's own words (settings.json
        // keeps its schema's completions).
        if self.primary_link().is_none() && !self.settings_file {
            let words = settings.editor_word_based_suggestions != "off";
            let provider = WordCompletions {
                tab: cx.entity().downgrade(),
                language: self.language.clone(),
            };
            self.state.update(cx, |state, _| {
                state.lsp_mut().completion_provider = words
                    .then(|| std::rc::Rc::new(provider) as std::rc::Rc<dyn CompletionProvider>);
            });
        }
        crate::editor_vim::Attached::update(
            &mut self.vim,
            settings.editor_vim_mode,
            &self.state,
            cx,
        );
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
        self.hooks = Some(hooks.clone());
        // Servers lsp.json no longer names for this language go.
        let configured = crate::lsp_hub::LspHub::configured(&self.path, &self.language);
        let dropped: Vec<String> = self
            .links
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| !configured.contains(id))
            .collect();
        for id in dropped {
            if let Some((_, link)) = self.links.iter().find(|(server, _)| *server == id) {
                link.closed();
            }
            self.detach_server(&id, cx);
        }
        let servers = hub.servers_for(&self.path, &self.language, cx);
        if self.primary.is_none() || self.links.is_empty() {
            self.primary = servers.first().map(|server| server.id.clone());
        }
        // Servers already attached or on their way stay (another one
        // crashed and is back, or lsp.json changed).
        let servers: Vec<_> = servers
            .into_iter()
            .filter(|s| !self.serves(&s.id) && !self.lsp_pending.contains(&s.id))
            .collect();
        for server in servers {
            let hooks = hooks.clone();
            self.lsp_pending.insert(server.id.clone());
            self._lsp_start
                .push(cx.spawn_in(window, async move |tab, cx| {
                    let started = server.client.await;
                    let _ = tab.update(cx, |tab, cx| {
                        tab.lsp_pending.remove(&server.id);
                        match started {
                            Ok(client) => tab.attach(server.id, client, hooks, cx),
                            Err(error) => log::warn!("{}: {error}", server.id),
                        }
                    });
                }));
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
        if let Some(list) = self.diagnostics.get(&id) {
            *link.diagnostics.borrow_mut() = list.clone();
        }
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
                lsp.definition_provider = Some(Rc::new(crate::editor_links::LinkDefinitions {
                    server: Some(link.clone()),
                }));
                lsp.show_document = Some(hooks.show_document);
            });
        }
        self.links.push((id, link));
    }

    /// Whether server `id` serves this file.
    pub(crate) fn serves(&self, id: &str) -> bool {
        self.links.iter().any(|(server, _)| server == id)
    }

    /// Drop the servers that were told to stop; whether there were any.
    pub(crate) fn detach_retired(&mut self, cx: &mut Context<Self>) -> bool {
        let retired: Vec<String> = self
            .links
            .iter()
            .filter(|(_, link)| link.client.is_retired())
            .map(|(id, _)| id.clone())
            .collect();
        for id in &retired {
            self.detach_server(id, cx);
        }
        !retired.is_empty()
    }

    /// Drop server `id` (it crashed), so `start_lsp` attaches it again.
    pub(crate) fn detach_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.links)
            .into_iter()
            .partition(|(server, _)| server == id);
        self.links = kept;
        // Its documents end with it (a restarted server is told them anew).
        for (_, link) in &gone {
            link.closed();
        }
        let primary = self.primary.as_deref() == Some(id);
        self.state.update(cx, |state, _| {
            let lsp = state.lsp_mut();
            lsp.code_action_providers.retain(|provider| {
                !gone
                    .iter()
                    .any(|(_, link)| std::ptr::addr_eq(Rc::as_ptr(provider), Rc::as_ptr(link)))
            });
            if primary {
                lsp.completion_provider = None;
                lsp.hover_provider = None;
                lsp.definition_provider = Some(Rc::new(crate::editor_links::LinkDefinitions {
                    server: None,
                }));
            }
        });
        self._lsp_start.clear();
        self.lsp_pending.clear();
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

    pub(crate) fn say(&mut self, notice: impl Into<String>, cx: &mut Context<Self>) {
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
        use gpui_kit::component::input::InputState;

        let input = cx.new(|cx| InputState::new(window, cx).default_value(current.clone()));
        let tab = cx.entity().downgrade();
        // Under the symbol's first character, as Monaco's rename box.
        let at = text.word_range(offset).map_or(offset, |range| range.start);
        let submit: Submit = Rc::new({
            let (input, tab) = (input.clone(), tab.clone());
            move |window, cx| {
                let new_name = input.read(cx).value().trim().to_string();
                let _ = tab.update(cx, |tab, cx| tab.close_rename(window, cx));
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
        // Return renames; leaving the box (Escape, a click elsewhere) doesn't.
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |tab, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => submit(window, cx),
                InputEvent::Blur => tab.close_rename(window, cx),
                _ => {}
            },
        );
        input.update(cx, |field, cx| {
            field.focus(window, cx);
            field.select_all(window, cx);
        });
        self.rename = Some(RenameBox {
            input,
            at,
            _subscription: subscription,
        });
        cx.notify();
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
        self.show_peek(
            "References",
            locations,
            (own_uri, own_text),
            hooks,
            window,
            cx,
        );
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
                    // The edits are for the text as it was asked about: typed
                    // over since, they'd land in the wrong places, so they're
                    // dropped, as Monaco cancels a format when its model
                    // changes.
                    if *state.read(cx).text() == text {
                        crate::lsp_hub::apply_edits(&state, &edits, utf32, window, cx)
                    }
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
    pub(crate) fn line_edit(
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

    /// Where each server's problems start, with whether it counts columns
    /// in characters.
    pub(crate) fn problem_positions(&self) -> Vec<(lsp_types::Position, bool)> {
        let utf32 = |id: &str| {
            self.links
                .iter()
                .find(|(server, _)| server == id)
                .is_some_and(|(_, link)| link.client.uses_utf32())
        };
        self.diagnostics
            .iter()
            .flat_map(|(id, list)| list.iter().map(move |d| (d.range.start, utf32(id))))
            .collect()
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
        if let Some((_, link)) = self.links.iter().find(|(id, _)| *id == server) {
            *link.diagnostics.borrow_mut() = diagnostics.clone();
        }
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

    /// Put the cursor at a 0-based line and character, its line in the
    /// middle of the view (Go to Line, search results, problems,
    /// definitions: Swift's `revealPositionInCenter`). A tab not laid out
    /// yet centers it after its first layout.
    pub fn go_to(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        let centered = crate::editor_diff::center_at(&self.state, position, window, cx);
        self.pending_center = (!centered).then_some(position);
        cx.notify();
    }

    /// Center the line `go_to` asked for before the first layout, once
    /// there is one (unless the cursor has moved on since).
    fn apply_pending_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(position) = self.pending_center else {
            return;
        };
        let state = self.state.read(cx);
        if state.visible_row_range().is_none() {
            return;
        }
        self.pending_center = None;
        if state.cursor_position() == position {
            crate::editor_diff::center_at(&self.state, position, window, cx);
        }
    }

    /// Show or hide the Markdown or SVG preview (`beside`: next to the
    /// editor). An error message for files that can't be previewed.
    pub fn toggle_preview(&mut self, beside: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let path = self.path.to_string_lossy();
        if !impulse_kit::markdown_preview::is_previewable_file(&path) {
            return Err("Open a Markdown or SVG file to preview it.".into());
        }
        self.preview = match self.preview {
            Some(current) if current == beside => None,
            _ => Some(beside),
        };
        cx.notify();
        Ok(())
    }

    fn preview_element(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let text = self.state.read(cx).text().to_string();
        if impulse_kit::svg_preview::is_svg_file(&self.path.to_string_lossy()) {
            return Self::svg_preview(&text);
        }
        // Parsed again only when the text changed.
        let blocks = {
            let mut cache = self.markdown.borrow_mut();
            match cache.as_ref() {
                Some((source, blocks)) if *source == text => blocks.clone(),
                _ => {
                    let blocks = std::rc::Rc::new(impulse_kit::markdown_blocks::parse(&text));
                    *cache = Some((text, blocks.clone()));
                    blocks
                }
            }
        };
        let folder = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let scroll = self.preview_scroll.clone();
        crate::markdown_view::preview(&blocks, &folder, &scroll, cx)
    }

    /// The SVG drawn centered and scaled down to fit, without its scripts,
    /// `foreignObject` and event handlers (`svg_preview::sanitize_svg`).
    fn svg_preview(source: &str) -> gpui_kit::AnyElement {
        let body = if source.len() > impulse_kit::svg_preview::MAX_SVG_SIZE {
            gpui_kit::div()
                .child("This SVG is too large to preview.")
                .into_any_element()
        } else {
            let svg = impulse_kit::svg_preview::sanitize_svg(source);
            let image = std::sync::Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Svg,
                svg.into_bytes(),
            ));
            gpui_kit::img(image)
                .max_w_full()
                .max_h_full()
                .object_fit(gpui_kit::ObjectFit::Contain)
                .into_any_element()
        };
        gpui_kit::div()
            .id("svg-preview")
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .child(body)
            .into_any_element()
    }

    #[cfg(debug_assertions)]
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

    /// The selected text and its first and last 1-based lines (Send
    /// Selection to Agent); `None` with nothing selected.
    pub fn selected_text(&self, cx: &gpui_kit::App) -> Option<(String, u32, u32)> {
        let state = self.state.read(cx);
        let range = state.selected_range();
        let value = state.value();
        let text = value.get(range.clone()).filter(|t| !t.is_empty())?;
        let line = |offset: usize| value[..offset].matches('\n').count() as u32 + 1;
        Some((text.to_string(), line(range.start), line(range.end)))
    }

    /// The file changed on disk (Replace All, a discard): show it, as one
    /// undoable edit, unless there are unsaved edits here, which win.
    pub fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            return;
        }
        // Grown past what the editor holds: the buffer stays, read-only,
        // and the pager is offered (once, as Swift's `diskTooLarge`).
        if text_file::stamp(&self.path).is_some_and(|s| s.size > text_file::MAX_EDITOR_FILE_SIZE) {
            self.read_only = true;
            if !self.too_large_told {
                self.too_large_told = true;
                cx.emit(crate::editor_git::EditorGitAction {
                    action: "too-large".into(),
                    line: 0,
                });
            }
            cx.notify();
            return;
        }
        let Some(file) = text_file::read(&self.path) else {
            return;
        };
        // Grown large while open: from now on it's treated like a file
        // opened that large (read-only, changes announced, not reloaded).
        if file.text.len() as u64 > text_file::LARGE_FILE_SIZE {
            self.read_only = true;
        }
        self.line_ending = LineEnding::detect(&file.text);
        self.bom = file.bom;
        let disk = LineEnding::normalize(&file.text);
        let old = self.state.read(cx).value().to_string();
        let (range, replacement) = impulse_lsp::edits::changed_span(&old, &disk);
        // Only a reload that changes the buffer raises the change event the
        // flag answers; set with nothing to change, it would swallow the
        // user's next edit.
        if !(range.is_empty() && replacement.is_empty()) {
            self.reloading = true;
            let utf16 = |byte: usize| old[..byte].encode_utf16().count();
            self.state.update(cx, |state, cx| {
                state.replace_text_in_range(
                    Some(utf16(range.start)..utf16(range.end)),
                    replacement,
                    window,
                    cx,
                );
            });
            if self.diff.is_some() {
                self.refresh_diff(std::time::Duration::ZERO, cx);
            }
        }
        // Reloading isn't an edit of the user's.
        self.dirty = false;
        self.stamp = text_file::stamp(&self.path);
        cx.emit(EditorTabChanged);
        cx.notify();
    }

    /// The buffer's text (`\n` line ends).
    pub fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).value().to_string()
    }

    /// Run a small editing script for the headless snapshots (the Rust
    /// app's stand-in for driving Monaco with JavaScript), one command a
    /// line: `cursor L:C`, `select L:C-L:C` (1-based, columns in
    /// characters), `type TEXT` and `paste TEXT` (`\n` for a line break,
    /// replacing the selection), `undo`, `redo`, `find TEXT` (the find bar
    /// with that query). Answers the buffer's text.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_edit(
        &mut self,
        script: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        use gpui_kit::component::input::{Redo, Undo};
        let offset = |text: &str, at: &str| -> Option<usize> {
            let (line, column) = at.split_once(':')?;
            let (line, column) = (line.parse::<usize>().ok()?, column.parse::<usize>().ok()?);
            let start = text
                .split_inclusive('\n')
                .take(line.saturating_sub(1))
                .map(str::len)
                .sum::<usize>();
            let rest = &text[start..];
            let within = rest
                .char_indices()
                .nth(column.saturating_sub(1))
                .map_or(rest.trim_end_matches('\n').len(), |(i, _)| i);
            Some(start + within)
        };
        // Leading space is indentation; trailing space is typed.
        for line in script
            .lines()
            .map(str::trim_start)
            .filter(|l| !l.is_empty())
        {
            let (command, argument) = line.split_once(' ').unwrap_or((line, ""));
            let argument = argument.replace("\\n", "\n");
            let text = self.text(cx);
            match command {
                "cursor" | "select" => {
                    let (from, to) = argument.split_once('-').unwrap_or((&argument, &argument));
                    if let (Some(from), Some(to)) = (offset(&text, from), offset(&text, to)) {
                        self.state
                            .update(cx, |state, cx| state.set_selected_range(from..to, cx));
                    }
                }
                "type" | "paste" => self
                    .state
                    .update(cx, |state, cx| state.replace(argument.clone(), window, cx)),
                // Show Commit for This Line, at the cursor.
                "line-commit" => self.show_line_commit(cx),
                "find" => self.debug_find(&argument, window, cx),
                "undo" | "redo" => {
                    let focus = self.focus_handle(cx);
                    if command == "undo" {
                        focus.dispatch_action(&Undo, window, cx);
                    } else {
                        focus.dispatch_action(&Redo, window, cx);
                    }
                }
                _ => log::warn!("edit: unknown command “{line}”"),
            }
        }
        self.text(cx)
    }

    /// Ask the primary server what Monaco's providers would for the
    /// headless snapshots (`lsp=`): hover texts, completion labels, the
    /// first definition (`{uri, line}`, 1-based), or a rename applied as
    /// Rename Symbol applies it (`true`). `line` and `character` are the
    /// LSP's (0-based). `None` without a server.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_lsp(
        &mut self,
        kind: &str,
        line: u32,
        character: u32,
        new_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Task<serde_json::Value>> {
        use gpui_kit::component::input::{CompletionProvider, DefinitionProvider, HoverProvider};
        use serde_json::{json, Value};
        let link = self.primary_link()?;
        let text = self.state.read(cx).text().clone();
        let offset = text.line_start_offset(line as usize) + character as usize;
        let task: gpui_kit::Task<Value> = match kind {
            "hover" => {
                let request = link.hover(&text, offset, window, cx);
                cx.background_spawn(async move {
                    let Ok(Some(hover)) = request.await else {
                        return Value::Null;
                    };
                    let texts: Vec<String> = match hover.contents {
                        lsp_types::HoverContents::Markup(m) => vec![m.value],
                        lsp_types::HoverContents::Scalar(s) => vec![marked(s)],
                        lsp_types::HoverContents::Array(a) => a.into_iter().map(marked).collect(),
                    };
                    json!(texts)
                })
            }
            "completion" => {
                let context = lsp_types::CompletionContext {
                    trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                    trigger_character: None,
                };
                let request = link.completions(&text, offset, context, window, cx);
                cx.background_spawn(async move {
                    match request.await {
                        Ok(lsp_types::CompletionResponse::Array(items)) => {
                            json!(items.into_iter().map(|i| i.label).collect::<Vec<_>>())
                        }
                        Ok(lsp_types::CompletionResponse::List(list)) => {
                            json!(list.items.into_iter().map(|i| i.label).collect::<Vec<_>>())
                        }
                        Err(_) => Value::Null,
                    }
                })
            }
            "definition" => {
                let request = link.definitions(&text, offset, window, cx);
                cx.background_spawn(async move {
                    match request
                        .await
                        .ok()
                        .and_then(|links| links.into_iter().next())
                    {
                        Some(found) => json!({
                            "uri": found.target_uri.to_string(),
                            "line": found.target_selection_range.start.line + 1,
                        }),
                        None => Value::Null,
                    }
                })
            }
            "rename" => {
                let hooks = self.hooks.clone()?;
                let new_name = new_name?;
                let request = link.rename(&text, offset, new_name.clone(), cx);
                window.spawn(cx, async move |cx| match request.await {
                    Ok(Some(edit)) => {
                        let pending = crate::workspace_edits::PendingEdit {
                            edit: impulse_lsp::workspace_edit::WorkspaceEdit::from(&edit),
                            utf32: link.client.uses_utf32(),
                            origin: link.path.clone(),
                            server_root: link.client.root.clone(),
                            title: format!("Rename to {new_name}"),
                        };
                        let _ = cx.update(|window, cx| (hooks.apply_edit)(pending, window, cx));
                        Value::Bool(true)
                    }
                    Ok(None) => Value::Null,
                    Err(error) => json!({ "error": error }),
                })
            }
            _ => return None,
        };
        return Some(task);

        fn marked(s: lsp_types::MarkedString) -> String {
            match s {
                lsp_types::MarkedString::String(s) => s,
                lsp_types::MarkedString::LanguageString(l) => l.value,
            }
        }
    }

    /// The buffer's text as the file holds it: with its line ending.
    pub fn disk_text(&self, cx: &gpui_kit::App) -> String {
        self.line_ending.apply(&self.text(cx))
    }

    /// Put `text` back as one undoable edit, if the buffer still reads
    /// `expected` (an Undo of an edit across files): whether it did.
    pub fn restore_text(
        &mut self,
        expected: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.state.read(cx).value().to_string();
        if current != expected {
            return false;
        }
        let (range, replacement) = impulse_lsp::edits::changed_span(&current, text);
        let utf16 = |byte: usize| current[..byte].encode_utf16().count();
        self.state.update(cx, |state, cx| {
            state.replace_text_in_range(
                Some(utf16(range.start)..utf16(range.end)),
                replacement,
                window,
                cx,
            );
        });
        true
    }

    /// An empty editor for a file not saved yet; Save asks where it goes.
    pub fn untitled(dir: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let opened = Opened {
            contents: text_file::Contents {
                text: String::new(),
                bom: false,
            },
            stamp: None,
            large: false,
        };
        let mut tab = Self::new(Path::new("Untitled"), opened, window, cx);
        tab.path = PathBuf::new();
        tab.untitled = Some(dir.to_path_buf());
        tab
    }

    /// The servers hear of the save, with the text as it now is.
    pub fn tell_saved(&mut self, cx: &mut Context<Self>) {
        let text = self.state.read(cx).text().clone();
        for (_, link) in &self.links {
            link.sync(&text);
            link.saved();
        }
    }

    /// The file changed on disk since it was loaded or saved here (by an
    /// agent, git or another app).
    pub fn changed_on_disk(&self) -> bool {
        self.stamp.is_some() && text_file::stamp(&self.path) != self.stamp
    }

    /// Write the buffer to its file: the text written, or `None` when it
    /// couldn't be (the notice says why).
    pub fn write(&mut self, cx: &mut Context<Self>) -> Option<String> {
        self.write_and_tell(true, cx)
    }

    /// `write`, telling the language servers (didSave) now or, with
    /// `tell` false, later with `tell_saved` (after the formatter).
    pub fn write_and_tell(&mut self, tell: bool, cx: &mut Context<Self>) -> Option<String> {
        if self.read_only || self.untitled.is_some() {
            return None;
        }
        let text = self.disk_text(cx);
        let written = match text_file::write(&text, self.bom, &self.path) {
            Ok(()) => {
                if tell {
                    self.tell_saved(cx);
                }
                self.dirty = false;
                self.notice = None;
                self.stamp = text_file::stamp(&self.path);
                self.refresh_git(cx);
                Some(text)
            }
            Err(error) => {
                self.notice = Some(format!("Couldn't save {}: {error}", self.title));
                None
            }
        };
        cx.emit(EditorTabChanged);
        cx.notify();
        written
    }

    /// A formatter or command on save is about to rewrite the file: its
    /// result is `adopt_rewrite`'s to take or leave, not a reload's.
    pub fn begin_rewrite(&mut self) {
        self.rewriting += 1;
    }

    /// A formatter or command on save finished: show what it left in the
    /// file, unless it failed, emptied it, or typing has moved on since
    /// `saved` was written (`save_command::rewrite_outcome`).
    pub fn adopt_rewrite(
        &mut self,
        saved: &str,
        succeeded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rewriting = self.rewriting.saturating_sub(1);
        let Some(disk) = text_file::read(&self.path) else {
            return;
        };
        let buffer = self.disk_text(cx);
        match impulse_kit::save_command::rewrite_outcome(saved, &disk.text, &buffer, succeeded) {
            RewriteOutcome::Adopt => {
                self.line_ending = LineEnding::detect(&disk.text);
                self.replace_buffer(&LineEnding::normalize(&disk.text), window, cx);
                self.dirty = false;
                self.stamp = text_file::stamp(&self.path);
            }
            RewriteOutcome::Record { dirty } => {
                self.stamp = text_file::stamp(&self.path);
                self.dirty = dirty;
            }
            // The next save asks before replacing what's there.
            RewriteOutcome::KeepSavedState => self.dirty = true,
        }
        cx.emit(EditorTabChanged);
        cx.notify();
    }

    /// The file changed on disk (the watcher saw it): a clean buffer shows
    /// it; one with unsaved edits keeps them and says so.
    pub fn disk_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.untitled.is_some() || self.rewriting > 0 || !self.changed_on_disk() {
            return;
        }
        if self.dirty || self.read_only {
            if self.read_only
                && text_file::stamp(&self.path)
                    .is_some_and(|s| s.size > text_file::MAX_EDITOR_FILE_SIZE)
            {
                return self.reload_from_disk(window, cx);
            }
            let notice = format!("{} changed on disk.", self.title);
            if self.notice.as_deref() != Some(notice.as_str()) {
                self.notice = Some(notice);
                // The window says so with Reload (Swift's toast).
                cx.emit(crate::editor_git::EditorGitAction {
                    action: "disk-changed".into(),
                    line: 0,
                });
                cx.emit(EditorTabChanged);
                cx.notify();
            }
            return;
        }
        self.reload_from_disk(window, cx);
    }

    /// Take what's on disk, discarding unsaved edits (Reload from Disk).
    pub fn take_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dirty = false;
        self.notice = None;
        self.reload_from_disk(window, cx);
    }

    /// Replace a byte range of the buffer as one undoable edit.
    pub(crate) fn replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.text(cx);
        let (start, end) = (range.start.min(current.len()), range.end.min(current.len()));
        let utf16 = |byte: usize| current[..byte].encode_utf16().count();
        self.state.update(cx, |state, cx| {
            state.replace_text_in_range(Some(utf16(start)..utf16(end)), text, window, cx);
        });
    }

    /// Replace the buffer with `text` as one undoable edit that isn't the
    /// user's.
    fn replace_buffer(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let old = state.read(cx).value().to_string();
        let (range, replacement) = impulse_lsp::edits::changed_span(&old, text);
        if range.is_empty() && replacement.is_empty() {
            return;
        }
        self.reloading = true;
        let utf16 = |byte: usize| old[..byte].encode_utf16().count();
        state.update(cx, |state, cx| {
            state.replace_text_in_range(
                Some(utf16(range.start)..utf16(range.end)),
                replacement,
                window,
                cx,
            );
        });
    }
}

impl Drop for EditorTab {
    fn drop(&mut self) {
        for (_, link) in &self.links {
            link.closed();
        }
    }
}

/// Monaco keeps room for five-digit line numbers where GPUI Kit's editor
/// keeps three, growing with the file: the difference, so the numbers and
/// the code sit where the Swift app's do. Plus the 2px the extra-small
/// size takes off GPUI Kit's left padding (4px where medium has 6).
pub(crate) fn gutter_inset(
    state: &Entity<EditorState>,
    font_size: f32,
    cx: &App,
) -> gpui_kit::Pixels {
    let digits = state
        .read(cx)
        .text()
        .lines_len()
        .max(1)
        .to_string()
        .len()
        .max(3);
    px((font_size * 6. / 7. - font_size * 0.6 * (digits - 3) as f32).max(0.) + 2.)
}

impl Render for EditorTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_inline_text(window, cx);
        self.sync_git_zones(cx);
        self.sync_diff_zones(window, cx);
        self.apply_pending_center(window, cx);
        self.track_text_focus(window, cx);
        let signature = self.signature_popup(cx);
        let rename = self.rename_box(cx);
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let (family, size) = (
            crate::fonts::family(&settings.font_family),
            settings.font_size as f32,
        );
        let line_height = settings.editor_line_height;
        let mut editor = Editor::new(&self.state)
            .size_full()
            // No inset above line 1, as Monaco has none (GPUI Kit's medium
            // size pads 8px above and below).
            .xsmall()
            .font_family(family)
            .text_size(px(size))
            .pl(gutter_inset(&self.state, size, cx))
            .bordered(false);
        // Set either way: the chrome around it turns ligatures off.
        editor = editor.font_features(if settings.font_ligatures {
            gpui_kit::FontFeatures::default()
        } else {
            gpui_kit::FontFeatures::disable_ligatures()
        });
        if line_height > 0 {
            editor = editor.line_height(px(line_height as f32));
        }
        // Room between the text and the scrollbar for the minimap.
        if let Some(gutter) = self.minimap_gutter(cx) {
            editor = editor.scrollbar_gutter(gutter);
        }
        editor = editor.context_menu(Self::context_menu(
            self.primary_link().is_some(),
            !self.read_only,
            self.has_conflicts(),
        ));
        let editor = crate::editor_vim::wrap(self.vim.as_ref(), editor, cx);
        let editor = self.wrap_git(editor, cx);
        // The minimap over the editor's right edge, beside the scrollbar.
        let editor = match self.minimap_overlay(cx) {
            Some(minimap) => gpui_kit::div()
                .relative()
                .size_full()
                .child(editor)
                .child(minimap)
                .into_any_element(),
            None => editor,
        };
        let editor = match self.peek_element(cx) {
            Some(peek) => gpui_kit::div()
                .size_full()
                .flex()
                .flex_col()
                .child(gpui_kit::div().flex_1().min_h_0().child(editor))
                .child(peek)
                .into_any_element(),
            None => editor,
        };
        // The find bar at the top, as the terminal's sits over its grid:
        // above the editor element, so the marks, rulers, sticky lines
        // and their clicks (`wrap_git`, over the editor) stay under it.
        let editor = match self.find_element(cx) {
            Some(find) => gpui_kit::div()
                .size_full()
                .flex()
                .flex_col()
                .child(find)
                .child(gpui_kit::div().flex_1().min_h_0().child(editor))
                .into_any_element(),
            None => editor,
        };
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
            .on_action(cx.listener(Self::change_all_occurrences))
            .on_action(cx.listener(Self::toggle_block_comment))
            .on_action(cx.listener(Self::go_to_bracket))
            .on_action(cx.listener(Self::next_problem))
            .on_action(cx.listener(Self::previous_problem))
            .on_action(cx.listener(Self::expand_selection))
            .on_action(cx.listener(Self::shrink_selection))
            .on_action(cx.listener(Self::show_parameter_hints))
            .on_action(cx.listener(Self::cursor_undo))
            .on_action(cx.listener(Self::cursors_at_line_ends))
            .on_action(cx.listener(Self::trim_trailing_whitespace))
            .on_action(cx.listener(Self::cursor_home))
            .on_action(cx.listener(Self::expand_line_selection))
            .on_action(cx.listener(Self::fold))
            .on_action(cx.listener(Self::unfold))
            .on_action(cx.listener(Self::fold_all))
            .on_action(cx.listener(Self::unfold_all))
            // Find: Impulse's bar. GPUI Kit's Search and Replace come up
            // from the text since it isn't `searchable`.
            .on_action(
                cx.listener(|tab, _: &gpui_kit::component::input::Search, window, cx| {
                    tab.open_find(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|tab, _: &gpui_kit::component::input::Replace, window, cx| {
                    tab.toggle_replace(window, cx)
                }),
            )
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::FindReplace, window, cx| {
                    tab.toggle_replace(window, cx)
                }),
            )
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::FindNext, window, cx| {
                    tab.find_next(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::FindPrevious, window, cx| {
                    tab.find_next(false, window, cx)
                }),
            )
            // Escape the text had no use for (no menu, one cursor) closes
            // the find bar, as in Monaco.
            .on_action(
                cx.listener(|tab, _: &gpui_kit::component::input::Escape, window, cx| {
                    if !tab.find_escape(window, cx) {
                        cx.propagate();
                    }
                }),
            )
            .on_key_down(
                cx.listener(|tab, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && tab.signature.is_some() {
                        tab.hide_signature(cx);
                    } else if event.keystroke.key == "escape" && tab.peek.is_some() {
                        tab.close_peek(window, cx);
                    }
                }),
            )
            .children(signature)
            .children(rename)
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::ShowLineCommit, _, cx| {
                    tab.show_line_commit(cx)
                }),
            )
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::NextConflict, window, cx| {
                    tab.go_to_conflict(1, window, cx)
                }),
            )
            .on_action(cx.listener(
                |tab, _: &crate::editor_keys::PreviousConflict, window, cx| {
                    tab.go_to_conflict(-1, window, cx)
                },
            ))
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::PeekDefinition, window, cx| {
                    tab.find_places(crate::lsp_hub::Navigation::Definition, true, window, cx)
                }),
            )
            .on_action(cx.listener(
                |tab, _: &crate::editor_keys::GoToTypeDefinition, window, cx| {
                    tab.navigate(crate::lsp_hub::Navigation::TypeDefinition, window, cx)
                },
            ))
            .on_action(cx.listener(
                |tab, _: &crate::editor_keys::GoToImplementations, window, cx| {
                    tab.navigate(crate::lsp_hub::Navigation::Implementation, window, cx)
                },
            ))
            .on_action(
                cx.listener(|tab, _: &crate::editor_keys::GoToDeclaration, window, cx| {
                    tab.navigate(crate::lsp_hub::Navigation::Declaration, window, cx)
                }),
            )
            // Files dropped from the file manager or the tree each open in a
            // tab (Swift's editor web view opens them).
            .on_drop(cx.listener(|_, paths: &gpui_kit::ExternalPaths, _, cx| {
                for path in paths.paths().iter().filter(|p| p.is_file()) {
                    cx.emit(crate::editor_git::EditorGitAction {
                        action: format!("open:{}", path.display()),
                        line: 0,
                    });
                }
            }))
            .on_drop(cx.listener(
                |_, dragged: &crate::file_tree_actions::DraggedPath, _, cx| {
                    if dragged.0.is_file() {
                        cx.emit(crate::editor_git::EditorGitAction {
                            action: format!("open:{}", dragged.0.display()),
                            line: 0,
                        });
                    }
                },
            ))
            .map(|d| match (self.preview, &self.diff) {
                (None, Some(_)) => d.child(self.diff_element(editor, cx)),
                (None, None) => d.child(editor),
                (Some(false), _) => d.child(self.preview_element(cx)),
                // The editor on the left half, the preview from 1 pt past
                // the middle, as the Swift tab lays them out (no rule
                // between them). A fixed half: flex shares would give the
                // padded preview more.
                (Some(true), _) => d
                    .flex()
                    .child(
                        gpui_kit::div()
                            .flex_none()
                            .w(gpui_kit::relative(0.5))
                            .min_w_0()
                            .h_full()
                            .child(editor),
                    )
                    .child(gpui_kit::div().flex_none().w(px(1.)).h_full())
                    .child(self.preview_element(cx)),
            })
    }
}

/// How long the cursor rests before occurrences are marked.
const MARK_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

impl EditorTab {
    /// The cursor or selection may have moved: mark the symbol's other
    /// occurrences (from the primary language server, else the same word)
    /// or the selection's other matches, once it rests.
    fn selection_moved(&mut self, cx: &mut Context<Self>) {
        let range = self.state.read(cx).selected_range();
        if self.marked_at.as_ref() == Some(&range) {
            return;
        }
        if let Some(previous) = self.marked_at.replace(range.clone()) {
            self.selections.moved(previous);
        }
        self.find_linked_tag(cx);
        // Monaco's parameter hints follow the cursor: asked again where it
        // now is, and gone once the server has none there.
        if self.signature.is_some() {
            self.request_signature(cx);
        }
        let settings = crate::settings_store::SettingsStore::get(cx);
        let wanted = self.text_focused
            && if range.is_empty() {
                settings.editor_occurrences_highlight
            } else {
                settings.editor_selection_highlight
            };
        // Marks for where the cursor was go as soon as it moves off them.
        let stale = self.marks.as_ref().is_some_and(|marks| {
            !marks
                .get_ranges(cx)
                .iter()
                .any(|r| r.start <= range.start && range.end <= r.end)
        });
        if stale || !wanted {
            if let Some(marks) = &self.marks {
                marks.clear(cx);
            }
        }
        self._marking = None;
        if !wanted {
            return;
        }
        let link = range.is_empty().then(|| self.primary_link()).flatten();
        let text = self.state.read(cx).text().clone();
        self._marking = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(MARK_DELAY).await;
            let from_server = match link {
                Some(link) => {
                    let task = cx.update(|cx| link.document_highlights(&text, range.start, cx));
                    task.await.ok().filter(|found| !found.is_empty())
                }
                None => None,
            };
            let ranges: Vec<std::ops::Range<usize>> = match from_server {
                Some(found) => found.into_iter().map(|(r, _)| r).collect(),
                None => {
                    let source = text.to_string();
                    cx.background_executor()
                        .spawn(async move {
                            use impulse_kit::occurrences;
                            if range.is_empty() {
                                occurrences::word_at(&source, range.start)
                                    .map(|word| {
                                        occurrences::whole_word_matches(&source, &source[word])
                                    })
                                    .unwrap_or_default()
                            } else {
                                occurrences::selection_matches(&source, range)
                            }
                        })
                        .await
                }
            };
            let _ = tab.update(cx, |tab, cx| tab.set_marks(ranges, cx));
        }));
    }

    /// Monaco's word highlighter: marks go when the text loses the
    /// keyboard and are worked out again when it comes back.
    fn track_text_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        use gpui_kit::Focusable as _;
        let focused = self.state.focus_handle(cx).is_focused(window);
        if focused == self.text_focused {
            return;
        }
        self.text_focused = focused;
        self.marked_at = None;
        self.selection_moved(cx);
    }

    fn set_marks(&mut self, ranges: Vec<std::ops::Range<usize>>, cx: &mut Context<Self>) {
        // A lone occurrence is the word itself: nothing to point at.
        let ranges = if ranges.len() < 2 && self.marked_at.as_ref().is_some_and(|r| r.is_empty()) {
            Vec::new()
        } else {
            ranges
        };
        let decorations: Vec<RangeDecoration> = ranges
            .into_iter()
            .map(|range| RangeDecoration::new(range).with_style(RangeDecorationStyle::Fill))
            .collect();
        match &self.marks {
            Some(marks) => marks.set(decorations, cx),
            None => {
                let marks = self.state.update(cx, |state, cx| {
                    state.create_range_decorations_collection(decorations, cx)
                });
                self.marks = Some(marks);
            }
        }
    }
}

impl EditorTab {
    /// Monaco's context menu: navigation, refactoring, the clipboard and
    /// the command palette. Language-server items are off without one.
    fn context_menu(
        lsp: bool,
        editable: bool,
        conflicts: bool,
    ) -> impl Fn(NativeMenu, &mut Window, &mut gpui_kit::App) -> NativeMenu + 'static {
        use crate::editor_keys::{
            ChangeAllOccurrences, GoToDeclaration, GoToImplementations, GoToTypeDefinition,
            PeekDefinition, ShowLineCommit,
        };
        use crate::workbench::{FindReferences, FormatDocument, RenameSymbol};
        use gpui_kit::component::input::{Copy, Cut, GoToDefinition, Paste, ToggleCodeActions};
        move |menu, _, _| {
            let mut menu = menu
                .menu_with_disabled("Go to Definition", !lsp, Box::new(GoToDefinition))
                .menu_with_disabled("Go to Type Definition", !lsp, Box::new(GoToTypeDefinition))
                .menu_with_disabled("Go to Implementations", !lsp, Box::new(GoToImplementations))
                .menu_with_disabled("Go to Declaration", !lsp, Box::new(GoToDeclaration))
                .menu_with_disabled("Go to References", !lsp, Box::new(FindReferences))
                .menu_with_disabled("Peek Definition", !lsp, Box::new(PeekDefinition))
                .separator()
                .menu_with_disabled("Rename Symbol", !(lsp && editable), Box::new(RenameSymbol))
                .menu_with_disabled(
                    "Change All Occurrences",
                    !editable,
                    Box::new(ChangeAllOccurrences),
                )
                .menu_with_disabled(
                    "Format Document",
                    !(lsp && editable),
                    Box::new(FormatDocument),
                )
                .menu_with_disabled(
                    "Show Code Actions",
                    !(lsp && editable),
                    Box::new(ToggleCodeActions),
                )
                .menu("Show Commit for This Line", Box::new(ShowLineCommit));
            if conflicts {
                menu = menu
                    .menu(
                        "Go to Next Merge Conflict",
                        Box::new(crate::editor_keys::NextConflict),
                    )
                    .menu(
                        "Go to Previous Merge Conflict",
                        Box::new(crate::editor_keys::PreviousConflict),
                    );
            }
            menu.separator()
                .menu_with_disabled("Cut", !editable, Box::new(Cut))
                .menu("Copy", Box::new(Copy))
                .menu_with_disabled("Paste", !editable, Box::new(Paste))
                .separator()
                .menu(
                    "Command Palette",
                    Box::new(crate::commands::RunCommand::new("command_palette")),
                )
        }
    }

    /// Change All Occurrences: a cursor on every whole-word occurrence of
    /// the word at the cursor, or every match of the selection.
    fn change_all_occurrences(
        &mut self,
        _: &crate::editor_keys::ChangeAllOccurrences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use impulse_kit::occurrences;
        let (text, selection) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        let (current, others) = if selection.is_empty() {
            let Some(word) = occurrences::word_at(&text, selection.start) else {
                return;
            };
            let all = occurrences::whole_word_matches(&text, &text[word.clone()]);
            (
                word.clone(),
                all.into_iter().filter(|r| *r != word).collect(),
            )
        } else {
            (
                selection.clone(),
                occurrences::selection_matches(&text, selection),
            )
        };
        self.state.update(cx, |state, cx| {
            state.set_selected_range(current, cx);
            for range in others {
                state.add_selection(range, cx);
            }
        });
    }

    /// Go to Type Definition, Implementations or Declaration: straight
    /// there for one place, a list to pick from for several.
    fn navigate(
        &mut self,
        kind: crate::lsp_hub::Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_places(kind, false, window, cx);
    }

    /// Ask the primary server where `kind` leads: one place is gone to
    /// (unless `peek`), several are shown in a peek.
    pub(crate) fn find_places(
        &mut self,
        kind: crate::lsp_hub::Navigation,
        peek: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(link), Some(hooks)) = (self.primary_link(), self.hooks.clone()) else {
            return self.say("No language server for this file.", cx);
        };
        let (text, offset) = {
            let state = self.state.read(cx);
            (state.text().clone(), state.cursor())
        };
        let task = link.navigate(kind, &text, offset, cx);
        let (own_text, own_uri) = (text.to_string(), link.uri.clone());
        use crate::lsp_hub::Navigation;
        let (nothing, title) = match kind {
            Navigation::Definition => ("No definition found.", "Definition"),
            Navigation::TypeDefinition => ("No type definition found.", "Type Definition"),
            Navigation::Implementation => ("No implementation found.", "Implementations"),
            Navigation::Declaration => ("No declaration found.", "Declaration"),
        };
        cx.spawn_in(window, async move |tab, cx| {
            let locations = task.await.unwrap_or_default();
            let _ = tab.update_in(cx, |tab, window, cx| match locations.as_slice() {
                [] => tab.say(nothing, cx),
                _ if peek => {
                    tab.show_peek(title, locations, (&own_uri, &own_text), hooks, window, cx)
                }
                [location] if location.uri == own_uri => {
                    tab.go_to(location.range.start, window, cx);
                }
                [location] => {
                    let params = lsp_types::ShowDocumentParams {
                        uri: location.uri.clone(),
                        external: Some(false),
                        take_focus: Some(true),
                        selection: Some(location.range),
                    };
                    // The workbench reads every editor tab, this one
                    // included.
                    let show = hooks.show_document.clone();
                    window.defer(cx, move |window, cx| {
                        show(&params, window, cx);
                    });
                }
                _ => tab.show_peek(title, locations, (&own_uri, &own_text), hooks, window, cx),
            });
        })
        .detach();
    }
}

impl EditorTab {
    /// After typing: ask for the signature on `(` and `,` (and again while
    /// it shows, so the active parameter follows); `)` closes it.
    fn typed_for_signature(&mut self, cx: &mut Context<Self>) {
        let before = {
            let state = self.state.read(cx);
            let offset = state.cursor();
            let text = state.text();
            // The character before the cursor (whole, however many bytes).
            (offset > 0)
                .then(|| {
                    let start = text.floor_char_boundary(offset - 1);
                    text.slice(start..offset).to_string()
                })
                .and_then(|s| s.chars().last())
        };
        match before {
            Some(')') => return self.hide_signature(cx),
            Some('(' | ',') => {}
            _ if self.signature.is_some() => {}
            _ => return,
        }
        self.request_signature(cx);
    }

    /// Ask the primary server for the signature at the cursor (parameter
    /// hints, ⇧⌘Space, or after typing).
    pub(crate) fn request_signature(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.primary_link() else {
            return;
        };
        let offset = self.state.read(cx).cursor();
        let text = self.state.read(cx).text().clone();
        let task = link.signature_help(&text, offset, cx);
        self._signature_request = Some(cx.spawn(async move |tab, cx| {
            let help = task
                .await
                .ok()
                .flatten()
                .filter(|h| !h.signatures.is_empty());
            let _ = tab.update(cx, |tab, cx| {
                tab.signature = help;
                cx.notify();
            });
        }));
    }

    fn hide_signature(&mut self, cx: &mut Context<Self>) {
        self._signature_request = None;
        if self.signature.take().is_some() {
            cx.notify();
        }
    }

    /// Take Rename Symbol's box away and give the editor the keyboard back.
    fn close_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// Monaco's rename box: the name in a field under the symbol, and how
    /// to answer it.
    fn rename_box(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use gpui_kit::component::input::Input;
        use gpui_kit::component::ActiveTheme as _;
        let rename = self.rename.as_ref()?;
        let at = self
            .state
            .read(cx)
            .range_to_bounds(&(rename.at..rename.at))?;
        let theme = cx.theme();
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.popover, |b| b.widget);
        let settings = crate::settings_store::SettingsStore::get(cx);
        let font = crate::fonts::family(&settings.font_family);
        let size = px(settings.font_size as f32);
        let field = gpui_kit::div()
            .h(px(26.))
            .px(px(4.))
            .flex()
            .items_center()
            .border_1()
            .border_color(theme.ring)
            .bg(theme.background)
            .child(
                Input::new(&rename.input)
                    .appearance(false)
                    .px_0()
                    .font_family(font)
                    .text_size(size),
            );
        let tab = cx.entity().downgrade();
        let popup = gpui_kit::div()
            .occlude()
            .w(px(260.))
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .bg(widget)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" {
                    let _ = tab.update(cx, |tab, cx| tab.close_rename(window, cx));
                    cx.stop_propagation();
                }
            })
            .child(field)
            .child(
                gpui_kit::div()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child("Enter to Rename"),
            );
        Some(gpui_kit::deferred(
            gpui_kit::anchored()
                .position(gpui_kit::point(at.left(), at.bottom()))
                .anchor(gpui_kit::Anchor::TopLeft)
                .child(popup),
        ))
    }

    /// The signature over the caret: the active parameter in bold, its
    /// documentation and the signature's under it (Markdown where the
    /// server sends it), and which of several. In the editor's font, on
    /// the editor widgets' background, as the rename box.
    fn signature_popup(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use gpui_kit::component::text::{TextView, TextViewStyle};
        use gpui_kit::component::ActiveTheme as _;
        use impulse_lsp::signature::{active_parameter_range, active_signature};
        let help = self.signature.as_ref()?;
        let (index, signature) = active_signature(help)?;
        let theme = cx.theme();
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.popover, |b| b.widget);
        let settings = crate::settings_store::SettingsStore::get(cx);
        let font = gpui_kit::font(crate::fonts::family(&settings.font_family));
        let font_size = px(settings.font_size as f32);
        let label = signature.label.clone();
        let bold = active_parameter_range(help, signature);
        let mut runs = Vec::new();
        let run = |len: usize, weight| gpui_kit::TextRun {
            len,
            font: gpui_kit::Font {
                weight,
                ..font.clone()
            },
            color: theme.popover_foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        match &bold {
            Some(range) => {
                runs.push(run(range.start, gpui_kit::FontWeight::NORMAL));
                runs.push(run(range.len(), gpui_kit::FontWeight::BOLD));
                runs.push(run(label.len() - range.end, gpui_kit::FontWeight::NORMAL));
                runs.retain(|r| r.len > 0);
            }
            None => runs.push(run(label.len(), gpui_kit::FontWeight::NORMAL)),
        }
        // Markdown as GPUI Kit's hover shows it; plain text as it is.
        let documentation = |id: &'static str,
                             doc: &Option<lsp_types::Documentation>|
         -> Option<gpui_kit::AnyElement> {
            let (text, markdown) = match doc.as_ref()? {
                lsp_types::Documentation::String(s) => (s.clone(), false),
                lsp_types::Documentation::MarkupContent(m) => {
                    (m.value.clone(), m.kind == lsp_types::MarkupKind::Markdown)
                }
            };
            if text.trim().is_empty() {
                return None;
            }
            Some(if markdown {
                TextView::markdown(id, text)
                    .style(
                        TextViewStyle::default()
                            .paragraph_gap(gpui_kit::rems(0.5))
                            .code_block(
                                gpui_kit::StyleRefinement::default()
                                    .bg(gpui_kit::transparent_black())
                                    .p_0()
                                    .text_size(px(11.)),
                            ),
                    )
                    .into_any_element()
            } else {
                gpui_kit::div()
                    .whitespace_normal()
                    .child(text)
                    .into_any_element()
            })
        };
        let active = signature
            .active_parameter
            .or(help.active_parameter)
            .and_then(|i| signature.parameters.as_ref()?.get(i as usize));
        let parameter_doc =
            active.and_then(|p| documentation("signature-parameter-doc", &p.documentation));
        let signature_doc = documentation("signature-doc", &signature.documentation);
        let count = help.signatures.len();
        let popup = gpui_kit::div()
            .occlude()
            .max_w(px(560.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(widget)
            .shadow_md()
            .text_size(px(12.))
            .text_color(theme.popover_foreground)
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                gpui_kit::div()
                    .flex()
                    .gap(px(8.))
                    .when(count > 1, |d| {
                        d.child(
                            gpui_kit::div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child(format!("{}/{count}", index + 1)),
                        )
                    })
                    .child(
                        gpui_kit::div()
                            .text_size(font_size)
                            .child(gpui_kit::StyledText::new(label).with_runs(runs)),
                    ),
            )
            .children(parameter_doc)
            .children(signature_doc.map(|doc| {
                gpui_kit::div()
                    .text_color(theme.muted_foreground)
                    .max_h(px(160.))
                    .overflow_hidden()
                    .child(doc)
            }));
        Some(gpui_kit::deferred(OverCaret {
            state: self.state.clone(),
            content: gpui_kit::div().pb(px(4.)).child(popup).into_any_element(),
            shown: false,
        }))
    }
}

/// A popup over the caret's line (Monaco's parameter hints), placed in
/// prepaint from this frame's caret (`prepaint_caret_geometry`, as GPUI
/// Kit places its completion menu with an element it keeps to itself), so
/// it stays on the caret as the file scrolls: above the line, or under it
/// where there's no room; gone while the line is scrolled out of view.
/// Used inside `deferred`, whose prepaint runs after the editor's.
struct OverCaret {
    state: Entity<EditorState>,
    content: gpui_kit::AnyElement,
    shown: bool,
}

impl IntoElement for OverCaret {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui_kit::Element for OverCaret {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui_kit::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        // Laid out in prepaint, once the caret is known.
        (
            window.request_layout(gpui_kit::Style::default(), [], cx),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        _: gpui_kit::Bounds<gpui_kit::Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(geometry) = self.state.read(cx).prepaint_caret_geometry() else {
            return;
        };
        // The caret's y leaves the vertical scroll out (its x has it).
        let caret = geometry.cursor_bounds();
        let line_height = geometry.line_height();
        let top = caret.top() - (line_height - caret.size.height) / 2. + geometry.scroll_offset().y;
        let input = geometry.input_bounds();
        if top + line_height <= input.top() || top >= input.bottom() {
            return;
        }
        let viewport = window.viewport_size();
        let size = self.content.layout_as_root(
            gpui_kit::Size {
                width: gpui_kit::AvailableSpace::MaxContent,
                height: gpui_kit::AvailableSpace::MaxContent,
            },
            window,
            cx,
        );
        let x = caret.left().min(viewport.width - size.width).max(px(0.));
        let above = top - size.height;
        let y = if above >= px(0.) {
            above
        } else {
            top + line_height
        };
        self.content.prepaint_at(gpui_kit::point(x, y), window, cx);
        self.shown = true;
    }

    fn paint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        _: gpui_kit::Bounds<gpui_kit::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.shown {
            self.content.paint(window, cx);
        }
    }
}

/// Word-based suggestions (Monaco's `wordBasedSuggestions`) for an editor
/// no language server completes: the document's words that start like
/// the one being typed (`impulse_kit::word_completions`), then other open
/// files' when the setting says so.
struct WordCompletions {
    tab: gpui_kit::WeakEntity<EditorTab>,
    language: String,
}

/// Every editor tab opened, for word suggestions from other files.
#[derive(Default)]
struct OpenDocuments(Vec<gpui_kit::WeakEntity<EditorTab>>);

impl gpui_kit::Global for OpenDocuments {}

/// The files with unsaved edits in any window's editor.
pub(crate) fn unsaved_paths(cx: &gpui_kit::App) -> Vec<PathBuf> {
    cx.try_global::<OpenDocuments>()
        .map(|docs| {
            docs.0
                .iter()
                .filter_map(|tab| tab.upgrade())
                .map(|tab| tab.read(cx))
                .filter(|tab| tab.dirty)
                .map(|tab| tab.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Very large documents aren't scanned on every keystroke.
const MAX_WORD_SCAN: usize = 2 * 1024 * 1024;

impl WordCompletions {
    /// The other open files' text the setting takes words from (ropes,
    /// which share their nodes: made into strings off the main thread).
    fn others(&self, cx: &mut gpui_kit::App) -> Vec<gpui_kit::component::input::Rope> {
        let scope = crate::settings_store::SettingsStore::get(cx)
            .editor_word_based_suggestions
            .clone();
        if scope != "matchingDocuments" && scope != "allDocuments" {
            return Vec::new();
        }
        let own = self.tab.entity_id();
        let docs = cx.default_global::<OpenDocuments>();
        docs.0.retain(|tab| tab.upgrade().is_some());
        let tabs = docs.0.clone();
        tabs.iter()
            .filter(|tab| tab.entity_id() != own)
            .filter_map(|tab| tab.upgrade())
            .filter(|tab| scope == "allDocuments" || tab.read(cx).language == self.language)
            .map(|tab| tab.read(cx).state.read(cx).text().clone())
            .filter(|text| text.len() <= MAX_WORD_SCAN)
            .collect()
    }
}

impl CompletionProvider for WordCompletions {
    fn completions(
        &self,
        text: &gpui_kit::component::input::Rope,
        offset: usize,
        _: lsp_types::CompletionContext,
        _: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> gpui_kit::Task<gpui_kit::Result<lsp_types::CompletionResponse>> {
        if text.len() > MAX_WORD_SCAN {
            return gpui_kit::Task::ready(Ok(lsp_types::CompletionResponse::Array(Vec::new())));
        }
        // The scan runs off the main thread, so typing never waits on it.
        let (text, others) = (text.clone(), self.others(cx));
        cx.background_spawn(async move {
            let others: Vec<String> = others.iter().map(ToString::to_string).collect();
            let others: Vec<&str> = others.iter().map(String::as_str).collect();
            let items =
                impulse_kit::word_completions::suggestions_with(&text.to_string(), offset, &others)
                    .into_iter()
                    .map(|word| lsp_types::CompletionItem {
                        label: word,
                        kind: Some(lsp_types::CompletionItemKind::TEXT),
                        ..Default::default()
                    })
                    .collect();
            Ok(lsp_types::CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut gpui_kit::App) -> bool {
        !new_text.is_empty()
            && new_text
                .chars()
                .all(impulse_kit::word_completions::is_word_char)
    }
}
