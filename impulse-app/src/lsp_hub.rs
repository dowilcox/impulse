//! Language servers for one workspace: started on first use (one client per
//! server), only in a trusted folder, and adapted to GPUI Kit's editor
//! providers. The protocol itself is `impulse-lsp`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::future::{FutureExt, Shared};
use gpui_kit::component::input::{
    CodeActionProvider, CompletionProvider, DefinitionProvider, EditorState, HoverProvider, Rope,
    RopeExt,
};
use gpui_kit::{App, Entity, EntityInputHandler, SharedString, Task, Window};
use impulse_kit::trust::WorkspaceTrust;
use impulse_lsp::completion::{filter_completions, typed_prefix};
use impulse_lsp::config::{
    default_init_options, detect_project_root, global_config_path, project_config_folder,
};
use impulse_lsp::{path_uri, resolve_command, LspClient, LspConfig, ServerEvent};
use lsp_types::request::{
    CodeActionRequest, CodeActionResolveRequest, Completion, ExecuteCommand, Formatting,
    GotoDefinition, HoverRequest,
};
use lsp_types::{
    CompletionContext, CompletionParams, CompletionResponse, GotoDefinitionParams,
    GotoDefinitionResponse, HoverParams, LocationLink, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri,
};

type ClientTask = Shared<Task<Result<Arc<LspClient>, String>>>;

pub struct LspHub {
    /// The workspace folder: the root when a file has no project root.
    root: PathBuf,
    trust: Arc<WorkspaceTrust>,
    /// Started servers, by server id and the folder they work in.
    clients: RefCell<HashMap<(String, PathBuf), ClientTask>>,
    events: async_channel::Sender<ServerEvent>,
}

/// A server for a document: its id and its (starting) client.
pub struct ServerFor {
    pub id: String,
    pub client: ClientTask,
}

impl LspHub {
    /// Servers run project code (build scripts, proc macros), so they only
    /// start for files in a trusted folder.
    pub fn new(
        root: PathBuf,
        trust: Arc<WorkspaceTrust>,
    ) -> (Rc<Self>, async_channel::Receiver<ServerEvent>) {
        let (events, receiver) = async_channel::unbounded();
        let hub = Rc::new(Self {
            root,
            trust,
            clients: RefCell::default(),
            events,
        });
        (hub, receiver)
    }

    /// The servers for `path` in `language`, started if they aren't
    /// running: those its project's `lsp.json` (or the global one, or the
    /// defaults) name and that are installed, each in the file's project
    /// root, which is never above the trusted folder. Empty outside trusted
    /// folders.
    pub fn servers_for(&self, path: &Path, language: &str, cx: &App) -> Vec<ServerFor> {
        let Some(ceiling) = self.trust.trusting_folder(&path.to_string_lossy()) else {
            return Vec::new();
        };
        let config = LspConfig::load(
            global_config_path().as_deref(),
            path.parent().and_then(project_config_folder).as_deref(),
        );
        let root = detect_project_root(path, &config.root_markers, Some(&ceiling))
            .unwrap_or_else(|| self.root.clone());
        config
            .servers_for(language)
            .into_iter()
            .filter_map(|(id, server)| {
                let key = (id.clone(), root.clone());
                if let Some(task) = self.clients.borrow().get(&key) {
                    return Some(ServerFor {
                        id,
                        client: task.clone(),
                    });
                }
                let command = resolve_command(&server.command)?;
                let options = server
                    .initialization_options
                    .clone()
                    .or_else(|| default_init_options(&id, None));
                let (name, root, events) = (id.clone(), root.clone(), self.events.clone());
                let task = cx
                    .background_executor()
                    .spawn(async move {
                        LspClient::start(&name, &command, &server.args, options, &root, events)
                            .await
                    })
                    .shared();
                self.clients.borrow_mut().insert(key, task.clone());
                Some(ServerFor { id, client: task })
            })
            .collect()
    }
}

/// One open document's link to its server: what it last sent, and the
/// providers GPUI Kit's editor calls.
/// What Go to Type Definition, Implementations and Declaration ask for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    TypeDefinition,
    Implementation,
    Declaration,
}

