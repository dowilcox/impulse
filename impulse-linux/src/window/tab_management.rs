use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use vte4::prelude::*;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::editor;
use crate::lsp_completion::{LspRequest, LspResponse};
use crate::sidebar;
use crate::terminal;
use crate::terminal_container;

use super::{file_path_to_uri, run_guarded_ui, uri_to_file_path, ClosedTab, MAX_CLOSED_TABS};

/// Create the closure that connects CWD-change and child-exited signals on a terminal.
pub(super) fn make_setup_terminal_signals(
    tab_view: &adw::TabView,
    status_bar: &Rc<RefCell<crate::status_bar::StatusBar>>,
    sidebar_state: &Rc<sidebar::SidebarState>,
) -> Rc<dyn Fn(&vte4::Terminal)> {
    let tab_view = tab_view.clone();
    let status_bar = status_bar.clone();
    let sidebar_state = sidebar_state.clone();
    let project_search_root = sidebar_state.project_search.current_root.clone();
    Rc::new(move |term: &vte4::Terminal| {
        // Connect CWD change signal (OSC 7)
        {
            let status_bar = status_bar.clone();
            let project_search_root = project_search_root.clone();
            let sidebar_state = sidebar_state.clone();
            let tab_view = tab_view.clone();
            term.connect_current_directory_uri_notify(move |terminal| {
                run_guarded_ui("terminal-cwd-notify", || {
                    if let Some(uri) = terminal.current_directory_uri() {
                        let uri_str = uri.to_string();
                        let path = uri_to_file_path(&uri_str);

                        // Only update sidebar/status bar if this terminal is in the active tab
                        let is_active = tab_view
                            .selected_page()
                            .is_some_and(|p| terminal.is_ancestor(&p.child()));
                        if is_active {
                            status_bar.borrow().update_cwd(&path);
                            sidebar_state.load_directory(&path);
                            *project_search_root.borrow_mut() = path.to_string();
                        } else {
                            // Background tab CWD changed: invalidate saved tree state
                            let n = tab_view.n_pages();
                            for i in 0..n {
                                let page = tab_view.nth_page(i);
                                if terminal.is_ancestor(&page.child()) {
                                    sidebar_state.remove_tab_state(&page.child());
                                    break;
                                }
                            }
                        }

                        // Always update tab title to directory name
                        let dir_name = std::path::Path::new(&path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&path);
                        let n = tab_view.n_pages();
                        for i in 0..n {
                            let page = tab_view.nth_page(i);
                            if terminal.is_ancestor(&page.child()) {
                                page.set_title(dir_name);
                                break;
                            }
                        }
                    }
                });
            });
        }

        // Connect child-exited to close the tab
        {
            let tab_view = tab_view.clone();
            let term_clone = term.clone();
            term.connect_child_exited(move |_terminal, _status| {
                run_guarded_ui("terminal-child-exited", || {
                    // Find and close the tab page containing this terminal
                    let n = tab_view.n_pages();
                    for i in 0..n {
                        let page = tab_view.nth_page(i);
                        if term_clone.is_ancestor(&page.child()) {
                            tab_view.close_page(&page);
                            break;
                        }
                    }
                });
            });
        }
    })
}

