use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use vte4::prelude::*;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::editor;
use crate::lsp_completion::{LspRequest, LspResponse};
use crate::sidebar;
use crate::terminal;
use crate::terminal_container;

use super::{ensure_file_uri, run_guarded_ui, uri_to_file_path, ClosedTab, MAX_CLOSED_TABS};

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
                        }

                        // Find the terminal's page once and update both tree state and title
                        let dir_name = std::path::Path::new(&path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&path);
                        let n = tab_view.n_pages();
                        for i in 0..n {
                            let page = tab_view.nth_page(i);
                            if terminal.is_ancestor(&page.child()) {
                                if !is_active {
                                    sidebar_state.remove_tab_state(&page.child());
                                }
                                page.set_title(dir_name);
                                break;
                            }
                        }
                    }
                });
            });
        }

        // Connect child-exited to close the tab or remove the split pane
        {
            let tab_view = tab_view.clone();
            let term_clone = term.clone();
            let status_bar = status_bar.clone();
            let sidebar_state = sidebar_state.clone();
            let project_search_root = project_search_root.clone();
            term.connect_child_exited(move |_terminal, _status| {
                run_guarded_ui("terminal-child-exited", || {
                    let n = tab_view.n_pages();
                    for i in 0..n {
                        let page = tab_view.nth_page(i);
                        if term_clone.is_ancestor(&page.child()) {
                            let container = page.child();
                            let terminals =
                                crate::terminal_container::collect_terminals(&container);
                            if terminals.len() <= 1 {
                                tab_view.close_page(&page);
                            } else {
                                crate::terminal_container::remove_terminal(&container, &term_clone);
                                // Update sidebar/status bar to the surviving terminal's CWD
                                if let Some(active) =
                                    crate::terminal_container::get_active_terminal(&container)
                                {
                                    if let Some(uri) = active.current_directory_uri() {
                                        let path = uri_to_file_path(&uri.to_string());
                                        status_bar.borrow().update_cwd(&path);
                                        sidebar_state.load_directory(&path);
                                        *project_search_root.borrow_mut() = path.to_string();
                                    }
                                }
                            }
                            break;
                        }
                    }
                });
            });
        }
    })
}