pub struct DocumentLink {
    pub client: Arc<LspClient>,
    pub uri: Uri,
    pub path: PathBuf,
    /// The workbench's handler for edits that reach other files.
    pub apply_edit: RefCell<Option<crate::editor_tab::ApplyEdit>>,
    version: RefCell<i32>,
    sent: RefCell<Rope>,
}

impl DocumentLink {
    pub fn open(
        client: Arc<LspClient>,
        path: &Path,
        language: &str,
        text: &Rope,
    ) -> Option<Rc<Self>> {
        let uri = path_uri(path)?;
        client.did_open(uri.clone(), language, 1, text.to_string());
        Some(Rc::new(Self {
            client,
            uri,
            path: path.to_path_buf(),
            apply_edit: RefCell::default(),
            version: RefCell::new(1),
            sent: RefCell::new(text.clone()),
        }))
    }

    /// Send the text if it changed since it was last sent, so a request is
    /// never answered against old text.
    pub fn sync(&self, text: &Rope) {
        if *self.sent.borrow() == *text {
            return;
        }
        let version = {
            let mut version = self.version.borrow_mut();
            *version += 1;
            *version
        };
        log::debug!("didChange v{version} {}", self.uri.as_str());
        self.client
            .did_change(self.uri.clone(), version, text.to_string());
        *self.sent.borrow_mut() = text.clone();
    }

    pub fn version(&self) -> i32 {
        *self.version.borrow()
    }

    pub fn saved(&self) {
        self.client.did_save(self.uri.clone());
    }

    pub fn closed(&self) {
        self.client.did_close(self.uri.clone());
    }