/// Create the closure that spawns a new terminal tab.
pub(super) fn make_create_tab(
    tab_view: &adw::TabView,
    setup_terminal_signals: &Rc<dyn Fn(&vte4::Terminal)>,
    settings: &Rc<RefCell<crate::settings::Settings>>,
    copy_on_select_flag: &Rc<Cell<bool>>,
    shell_cache: &Rc<terminal::ShellSpawnCache>,
    icon_cache: &Rc<RefCell<crate::file_icons::IconCache>>,
) -> impl Fn() + Clone {
    let tab_view = tab_view.clone();
    let setup_terminal_signals = setup_terminal_signals.clone();
    let settings = settings.clone();
    let copy_on_select_flag = copy_on_select_flag.clone();
    let shell_cache = shell_cache.clone();
    let icon_cache = icon_cache.clone();
    move || {
        let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
        let term =
            terminal::create_terminal(&settings.borrow(), theme, copy_on_select_flag.clone());
        setup_terminal_signals(&term);
        terminal::spawn_shell(&term, &shell_cache);

        let container = terminal_container::TerminalContainer::new(&term);
        let page = tab_view.append(&container.widget);
        page.set_title(shell_cache.shell_name());
        if let Some(texture) = icon_cache.borrow().get_toolbar_icon("console") {
            page.set_icon(Some(texture));
        }
        tab_view.set_selected_page(&page);
        term.grab_focus();
    }
}

/// Set up tab context menu actions (new, close, close-others, pin).
pub(super) fn setup_tab_context_menu(
    window: &adw::ApplicationWindow,
    tab_view: &adw::TabView,
    create_tab: &(impl Fn() + Clone + 'static),
) {
    let menu_page: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));

    // Track which page the context menu was opened on
    {
        let menu_page = menu_page.clone();
        tab_view.connect_setup_menu(move |_tv, page| {
            *menu_page.borrow_mut() = page.cloned();
        });
    }

    let tab_actions = gio::SimpleActionGroup::new();

    // tab.close action
    {
        let action = gio::SimpleAction::new("close", None);
        let tab_view = tab_view.clone();
        let menu_page = menu_page.clone();
        action.connect_activate(move |_, _| {
            if let Some(page) = menu_page.borrow().as_ref() {
                tab_view.close_page(page);
            }
        });
        tab_actions.add_action(&action);
    }

    // tab.close-others action
    {
        let action = gio::SimpleAction::new("close-others", None);
        let tab_view = tab_view.clone();
        let menu_page = menu_page.clone();
        action.connect_activate(move |_, _| {
            if let Some(keep_page) = menu_page.borrow().as_ref() {
                let n = tab_view.n_pages();
                let mut pages_to_close = Vec::new();
                for i in 0..n {
                    let page = tab_view.nth_page(i);
                    if &page != keep_page && !page.is_pinned() {
                        pages_to_close.push(page);
                    }
                }
                for page in pages_to_close {
                    tab_view.close_page(&page);
                }
            }
        });
        tab_actions.add_action(&action);
    }

    // tab.pin action - toggle pin state
    {
        let action = gio::SimpleAction::new("pin", None);
        let tab_view = tab_view.clone();
        let menu_page = menu_page.clone();
        action.connect_activate(move |_, _| {
            if let Some(page) = menu_page.borrow().as_ref() {
                tab_view.set_page_pinned(page, !page.is_pinned());
            }
        });
        tab_actions.add_action(&action);
    }

    // tab.new action
    {
        let action = gio::SimpleAction::new("new", None);
        let create_tab = create_tab.clone();
        action.connect_activate(move |_, _| {
            create_tab();
        });
        tab_actions.add_action(&action);
    }

    window.insert_action_group("tab", Some(&tab_actions));
}