/// Insert a widget into the tab view immediately after the currently selected tab.
/// Falls back to `append()` if no tab is selected.
pub(super) fn insert_after_selected(
    tab_view: &adw::TabView,
    widget: &impl gtk4::prelude::IsA<gtk4::Widget>,
) -> adw::TabPage {
    if let Some(selected) = tab_view.selected_page() {
        let abs_pos = tab_view.page_position(&selected);
        let n_pinned = tab_view.n_pinned_pages();
        if abs_pos < n_pinned {
            // Selected tab is pinned — insert at the first unpinned slot (position 0).
            tab_view.insert(widget, 0)
        } else {
            // Position relative to unpinned pages, +1 to insert after.
            let unpinned_pos = abs_pos - n_pinned + 1;
            tab_view.insert(widget, unpinned_pos)
        }
    } else {
        tab_view.append(widget)
    }
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
        let page = insert_after_selected(&tab_view, &container.widget);
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

fn validate_lsp_response(
    uri: &str,
    request_id: u64,
    version: i32,
    latest_req: &RefCell<std::collections::HashMap<String, u64>>,
    doc_versions: &RefCell<std::collections::HashMap<String, i32>>,
    tab_view: &adw::TabView,
) -> Option<(String, Rc<crate::editor_webview::MonacoEditorHandle>)> {
    let source_path = uri_to_file_path(uri);
    let latest = latest_req.borrow().get(&source_path).copied().unwrap_or(0);
    if latest != request_id {
        return None;
    }
    let current_version = *doc_versions.borrow().get(&source_path).unwrap_or(&0);
    if current_version != version {
        return None;
    }
    if let Some(page) = tab_view.selected_page() {
        let child = page.child();
        if editor::is_editor(&child) && child.widget_name().as_str() == source_path {
            if let Some(handle) = editor::get_handle_for_widget(&child) {
                return Some((source_path, handle));
            }
        }
    }
    None
}

/// Poll LSP responses on the GTK main loop and dispatch them.
pub(super) fn setup_lsp_response_polling(
    ctx: &super::context::WindowContext,
    lsp_gtk_rx: &Rc<RefCell<std::sync::mpsc::Receiver<LspResponse>>>,
    lsp_install_result_rx: &Rc<RefCell<std::sync::mpsc::Receiver<Result<String, String>>>>,
) {
    let tab_view = ctx.tab_view.clone();
    let lsp_gtk_rx = lsp_gtk_rx.clone();
    let doc_versions = ctx.lsp.doc_versions.clone();
    let latest_completion_req = ctx.lsp.latest_completion_req.clone();
    let latest_hover_req = ctx.lsp.latest_hover_req.clone();
    let latest_definition_req = ctx.lsp.latest_definition_req.clone();
    let definition_monaco_ids = ctx.lsp.definition_monaco_ids.clone();
    let latest_formatting_req = ctx.lsp.latest_formatting_req.clone();
    let latest_signature_help_req = ctx.lsp.latest_signature_help_req.clone();
    let latest_references_req = ctx.lsp.latest_references_req.clone();
    let latest_code_action_req = ctx.lsp.latest_code_action_req.clone();
    let latest_rename_req = ctx.lsp.latest_rename_req.clone();
    let toast_overlay = ctx.toast_overlay.clone();
    let lsp_error_toast_dedupe = ctx.lsp.error_toast_dedupe.clone();
    let lsp_install_result_rx = lsp_install_result_rx.clone();
    let editor_tab_pages = ctx.editor_tab_pages.clone();
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
                        if let Some(page) = editor_tab_pages.borrow().get(&file_path) {
                            let child = page.child();
                            if let Some(handle) = editor::get_handle_for_widget(&child) {
                                handle.apply_diagnostics(&diagnostics);
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

                        let monaco_id = definition_monaco_ids.borrow_mut().remove(&request_id);
                        if let Some(monaco_id) = monaco_id {
                            if let Some(handle) = editor::get_handle(&source_path) {
                                handle.send_resolve_definition(
                                    monaco_id,
                                    Some(uri),
                                    Some(line),
                                    Some(character),
                                );
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
                        lsp_error_toast_dedupe.borrow_mut().clear();
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
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_completion_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_completions(request_id, &items);
                        }
                    }
                    LspResponse::HoverResult {
                        request_id,
                        uri,
                        version,
                        contents,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_hover_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            let text = crate::lsp_hover::extract_hover_text(&contents);
                            handle.resolve_hover(request_id, &text);
                        }
                    }
                    LspResponse::FormattingResult {
                        request_id,
                        uri,
                        version,
                        edits,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_formatting_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_formatting(request_id, &edits);
                        }
                    }
                    LspResponse::SignatureHelpResult {
                        request_id,
                        uri,
                        version,
                        signature_help,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_signature_help_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_signature_help(request_id, signature_help.as_ref());
                        }
                    }
                    LspResponse::ReferencesResult {
                        request_id,
                        uri,
                        version,
                        locations,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_references_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_references(request_id, &locations);
                        }
                    }
                    LspResponse::CodeActionResult {
                        request_id,
                        uri,
                        version,
                        actions,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_code_action_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_code_actions(request_id, &actions);
                        }
                    }
                    LspResponse::RenameResult {
                        request_id,
                        uri,
                        version,
                        edits,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_rename_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_rename(request_id, &edits);
                        }
                    }
                    LspResponse::PrepareRenameResult {
                        request_id,
                        uri,
                        version,
                        range,
                        placeholder,
                    } => {
                        if let Some((_path, handle)) = validate_lsp_response(
                            &uri,
                            request_id,
                            version,
                            &latest_rename_req,
                            &doc_versions,
                            &tab_view,
                        ) {
                            handle.resolve_prepare_rename(
                                request_id,
                                range.as_ref(),
                                placeholder.as_deref(),
                            );
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
                        sidebar_state.switch_to_tab(&child, &dir);
                        // Use the sidebar's restored current_path for the status bar,
                        // which preserves the project root rather than the file's parent dir.
                        let cwd = sidebar_state.current_path.borrow().clone();
                        status_bar.borrow().update_cwd(&cwd);
                    }
                    // Refresh git diff decorations (they may be stale after
                    // terminal git operations like commit/stash/checkout).
                    if editor::get_handle_for_widget(&child).is_some() {
                        super::send_diff_decorations(&file_path);
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
                    // Show/hide preview button based on file type
                    if editor::is_previewable_file(&file_path) {
                        let is_previewing = editor::get_handle_for_widget(&child)
                            .map(|h| h.is_previewing.get())
                            .unwrap_or(false);
                        status_bar.borrow().show_preview_button(is_previewing);
                    } else {
                        status_bar.borrow().hide_preview_button();
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
    ctx: &super::context::WindowContext,
    create_tab: &(impl Fn() + Clone + 'static),
    closed_tabs: &Rc<RefCell<std::collections::VecDeque<ClosedTab>>>,
) {
    let window_ref = ctx.window.clone();
    let sidebar_state = ctx.sidebar_state.clone();
    let lsp_tx = ctx.lsp.request_tx.clone();
    let create_tab_on_empty = create_tab.clone();
    let doc_versions_for_close = ctx.lsp.doc_versions.clone();
    let completion_req_for_close = ctx.lsp.latest_completion_req.clone();
    let hover_req_for_close = ctx.lsp.latest_hover_req.clone();
    let definition_req_for_close = ctx.lsp.latest_definition_req.clone();
    let definition_monaco_ids = ctx.lsp.definition_monaco_ids.clone();
    let closed_tabs_for_close = closed_tabs.clone();
    let open_editor_paths = ctx.open_editor_paths.clone();
    let editor_tab_pages = ctx.editor_tab_pages.clone();
    ctx.tab_view.connect_close_page(move |tv, page| {
        // Confirm before closing pinned tabs
        if page.is_pinned() {
            let dialog = adw::AlertDialog::builder()
                .heading("Pinned Tab")
                .body("This tab is pinned. Close anyway?")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("close", "Close");
            dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("close"));
            dialog.set_close_response("cancel");

            let tv = tv.clone();
            let page = page.clone();
            dialog.connect_response(None, move |_dialog, response| {
                if response == "close" {
                    tv.set_page_pinned(&page, false);
                    tv.close_page(&page);
                }
            });
            dialog.present(Some(&window_ref));
            return gtk4::glib::Propagation::Stop;
        }

        sidebar_state.remove_tab_state(&page.child());
        let child = page.child();

        // Record closed tab info for "reopen closed tab" feature.
        // Only editor and image preview tabs can be reopened (terminals cannot).
        if editor::is_editor(&child) {
            let path = child.widget_name().to_string();
            if !path.is_empty() && path != "GtkBox" {
                let mut stack = closed_tabs_for_close.borrow_mut();
                stack.push_back(ClosedTab::Editor(path));
                if stack.len() > MAX_CLOSED_TABS {
                    stack.pop_front();
                }
            }
        } else if editor::is_image_preview(&child) {
            let path = child.widget_name().to_string();
            if !path.is_empty() && path != "GtkBox" {
                let mut stack = closed_tabs_for_close.borrow_mut();
                stack.push_back(ClosedTab::ImagePreview(path));
                if stack.len() > MAX_CLOSED_TABS {
                    stack.pop_front();
                }
            }
        }

        // Clean up LSP tracking state for editor tabs
        if editor::is_editor(&child) {
            let path = child.widget_name().to_string();
            doc_versions_for_close.borrow_mut().remove(&path);
            completion_req_for_close.borrow_mut().remove(&path);
            hover_req_for_close.borrow_mut().remove(&path);
            // Remove definition req and any pending definition_monaco_ids for this file
            if let Some(seq) = definition_req_for_close.borrow_mut().remove(&path) {
                definition_monaco_ids.borrow_mut().remove(&seq);
            }
        }

        // Remove from dedup set and page map
        {
            let path = child.widget_name().to_string();
            if !path.is_empty() && path != "GtkBox" {
                open_editor_paths.borrow_mut().remove(&path);
                editor_tab_pages.borrow_mut().remove(&path);
            }
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
            dialog.add_response("discard", "Don't Save");
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
                        let uri = ensure_file_uri(&path);
                        if let Some(text) = editor::get_editor_text(&child) {
                            if std::fs::write(&path, &text).is_ok() {
                                if let Err(e) =
                                    lsp_tx.try_send(LspRequest::DidSave { uri: uri.clone() })
                                {
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
                        let uri = ensure_file_uri(&path);
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
                uri: ensure_file_uri(&path),
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
