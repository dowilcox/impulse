use gtk4::prelude::*;
use libadwaita as adw;
use vte4::prelude::*;

use crate::editor;
use crate::lsp_completion::LspRequest;
use crate::terminal_container;

use super::{file_path_to_uri, language_from_uri, run_commands_on_save, run_guarded_ui, send_diff_decorations, uri_to_file_path};

/// Wire up sidebar file activation, project search result activation,
/// and "Open in Terminal" context menu callbacks.
pub(super) fn wire_sidebar_signals(ctx: &super::context::WindowContext) {
    let sidebar_state = &ctx.sidebar_state;
    let tab_view = &ctx.tab_view;
    let status_bar = &ctx.status_bar;
    let settings = &ctx.settings;
    let lsp_request_tx = &ctx.lsp.request_tx;
    let lsp_doc_versions = &ctx.lsp.doc_versions;
    let lsp_request_seq = &ctx.lsp.request_seq;
    let latest_completion_req = &ctx.lsp.latest_completion_req;
    let latest_hover_req = &ctx.lsp.latest_hover_req;
    let latest_definition_req = &ctx.lsp.latest_definition_req;
    let definition_monaco_ids = &ctx.lsp.definition_monaco_ids;
    let latest_formatting_req = &ctx.lsp.latest_formatting_req;
    let latest_signature_help_req = &ctx.lsp.latest_signature_help_req;
    let latest_references_req = &ctx.lsp.latest_references_req;
    let latest_code_action_req = &ctx.lsp.latest_code_action_req;
    let latest_rename_req = &ctx.lsp.latest_rename_req;
    let toast_overlay = &ctx.toast_overlay;

    // Wire up file activation to open in editor tab
    {
        let tab_view = tab_view.clone();
        let status_bar = status_bar.clone();
        let settings = settings.clone();
        let lsp_tx = lsp_request_tx.clone();
        let doc_versions = lsp_doc_versions.clone();
        let sidebar_state_for_editor = sidebar_state.clone();
        let tree_states = sidebar_state.tab_tree_states.clone();
        let tree_nodes = sidebar_state.tree_nodes.clone();
        let tree_current_path = sidebar_state.current_path.clone();
        let tree_scroll = sidebar_state.file_tree_scroll.clone();
        let lsp_request_seq = lsp_request_seq.clone();
        let latest_completion_req = latest_completion_req.clone();
        let latest_hover_req = latest_hover_req.clone();
        let latest_definition_req = latest_definition_req.clone();
        let definition_monaco_ids = definition_monaco_ids.clone();
        let latest_formatting_req = latest_formatting_req.clone();
        let latest_signature_help_req = latest_signature_help_req.clone();
        let latest_references_req = latest_references_req.clone();
        let latest_code_action_req = latest_code_action_req.clone();
        let latest_rename_req = latest_rename_req.clone();
        let icon_cache = sidebar_state.icon_cache.clone();
        let toast_overlay_for_editor = toast_overlay.clone();
        *sidebar_state.on_file_activated.borrow_mut() = Some(Box::new(move |path: &str| {
            run_guarded_ui("on-file-activated", || {
                // Check if the file is already open in a tab
                let n = tab_view.n_pages();
                for i in 0..n {
                    let page = tab_view.nth_page(i);
                    if page.child().widget_name().as_str() == path {
                        tab_view.set_selected_page(&page);
                        return;
                    }
                }

                let filename = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string();

                if editor::is_image_file(path) {
                    // Open image preview
                    let preview = editor::create_image_preview(path);
                    let page = tab_view.append(&preview);
                    page.set_title(&filename);
                    if let Some(texture) = icon_cache.borrow().get_toolbar_icon("image") {
                        page.set_icon(Some(texture));
                    }
                    // Preserve sidebar tree state for the new tab
                    tree_states.borrow_mut().insert(
                        preview.clone().upcast::<gtk4::Widget>(),
                        crate::sidebar::TabTreeState {
                            nodes: tree_nodes.borrow().clone(),
                            current_path: tree_current_path.borrow().clone(),
                            scroll_position: tree_scroll.vadjustment().value(),
                        },
                    );
                    tab_view.set_selected_page(&page);
                } else if !editor::is_binary_file(path) {
                    // Open file in new editor tab
                    let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
                    let (editor_widget, _handle) = editor::create_editor(
                        path,
                        &settings.borrow(),
                        theme,
                        {
                            let lsp_tx = lsp_tx.clone();
                            let doc_versions = doc_versions.clone();
                            let status_bar = status_bar.clone();
                            let tab_view = tab_view.clone();
                            let settings = settings.clone();
                            let lsp_request_seq = lsp_request_seq.clone();
                            let latest_completion_req = latest_completion_req.clone();
                            let latest_hover_req = latest_hover_req.clone();
                            let latest_definition_req = latest_definition_req.clone();
                            let definition_monaco_ids = definition_monaco_ids.clone();
                            let latest_formatting_req = latest_formatting_req.clone();
                            let latest_signature_help_req = latest_signature_help_req.clone();
                            let latest_references_req = latest_references_req.clone();
                            let latest_code_action_req = latest_code_action_req.clone();
                            let latest_rename_req = latest_rename_req.clone();
                            let sidebar_state = sidebar_state_for_editor.clone();
                            let toast_overlay = toast_overlay_for_editor.clone();
                            let path = path.to_string();
                            move |handle, event| {
                                match event {
                                    impulse_editor::protocol::EditorEvent::Ready => {
                                        // No-op: initialization now happens on FileOpened
                                    }
                                    impulse_editor::protocol::EditorEvent::FileOpened => {
                                        // Flush any pending go-to-position from cross-file navigation.
                                        handle.flush_pending_position();
                                        // Send LSP didOpen
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let language_id = language_from_uri(&uri);
                                        let content = handle.get_content();
                                        let mut versions = doc_versions.borrow_mut();
                                        let version = versions.entry(path.clone()).or_insert(0);
                                        *version += 1;
                                        if let Err(e) = lsp_tx.try_send(LspRequest::DidOpen {
                                            uri,
                                            language_id,
                                            version: *version,
                                            text: content,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                        // Send initial diff decorations
                                        send_diff_decorations(handle, &path);
                                    }
                                    impulse_editor::protocol::EditorEvent::ContentChanged { content, version: _ } => {
                                        // Update tab title based on modified state
                                        let n = tab_view.n_pages();
                                        for i in 0..n {
                                            let page = tab_view.nth_page(i);
                                            if page.child().widget_name().as_str() == path {
                                                let filename = std::path::Path::new(&path)
                                                    .file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or(&path);
                                                if handle.is_modified.get() {
                                                    page.set_title(&format!("\u{25CF} {}", filename));
                                                } else {
                                                    page.set_title(filename);
                                                }
                                                break;
                                            }
                                        }
                                        // Send LSP didChange
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let mut versions = doc_versions.borrow_mut();
                                        let version = versions.entry(path.clone()).or_insert(0);
                                        *version += 1;
                                        if let Err(e) = lsp_tx.try_send(LspRequest::DidChange {
                                            uri,
                                            version: *version,
                                            text: content,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::CursorMoved { line, column } => {
                                        status_bar.borrow().update_cursor_position(line as i32 - 1, column as i32 - 1);
                                        // Git blame
                                        match impulse_core::git::get_line_blame(&path, line) {
                                            Ok(blame) => {
                                                let text = format!(
                                                    "{} \u{2022} {} \u{2022} {}",
                                                    blame.author, blame.date, blame.summary
                                                );
                                                status_bar.borrow().update_blame(&text);
                                            }
                                            Err(_) => {
                                                status_bar.borrow().clear_blame();
                                            }
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::SaveRequested => {
                                        let content = handle.get_content();
                                        if let Err(e) = std::fs::write(&path, &content) {
                                            log::error!("Failed to save {}: {}", path, e);
                                            let toast = adw::Toast::new(&format!("Error saving: {}", e));
                                            toast.set_timeout(4);
                                            toast_overlay.add_toast(toast);
                                        } else {
                                            handle.is_modified.set(false);
                                            // Revert tab title
                                            let n = tab_view.n_pages();
                                            for i in 0..n {
                                                let page = tab_view.nth_page(i);
                                                if page.child().widget_name().as_str() == path {
                                                    let filename = std::path::Path::new(&path)
                                                        .file_name()
                                                        .and_then(|n| n.to_str())
                                                        .unwrap_or(&path);
                                                    page.set_title(filename);
                                                    break;
                                                }
                                            }
                                            let uri = file_path_to_uri(std::path::Path::new(&path))
                                                .unwrap_or_else(|| format!("file://{}", path));
                                            if let Err(e) = lsp_tx.try_send(LspRequest::DidSave { uri }) {
                                                log::warn!("LSP request channel full, dropping request: {}", e);
                                            }
                                            // Refresh diff decorations after save
                                            send_diff_decorations(handle, &path);
                                            // Refresh sidebar to update git status badges
                                            sidebar_state.refresh();
                                            // Run commands-on-save in a background thread
                                            let commands = settings.borrow().commands_on_save.clone();
                                            let save_path = path.clone();
                                            std::thread::spawn(move || {
                                                if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                                    let needs_reload = run_commands_on_save(&save_path, &commands);
                                                    if needs_reload {
                                                        let reload_path = save_path.clone();
                                                        gtk4::glib::MainContext::default().invoke(move || {
                                                            if let Some(handle) = crate::editor::get_handle(&reload_path) {
                                                                if let Ok(new_content) = std::fs::read_to_string(&reload_path) {
                                                                    let lang = handle.language.borrow().clone();
                                                                    handle.suppress_next_modify.set(true);
                                                                    handle.open_file(&reload_path, &new_content, &lang);
                                                                    send_diff_decorations(&handle, &reload_path);
                                                                }
                                                            }
                                                        });
                                                    }
                                                })) {
                                                    log::error!("Background thread panicked: {:?}", e);
                                                }
                                            });
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::CompletionRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_completion_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::Completion {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::HoverRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_hover_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::Hover {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::DefinitionRequested { request_id: monaco_id, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_definition_req.borrow_mut().insert(path.clone(), seq);
                                        definition_monaco_ids.borrow_mut().insert(seq, monaco_id);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::Definition {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::OpenFileRequested { uri, line, character } => {
                                        let file_path = uri_to_file_path(&uri);
                                        if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                                            cb(&file_path);
                                        }
                                        // Navigate to position once the tab is created
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
                                    impulse_editor::protocol::EditorEvent::FocusChanged { focused } => {
                                        // Auto-save on focus loss
                                        if !focused && settings.borrow().auto_save && handle.is_modified.get() {
                                            let content = handle.get_content();
                                            if let Err(e) = std::fs::write(&path, &content) {
                                                log::error!("Auto-save failed for {}: {}", path, e);
                                            } else {
                                                handle.is_modified.set(false);
                                                let n = tab_view.n_pages();
                                                for i in 0..n {
                                                    let page = tab_view.nth_page(i);
                                                    if page.child().widget_name().as_str() == path {
                                                        let filename = std::path::Path::new(&path)
                                                            .file_name()
                                                            .and_then(|n| n.to_str())
                                                            .unwrap_or(&path);
                                                        page.set_title(filename);
                                                        break;
                                                    }
                                                }
                                                let uri = file_path_to_uri(std::path::Path::new(&path))
                                                    .unwrap_or_else(|| format!("file://{}", path));
                                                if let Err(e) = lsp_tx.try_send(LspRequest::DidSave { uri }) {
                                                    log::warn!("LSP request channel full, dropping request: {}", e);
                                                }
                                                send_diff_decorations(handle, &path);
                                                sidebar_state.refresh();
                                            }
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::FormattingRequested { request_id: _, tab_size, insert_spaces } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_formatting_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::Formatting {
                                            request_id: seq,
                                            uri,
                                            version,
                                            tab_size,
                                            insert_spaces,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::SignatureHelpRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_signature_help_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::SignatureHelp {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::ReferencesRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_references_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::References {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::CodeActionRequested { request_id: _, start_line, start_column, end_line, end_column, diagnostics } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_code_action_req.borrow_mut().insert(path.clone(), seq);
                                        // Convert MonacoDiagnostic to DiagnosticInfo
                                        let diag_infos: Vec<crate::lsp_completion::DiagnosticInfo> = diagnostics.into_iter().map(|d| {
                                            crate::lsp_completion::DiagnosticInfo {
                                                line: d.start_line,
                                                character: d.start_column,
                                                end_line: d.end_line,
                                                end_character: d.end_column,
                                                severity: match d.severity {
                                                    8 => crate::lsp_completion::DiagnosticSeverity::Error,
                                                    4 => crate::lsp_completion::DiagnosticSeverity::Warning,
                                                    2 => crate::lsp_completion::DiagnosticSeverity::Information,
                                                    _ => crate::lsp_completion::DiagnosticSeverity::Hint,
                                                },
                                                message: d.message,
                                            }
                                        }).collect();
                                        if let Err(e) = lsp_tx.try_send(LspRequest::CodeAction {
                                            request_id: seq,
                                            uri,
                                            version,
                                            start_line,
                                            start_column,
                                            end_line,
                                            end_column,
                                            diagnostics: diag_infos,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::RenameRequested { request_id: _, line, character, new_name } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_rename_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::Rename {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                            new_name,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                    impulse_editor::protocol::EditorEvent::PrepareRenameRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_rename_req.borrow_mut().insert(path.clone(), seq);
                                        if let Err(e) = lsp_tx.try_send(LspRequest::PrepareRename {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        }) {
                                            log::warn!("LSP request channel full, dropping request: {}", e);
                                        }
                                    }
                                }
                            }
                        },
                    );
                    let page = tab_view.append(&editor_widget);
                    page.set_title(&filename);
                    if let Some(texture) = icon_cache.borrow().get(&filename, false, false) {
                        page.set_icon(Some(texture));
                    }

                    // Preserve sidebar tree state for the new tab
                    tree_states.borrow_mut().insert(
                        editor_widget.clone().upcast::<gtk4::Widget>(),
                        crate::sidebar::TabTreeState {
                            nodes: tree_nodes.borrow().clone(),
                            current_path: tree_current_path.borrow().clone(),
                            scroll_position: tree_scroll.vadjustment().value(),
                        },
                    );
                    tab_view.set_selected_page(&page);
                }
            });
        }));
    }

    // Wire up project search result activation to open file at line
    {
        let sidebar_on_file = sidebar_state.on_file_activated.clone();
        let tab_view = tab_view.clone();
        *sidebar_state
            .project_search
            .on_result_activated
            .borrow_mut() = Some(Box::new(move |path: &str, line: u32| {
            run_guarded_ui("project-search-result-activated", || {
                // First, open the file (reuse sidebar's callback)
                if let Some(cb) = sidebar_on_file.borrow().as_ref() {
                    cb(path);
                }
                // Then scroll to the specific line in the editor
                let n = tab_view.n_pages();
                for i in 0..n {
                    let page = tab_view.nth_page(i);
                    if page.child().widget_name().as_str() == path {
                        editor::go_to_position(&page.child(), line, 1);
                        break;
                    }
                }
            });
        }));
    }

    // Wire up "Open in Terminal" context menu action to cd into directory
    {
        let tab_view = tab_view.clone();
        *sidebar_state.on_open_terminal.borrow_mut() = Some(Box::new(move |path: &str| {
            // Feed cd command to the active terminal
            if let Some(page) = tab_view.selected_page() {
                if let Some(term) = terminal_container::get_active_terminal(&page.child()) {
                    let cmd = format!("cd '{}'\n", path.replace('\'', "'\\''"));
                    term.feed_child(cmd.as_bytes());
                }
            }
        }));
    }
}