    /// The server's edits to the text, then formatting the document.
    pub fn format(
        &self,
        text: &Rope,
        tab_size: usize,
        insert_spaces: bool,
        cx: &App,
    ) -> Task<Result<Vec<lsp_types::TextEdit>, String>> {
        self.sync(text);
        let params = lsp_types::DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: self.uri.clone(),
            },
            options: lsp_types::FormattingOptions {
                tab_size: tab_size as u32,
                insert_spaces,
                trim_trailing_whitespace: Some(true),
                insert_final_newline: Some(true),
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            Ok(client
                .request::<Formatting>(params)
                .await?
                .unwrap_or_default())
        })
    }

    /// Where the symbol at `offset` is and what it's called, if it can be
    /// renamed (`prepareRename`, when the server offers it).
    pub fn prepare_rename(
        &self,
        text: &Rope,
        offset: usize,
        cx: &App,
    ) -> Task<Result<Option<lsp_types::PrepareRenameResponse>, String>> {
        let params = self.at(text, offset);
        let client = self.client.clone();
        let prepares = matches!(
            client.capabilities().rename_provider,
            Some(lsp_types::OneOf::Right(lsp_types::RenameOptions {
                prepare_provider: Some(true),
                ..
            }))
        );
        cx.background_executor().spawn(async move {
            if !prepares {
                return Ok(None);
            }
            client
                .request::<lsp_types::request::PrepareRenameRequest>(params)
                .await
        })
    }

    pub fn rename(
        &self,
        text: &Rope,
        offset: usize,
        new_name: String,
        cx: &App,
    ) -> Task<Result<Option<lsp_types::WorkspaceEdit>, String>> {
        let params = lsp_types::RenameParams {
            text_document_position: self.at(text, offset),
            new_name,
            work_done_progress_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor()
            .spawn(async move { client.request::<lsp_types::request::Rename>(params).await })
    }

    pub fn references(
        &self,
        text: &Rope,
        offset: usize,
        cx: &App,
    ) -> Task<Result<Vec<lsp_types::Location>, String>> {
        let params = lsp_types::ReferenceParams {
            text_document_position: self.at(text, offset),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            Ok(client
                .request::<lsp_types::request::References>(params)
                .await?
                .unwrap_or_default())
        })
    }

    /// Where the symbol at `offset` has its type defined, is implemented or
    /// is declared (Monaco's Go to Type Definition, Implementations and
    /// Declaration).
    pub fn navigate(
        &self,
        kind: Navigation,
        text: &Rope,
        offset: usize,
        cx: &App,
    ) -> Task<Result<Vec<lsp_types::Location>, String>> {
        use lsp_types::request::{GotoDeclaration, GotoImplementation, GotoTypeDefinition};
        let params = GotoDefinitionParams {
            text_document_position_params: self.at(text, offset),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            let response = match kind {
                Navigation::TypeDefinition => client.request::<GotoTypeDefinition>(params).await?,
                Navigation::Implementation => client.request::<GotoImplementation>(params).await?,
                Navigation::Declaration => client.request::<GotoDeclaration>(params).await?,
            };
            Ok(match response {
                Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
                Some(GotoDefinitionResponse::Array(locations)) => locations,
                Some(GotoDefinitionResponse::Link(links)) => links
                    .into_iter()
                    .map(|l| lsp_types::Location::new(l.target_uri, l.target_selection_range))
                    .collect(),
                None => Vec::new(),
            })
        })
    }

    /// The signature of the call the cursor is in (Monaco's parameter hints).
    pub fn signature_help(
        &self,
        text: &Rope,
        offset: usize,
        cx: &App,
    ) -> Task<Result<Option<lsp_types::SignatureHelp>, String>> {
        let params = lsp_types::SignatureHelpParams {
            context: None,
            text_document_position_params: self.at(text, offset),
            work_done_progress_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            client
                .request::<lsp_types::request::SignatureHelpRequest>(params)
                .await
        })
    }

    /// The other places the symbol at `offset` appears in this file, as
    /// byte ranges, and whether each is written there (Monaco's document
    /// highlights).
    pub fn document_highlights(
        &self,
        text: &Rope,
        offset: usize,
        cx: &App,
    ) -> Task<Result<Vec<(std::ops::Range<usize>, bool)>, String>> {
        let params = lsp_types::DocumentHighlightParams {
            text_document_position_params: self.at(text, offset),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        let source = text.to_string();
        cx.background_executor().spawn(async move {
            let highlights = client
                .request::<lsp_types::request::DocumentHighlightRequest>(params)
                .await?
                .unwrap_or_default();
            let utf32 = client.uses_utf32();
            let offset = |p| impulse_lsp::edits::offset_of(&source, p, utf32);
            Ok(highlights
                .into_iter()
                .map(|h| {
                    let write = h.kind == Some(lsp_types::DocumentHighlightKind::WRITE);
                    (offset(h.range.start)..offset(h.range.end), write)
                })
                .filter(|(range, _)| range.start < range.end)
                .collect())
        })
    }

    /// The file's symbols in document order (the palette's @ mode).
    pub fn document_symbols(
        &self,
        text: &Rope,
        cx: &App,
    ) -> Task<Result<Vec<impulse_kit::document_symbols::OutlineSymbol>, String>> {
        self.sync(text);
        let params = lsp_types::DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: self.uri.clone(),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            let response = client
                .request::<lsp_types::request::DocumentSymbolRequest>(params)
                .await?;
            let json = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
            Ok(impulse_kit::document_symbols::parse(&json))
        })
    }

    /// The project's symbols matching `query`, each with its file (the
    /// palette's # mode).
    pub fn workspace_symbols(
        &self,
        query: String,
        cx: &App,
    ) -> Task<Result<Vec<(impulse_kit::document_symbols::OutlineSymbol, String)>, String>> {
        let params = lsp_types::WorkspaceSymbolParams {
            query,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            let response = client
                .request::<lsp_types::request::WorkspaceSymbolRequest>(params)
                .await?;
            let json = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
            Ok(impulse_kit::document_symbols::parse_workspace(&json))
        })
    }

    /// The LSP position of a byte offset, in the server's encoding.
    fn position(&self, text: &Rope, offset: usize) -> Position {
        let chars = text.offset_to_position(offset);
        if self.client.uses_utf32() {
            return chars;
        }
        let line = text.slice_line(chars.line as usize);
        let byte = line.char_to_byte_idx(chars.character as usize);
        Position::new(chars.line, line.byte_to_utf16_idx(byte) as u32)
    }

    fn at(&self, text: &Rope, offset: usize) -> TextDocumentPositionParams {
        self.sync(text);
        TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: self.uri.clone(),
            },
            position: self.position(text, offset),
        }
    }
}