/// Poll LSP responses on the GTK main loop and dispatch them.
pub(super) fn setup_lsp_response_polling(
    tab_view: &adw::TabView,
    sidebar_state: &Rc<sidebar::SidebarState>,
    lsp_gtk_rx: &Rc<RefCell<std::sync::mpsc::Receiver<LspResponse>>>,
    lsp_doc_versions: &Rc<RefCell<HashMap<String, i32>>>,
    latest_completion_req: &Rc<RefCell<HashMap<String, u64>>>,
    latest_hover_req: &Rc<RefCell<HashMap<String, u64>>>,
    latest_definition_req: &Rc<RefCell<HashMap<String, u64>>>,
    toast_overlay: &adw::ToastOverlay,
    lsp_error_toast_dedupe: &Rc<RefCell<HashSet<String>>>,
    lsp_install_result_rx: &Rc<RefCell<std::sync::mpsc::Receiver<Result<String, String>>>>,
) {
    let tab_view = tab_view.clone();
    let sidebar_state = sidebar_state.clone();
    let lsp_gtk_rx = lsp_gtk_rx.clone();
    let doc_versions = lsp_doc_versions.clone();
    let latest_completion_req = latest_completion_req.clone();
    let latest_hover_req = latest_hover_req.clone();
    let latest_definition_req = latest_definition_req.clone();
    let toast_overlay = toast_overlay.clone();
    let lsp_error_toast_dedupe = lsp_error_toast_dedupe.clone();
    let lsp_install_result_rx = lsp_install_result_rx.clone();
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        run_guarded_ui("lsp-gtk-poll", || {
            {
                let install_rx = lsp_install_result_rx.borrow();
                while let Ok(result) = install_rx.try_recv() {
                    let text = match result {
                        Ok(msg) => msg,
                        Err(err) => format!("Failed to install web LSP servers: {}", err),
                    };
                    let toast = adw::Toast::new(&text);
                    toast.set_timeout(5);
                    toast_overlay.add_toast(toast);
                }
            }

            let rx = lsp_gtk_rx.borrow();
            while let Ok(response) = rx.try_recv() {
                match response {
                    LspResponse::Diagnostics {
                        uri,
                        version,
                        diagnostics,
                    } => {
                        let file_path = uri_to_file_path(&uri);
                        if let Some(diag_version) = version {
                            let current_version =
                                *doc_versions.borrow().get(&file_path).unwrap_or(&0);
                            if diag_version < current_version {
                                continue;
                            }
                        }
                        let n = tab_view.n_pages();
                        for i in 0..n {
                            let page = tab_view.nth_page(i);
                            let child = page.child();
                            if child.widget_name().as_str() == file_path {
                                if let Some(handle) = editor::get_handle_for_widget(&child) {
                                    handle.apply_diagnostics(&diagnostics);
                                }
                                break;
                            }
                        }
                    }
                    LspResponse::DefinitionResult {
                        request_id,
                        source_uri,
                        source_version,
                        uri,
                        line,
                        character,
                    } => {
                        let source_path = uri_to_file_path(&source_uri);
                        let latest = latest_definition_req
                            .borrow()
                            .get(&source_path)
                            .copied()
                            .unwrap_or(0);
                        if latest != request_id {
                            continue;
                        }
                        let current_version =
                            *doc_versions.borrow().get(&source_path).unwrap_or(&0);
                        if current_version != source_version {
                            continue;
                        }
                        if let Some(page) = tab_view.selected_page() {
                            if page.child().widget_name().as_str() != source_path {
                                continue;
                            }
                        }

                        let file_path = uri_to_file_path(&uri);
                        let is_same_file = file_path == source_path;

                        if is_same_file {
                            // Same-file navigation: just move the cursor
                            if let Some(page) = tab_view.selected_page() {
                                let child = page.child();
                                editor::go_to_position(&child, line + 1, character + 1);
                            }
                        } else {
                            // Cross-file navigation: open the target file, then jump
                            if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref()
                            {
                                cb(&file_path);
                            }
                            let n = tab_view.n_pages();
                            for i in 0..n {
                                let page = tab_view.nth_page(i);
                                let child = page.child();
                                if child.widget_name().as_str() == file_path {
                                    editor::go_to_position(&child, line + 1, character + 1);
                                    tab_view.set_selected_page(&page);
                                    break;
                                }
                            }
                        }
                    }
                    LspResponse::ServerInitialized {
                        client_key,
                        server_id,
                    } => {
                        log::info!(
                            "LSP server initialized: server_id={}, key={}",
                            server_id,
                            client_key
                        );
                    }
                    LspResponse::ServerError {
                        client_key,
                        server_id,
                        message,
                    } => {
                        log::warn!(
                            "LSP server error for {} (key={}): {}",
                            server_id,
                            client_key,
                            message
                        );

                        let dedupe_key = format!("{}|{}", server_id, message);
                        if lsp_error_toast_dedupe.borrow_mut().insert(dedupe_key) {
                            let toast_message = if message.contains("install-lsp-servers") {
                                format!(
                                "LSP '{}' missing. Open Command Palette and run 'Install Web LSP Servers'.",
                                server_id
                            )
                            } else {
                                format!("LSP '{}' failed to start: {}", server_id, message)
                            };
                            let toast = adw::Toast::new(&toast_message);
                            toast.set_timeout(7);
                            toast_overlay.add_toast(toast);
                        }
                    }
                    LspResponse::ServerExited {
                        client_key,
                        server_id,
                    } => {
                        log::info!(
                            "LSP server exited: server_id={}, key={}",
                            server_id,
                            client_key
                        );
                    }
                    LspResponse::CompletionResult {
                        request_id,
                        uri,
                        version,
                        items,
                    } => {
                        let source_path = uri_to_file_path(&uri);
                        let latest = latest_completion_req
                            .borrow()
                            .get(&source_path)
                            .copied()
                            .unwrap_or(0);
                        if latest != request_id {
                            continue;
                        }
                        let current_version =
                            *doc_versions.borrow().get(&source_path).unwrap_or(&0);
                        if current_version != version {
                            continue;
                        }
                        // Resolve completion into the Monaco editor
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            if editor::is_editor(&child)
                                && child.widget_name().as_str() == source_path
                            {
                                if let Some(handle) = editor::get_handle_for_widget(&child) {
                                    handle.resolve_completions(request_id, &items);
                                }
                            }
                        }
                    }
                    LspResponse::HoverResult {
                        request_id,
                        uri,
                        version,
                        contents,
                    } => {
                        let source_path = uri_to_file_path(&uri);
                        let latest = latest_hover_req
                            .borrow()
                            .get(&source_path)
                            .copied()
                            .unwrap_or(0);
                        if latest != request_id {
                            continue;
                        }
                        let current_version =
                            *doc_versions.borrow().get(&source_path).unwrap_or(&0);
                        if current_version != version {
                            continue;
                        }
                        // Resolve hover into the Monaco editor
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            if editor::is_editor(&child)
                                && child.widget_name().as_str() == source_path
                            {
                                if let Some(handle) = editor::get_handle_for_widget(&child) {
                                    let text = crate::lsp_hover::extract_hover_text(&contents);
                                    handle.resolve_hover(request_id, &text);
                                }
                            }
                        }
                    }
                }
            }
        });
        gtk4::glib::ControlFlow::Continue
    });
}

