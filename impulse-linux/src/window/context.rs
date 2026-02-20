use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use libadwaita as adw;

use crate::lsp_completion::LspRequest;
use crate::sidebar;

/// LSP-related shared state that is passed between window module functions.
///
/// All fields are cheaply cloneable (Rc-wrapped) and shared across signal
/// closures within the GTK main loop.
#[derive(Clone)]
pub(crate) struct LspState {
    pub request_tx: Rc<tokio::sync::mpsc::Sender<LspRequest>>,
    pub doc_versions: Rc<RefCell<HashMap<String, i32>>>,
    pub request_seq: Rc<Cell<u64>>,
    pub latest_completion_req: Rc<RefCell<HashMap<String, u64>>>,
    pub latest_hover_req: Rc<RefCell<HashMap<String, u64>>>,
    pub latest_definition_req: Rc<RefCell<HashMap<String, u64>>>,
    pub definition_monaco_ids: Rc<RefCell<HashMap<u64, u64>>>,
    pub error_toast_dedupe: Rc<RefCell<HashSet<String>>>,
}

/// Shared window state passed between window module functions.
///
/// Bundles the commonly-shared references that would otherwise require
/// 10-18 individual function parameters.
#[derive(Clone)]
pub(crate) struct WindowContext {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    pub sidebar_state: Rc<sidebar::SidebarState>,
    pub settings: Rc<RefCell<crate::settings::Settings>>,
    pub lsp: LspState,
    pub toast_overlay: adw::ToastOverlay,
    pub status_bar: Rc<RefCell<crate::status_bar::StatusBar>>,
}