fn failed(error: String) -> anyhow::Error {
    anyhow::anyhow!(error)
}

impl CompletionProvider for DocumentLink {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<CompletionResponse>> {
        let params = CompletionParams {
            text_document_position: self.at(text, offset),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: Some(trigger),
        };
        // What's typed of the word at the cursor narrows the server's list.
        let line_start = text.line_start_offset(text.offset_to_point(offset).row);
        let prefix = typed_prefix(&text.slice(line_start..offset).to_string()).to_string();
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            let items = match client.request::<Completion>(params).await.map_err(failed)? {
                Some(CompletionResponse::Array(items)) => items,
                Some(CompletionResponse::List(list)) => list.items,
                None => Vec::new(),
            };
            Ok(CompletionResponse::Array(filter_completions(
                items, &prefix,
            )))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        let triggers = self
            .client
            .capabilities()
            .completion_provider
            .and_then(|c| c.trigger_characters)
            .unwrap_or_default();
        new_text.chars().all(|c| c.is_alphanumeric() || c == '_')
            || triggers.iter().any(|t| t == new_text)
    }
}

impl HoverProvider for DocumentLink {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<Option<lsp_types::Hover>>> {
        let params = HoverParams {
            text_document_position_params: self.at(text, offset),
            work_done_progress_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor()
            .spawn(async move { client.request::<HoverRequest>(params).await.map_err(failed) })
    }
}

impl DefinitionProvider for DocumentLink {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<Vec<LocationLink>>> {
        let params = GotoDefinitionParams {
            text_document_position_params: self.at(text, offset),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            Ok(
                match client
                    .request::<GotoDefinition>(params)
                    .await
                    .map_err(failed)?
                {
                    Some(GotoDefinitionResponse::Link(links)) => links,
                    Some(GotoDefinitionResponse::Scalar(location)) => vec![link(location)],
                    Some(GotoDefinitionResponse::Array(locations)) => {
                        locations.into_iter().map(link).collect()
                    }
                    None => Vec::new(),
                },
            )
        })
    }
}

impl CodeActionProvider for DocumentLink {
    fn id(&self) -> SharedString {
        self.client.name.clone().into()
    }