/// Connect the selected-page-notify signal to focus terminals/editors and
/// update the status bar when switching tabs.
pub(super) fn setup_tab_switch_handler(
    tab_view: &adw::TabView,
    status_bar: &Rc<RefCell<crate::status_bar::StatusBar>>,
    sidebar_state: &Rc<sidebar::SidebarState>,
) {
    let status_bar = status_bar.clone();
    let sidebar_state = sidebar_state.clone();
    tab_view.connect_selected_page_notify(move |tv| {
        run_guarded_ui("tab-selected-page-notify", || {
            if let Some(page) = tv.selected_page() {
                let child = page.child();

                // Always save outgoing tab's tree state before switching
                sidebar_state.save_active_tab_state();

                if let Some(term) = terminal_container::get_active_terminal(&child) {
                    term.grab_focus();
                    status_bar.borrow().hide_editor_info();
                    // Restore saved tree state or load directory for this tab
                    if let Some(uri) = term.current_directory_uri() {
                        let uri_str = uri.to_string();
                        let path = uri_to_file_path(&uri_str);
                        status_bar.borrow().update_cwd(&path);
                        sidebar_state.switch_to_tab(&child, &path);
                    } else {
                        // New terminal without CWD yet -- just set active tab
                        sidebar_state.set_active_tab(&child);
                    }
                } else if editor::is_editor(&child) {
                    // Editor tab: focus the editor and show its parent directory
                    child.grab_focus();
                    let file_path = child.widget_name().to_string();
                    if let Some(parent) = std::path::Path::new(&file_path).parent() {
                        let dir = parent.to_string_lossy().to_string();
                        status_bar.borrow().update_cwd(&dir);
                        sidebar_state.switch_to_tab(&child, &dir);
                    }
                    // Cursor position is updated via CursorMoved events from Monaco
                    // Show language and encoding for editor tabs
                    if let Some(lang) = editor::get_editor_language(&child) {
                        status_bar.borrow().update_language(&lang);
                    } else {
                        status_bar.borrow().update_language("Plain Text");
                    }
                    status_bar.borrow().update_encoding("UTF-8");
                    // Show indent info for editor tabs
                    if let Some(indent) = editor::get_editor_indent_info(&child) {
                        status_bar.borrow().update_indent_info(&indent);
                    }
                } else {
                    child.grab_focus();
                    status_bar.borrow().hide_editor_info();
                }
            }
        });
    });
}

