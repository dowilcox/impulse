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
    CompletionProvider, DefinitionProvider, HoverProvider, Rope, RopeExt,
};
use gpui_kit::{App, Task, Window};
use impulse_lsp::completion::{filter_completions, typed_prefix};
use impulse_lsp::{path_uri, primary_server, resolve_command, LspClient, ServerEvent};
use lsp_types::request::{Completion, GotoDefinition, HoverRequest};
use lsp_types::{
    CompletionContext, CompletionParams, CompletionResponse, GotoDefinitionParams,
    GotoDefinitionResponse, HoverParams, LocationLink, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri,
};

type ClientTask = Shared<Task<Result<Arc<LspClient>, String>>>;

pub struct LspHub {
    root: PathBuf,
    clients: RefCell<HashMap<&'static str, ClientTask>>,
    events: async_channel::Sender<ServerEvent>,
}

impl LspHub {
    /// Servers run project code (build scripts, proc macros), so a hub only
    /// exists for a folder the user trusts.
    pub fn new(root: PathBuf) -> (Rc<Self>, async_channel::Receiver<ServerEvent>) {
        let (events, receiver) = async_channel::unbounded();
        let hub = Rc::new(Self {
            root,
            clients: RefCell::default(),
            events,
        });
        (hub, receiver)
    }

    /// The client for `language`'s server, started if it isn't running.
    /// `None` when there's no server for it or it isn't installed.
    pub fn client_for(&self, language: &str, cx: &App) -> Option<ClientTask> {
        let spec = primary_server(language)?;
        if let Some(task) = self.clients.borrow().get(spec.name) {
            return Some(task.clone());
        }
        let command = resolve_command(spec.command)?;
        let args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();
        let (root, events) = (self.root.clone(), self.events.clone());
        let task = cx
            .background_executor()
            .spawn(async move { LspClient::start(spec.name, &command, &args, &root, events).await })
            .shared();
        self.clients.borrow_mut().insert(spec.name, task.clone());
        Some(task)
    }
}

/// One open document's link to its server: what it last sent, and the
/// providers GPUI Kit's editor calls.
pub struct DocumentLink {
    pub client: Arc<LspClient>,
    pub uri: Uri,
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

    pub fn saved(&self) {
        self.client.did_save(self.uri.clone());
    }

    pub fn closed(&self) {
        self.client.did_close(self.uri.clone());
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

/// A `Location` as the `LocationLink` GPUI Kit navigates to.
fn link(location: lsp_types::Location) -> LocationLink {
    LocationLink {
        origin_selection_range: None,
        target_uri: location.uri,
        target_range: location.range,
        target_selection_range: location.range,
    }
}