    fn code_actions(
        &self,
        state: Entity<EditorState>,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<Vec<lsp_types::CodeAction>>> {
        let text = state.read(cx).text().clone();
        self.sync(&text);
        let params = lsp_types::CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: self.uri.clone(),
            },
            range: lsp_types::Range::new(
                self.position(&text, range.start),
                self.position(&text, range.end),
            ),
            context: lsp_types::CodeActionContext {
                diagnostics: Vec::new(),
                only: None,
                trigger_kind: Some(lsp_types::CodeActionTriggerKind::INVOKED),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let client = self.client.clone();
        cx.background_executor().spawn(async move {
            let actions = client.request::<CodeActionRequest>(params).await;
            log::debug!(
                "code actions: {:?}",
                actions.as_ref().map(|a| a.as_ref().map_or(0, Vec::len))
            );
            let actions = actions.map_err(failed)?;
            Ok(actions
                .unwrap_or_default()
                .into_iter()
                .map(|action| match action {
                    lsp_types::CodeActionOrCommand::CodeAction(action) => action,
                    lsp_types::CodeActionOrCommand::Command(command) => lsp_types::CodeAction {
                        title: command.title.clone(),
                        command: Some(command),
                        ..Default::default()
                    },
                })
                .collect())
        })
    }

    fn perform_code_action(
        &self,
        state: Entity<EditorState>,
        action: lsp_types::CodeAction,
        _push_to_history: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<()>> {
        let client = self.client.clone();
        let uri = self.uri.clone();
        let utf32 = client.uses_utf32();
        let handler = self.apply_edit.borrow().clone();
        let origin = self.path.clone();
        window.spawn(cx, async move |cx| {
            // Servers may leave the edit out until the action is picked.
            let action = if action.edit.is_none() && action.data.is_some() {
                client
                    .request::<CodeActionResolveRequest>(action.clone())
                    .await
                    .unwrap_or(action)
            } else {
                action
            };
            if let Some(edit) = &action.edit {
                match handler {
                    // Across files, asking first when it reaches far.
                    Some(handler) => {
                        let pending = crate::workspace_edits::PendingEdit {
                            edit: impulse_lsp::workspace_edit::WorkspaceEdit::from(edit),
                            utf32,
                            origin,
                            server_root: client.root.clone(),
                            title: action.title.clone(),
                        };
                        cx.update(|window, cx| handler(pending, window, cx))?;
                    }
                    None => {
                        let (here, _) = edits_for(edit, &uri);
                        cx.update(|window, cx| apply_edits(&state, &here, utf32, window, cx))?;
                    }
                }
            }
            if let Some(command) = action.command {
                let params = lsp_types::ExecuteCommandParams {
                    command: command.command,
                    arguments: command.arguments.unwrap_or_default(),
                    work_done_progress_params: Default::default(),
                };
                client
                    .request::<ExecuteCommand>(params)
                    .await
                    .map_err(failed)?;
            }
            Ok(())
        })
    }
}

/// A workspace edit's text edits for `uri`, and how many it has for other
/// files.
fn edits_for(edit: &lsp_types::WorkspaceEdit, uri: &Uri) -> (Vec<lsp_types::TextEdit>, usize) {
    let mut here = Vec::new();
    let mut elsewhere = 0;
    for (target, edits) in edit.changes.iter().flatten() {
        if target == uri {
            here.extend(edits.iter().cloned());
        } else {
            elsewhere += edits.len();
        }
    }
    let document_edits: Vec<&lsp_types::TextDocumentEdit> = match &edit.document_changes {
        Some(lsp_types::DocumentChanges::Edits(edits)) => edits.iter().collect(),
        Some(lsp_types::DocumentChanges::Operations(operations)) => operations
            .iter()
            .filter_map(|op| match op {
                lsp_types::DocumentChangeOperation::Edit(edit) => Some(edit),
                lsp_types::DocumentChangeOperation::Op(_) => {
                    elsewhere += 1;
                    None
                }
            })
            .collect(),
        None => Vec::new(),
    };
    for document in document_edits {
        let edits = document.edits.iter().map(|edit| match edit {
            lsp_types::OneOf::Left(edit) => edit.clone(),
            lsp_types::OneOf::Right(annotated) => annotated.text_edit.clone(),
        });
        if document.text_document.uri == *uri {
            here.extend(edits);
        } else {
            elsewhere += edits.count();
        }
    }
    (here, elsewhere)
}

/// Apply a server's edits to an editor as one change (one undo step), only
/// over the span that changed so the cursor stays put outside it.
pub fn apply_edits(
    state: &Entity<EditorState>,
    edits: &[lsp_types::TextEdit],
    utf32: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if edits.is_empty() {
        return;
    }
    state.update(cx, |state, cx| {
        let old = state.value().to_string();
        let Some(new) = impulse_lsp::edits::apply_text_edits(&old, edits, utf32) else {
            log::warn!("a language server sent overlapping edits; nothing was changed");
            return;
        };
        let (range, replacement) = impulse_lsp::edits::changed_span(&old, &new);
        if range.is_empty() && replacement.is_empty() {
            return;
        }
        let utf16 = |byte: usize| old[..byte].encode_utf16().count();
        state.replace_text_in_range(
            Some(utf16(range.start)..utf16(range.end)),
            replacement,
            window,
            cx,
        );
    });
}

/// A `Location` as the `LocationLink` GPUI Kit navigates to.
fn link(location: lsp_types::Location) -> LocationLink {
    LocationLink {
        origin_selection_range: None,
        target_uri: location.uri,
        target_range: location.range,
        target_selection_range: location.range,
    }
}