/// Connect the close-page signal to handle unsaved editor changes and
/// open a new terminal when the last tab is closed.
pub(super) fn setup_tab_close_handler(
    tab_view: &adw::TabView,
    window: &adw::ApplicationWindow,
    sidebar_state: &Rc<sidebar::SidebarState>,
    lsp_request_tx: &Rc<tokio::sync::mpsc::Sender<LspRequest>>,
    lsp_doc_versions: &Rc<RefCell<HashMap<String, i32>>>,
    latest_completion_req: &Rc<RefCell<HashMap<String, u64>>>,
    latest_hover_req: &Rc<RefCell<HashMap<String, u64>>>,
    latest_definition_req: &Rc<RefCell<HashMap<String, u64>>>,
    create_tab: &(impl Fn() + Clone + 'static),
    closed_tabs: &Rc<RefCell<Vec<ClosedTab>>>,
) {
    let window_ref = window.clone();
    let sidebar_state = sidebar_state.clone();
    let lsp_tx = lsp_request_tx.clone();
    let create_tab_on_empty = create_tab.clone();
    let doc_versions_for_close = lsp_doc_versions.clone();
    let completion_req_for_close = latest_completion_req.clone();
    let hover_req_for_close = latest_hover_req.clone();
    let definition_req_for_close = latest_definition_req.clone();
    let closed_tabs_for_close = closed_tabs.clone();
    tab_view.connect_close_page(move |tv, page| {
        sidebar_state.remove_tab_state(&page.child());
        let child = page.child();

        // Record closed tab info for "reopen closed tab" feature.
        // Only editor and image preview tabs can be reopened (terminals cannot).
        if editor::is_editor(&child) {
            let path = child.widget_name().to_string();
            if !path.is_empty() && path != "GtkBox" {
                let mut stack = closed_tabs_for_close.borrow_mut();
                stack.push(ClosedTab::Editor(path));
                if stack.len() > MAX_CLOSED_TABS {
                    stack.remove(0);
                }
            }
        } else if editor::is_image_preview(&child) {
            let path = child.widget_name().to_string();
            if !path.is_empty() && path != "GtkBox" {
                let mut stack = closed_tabs_for_close.borrow_mut();
                stack.push(ClosedTab::ImagePreview(path));
                if stack.len() > MAX_CLOSED_TABS {
                    stack.remove(0);
                }
            }
        }

        // Clean up LSP tracking state for editor tabs
        if editor::is_editor(&child) {
            let path = child.widget_name().to_string();
            doc_versions_for_close.borrow_mut().remove(&path);
            completion_req_for_close.borrow_mut().remove(&path);
            hover_req_for_close.borrow_mut().remove(&path);
            definition_req_for_close.borrow_mut().remove(&path);
        }

        // Check if this is an editor tab with unsaved changes
        if editor::is_editor(&child) && editor::is_modified(&child) {
                // Extract filename for the dialog message
                let filename = std::path::Path::new(&child.widget_name().to_string())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("File")
                    .to_string();

                // Show confirmation dialog
                let dialog = adw::AlertDialog::builder()
                    .heading("Unsaved Changes")
                    .body(format!(
                        "\"{}\" has unsaved changes. Close anyway?",
                        filename
                    ))
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("discard", "Discard");
                dialog.add_response("save", "Save & Close");
                dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
                dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                dialog.set_default_response(Some("save"));
                dialog.set_close_response("cancel");

                let tv = tv.clone();
                let page = page.clone();
                let child = child.clone();
                let lsp_tx = lsp_tx.clone();
                let create_tab2 = create_tab_on_empty.clone();
                let create_tab3 = create_tab_on_empty.clone();
                dialog.connect_response(None, move |_dialog, response| {
                    match response {
                        "save" => {
                            // Save then close
                            let path = child.widget_name().to_string();
                            let uri = file_path_to_uri(std::path::Path::new(&path))
                                .unwrap_or_else(|| format!("file://{}", path));
                            if let Some(text) = editor::get_editor_text(&child) {
                                if std::fs::write(&path, &text).is_ok() {
                                    if let Err(e) = lsp_tx.try_send(LspRequest::DidSave { uri: uri.clone() }) {
                                        log::warn!("LSP request channel full, dropping request: {}", e);
                                    }
                                }
                            }
                            editor::unregister_handle(&path);
                            if let Err(e) = lsp_tx.try_send(LspRequest::DidClose { uri }) {
                                log::warn!("LSP request channel full, dropping request: {}", e);
                            }
                            tv.close_page_finish(&page, true);
                            let tv2 = tv.clone();
                            let new_tab = create_tab2.clone();
                            gtk4::glib::idle_add_local_once(move || {
                                if tv2.n_pages() == 0 {
                                    new_tab();
                                }
                            });
                        }
                        "discard" => {
                            let path = child.widget_name().to_string();
                            editor::unregister_handle(&path);
                            let uri = file_path_to_uri(std::path::Path::new(&path))
                                .unwrap_or_else(|| format!("file://{}", path));
                            if let Err(e) = lsp_tx.try_send(LspRequest::DidClose { uri }) {
                                log::warn!("LSP request channel full, dropping request: {}", e);
                            }
                            tv.close_page_finish(&page, true);
                            let tv2 = tv.clone();
                            let new_tab = create_tab3.clone();
                            gtk4::glib::idle_add_local_once(move || {
                                if tv2.n_pages() == 0 {
                                    new_tab();
                                }
                            });
                        }
                        _ => {
                            // Cancel - don't close
                            tv.close_page_finish(&page, false);
                        }
                    }
                });

                dialog.present(Some(&window_ref));

                return gtk4::glib::Propagation::Stop;
        }

        // Terminal tab or unmodified editor: close immediately
        if editor::is_editor(&child) {
            let path = child.widget_name().to_string();
            editor::unregister_handle(&path);
            if let Err(e) = lsp_tx.try_send(LspRequest::DidClose {
                uri: file_path_to_uri(std::path::Path::new(&path))
                    .unwrap_or_else(|| format!("file://{}", path)),
            }) {
                log::warn!("LSP request channel full, dropping request: {}", e);
            }
        }
        tv.close_page_finish(page, true);
        let tv = tv.clone();
        let new_tab = create_tab_on_empty.clone();
        gtk4::glib::idle_add_local_once(move || {
            if tv.n_pages() == 0 {
                new_tab();
            }
        });
        gtk4::glib::Propagation::Stop
    });
}
