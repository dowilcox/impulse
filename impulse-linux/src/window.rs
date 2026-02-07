use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use sourceview5::prelude::*;
use vte4::prelude::*;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::editor;
use crate::lsp_completion::{LspRequest, LspResponse};
use crate::sidebar;
use crate::status_bar;
use crate::terminal;
use crate::terminal_container;

#[derive(Clone)]
struct Command {
    name: String,
    shortcut: String,
    action: Rc<dyn Fn()>,
}

pub fn build_window(app: &adw::Application) {
    let settings = Rc::new(RefCell::new(crate::settings::load()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Impulse")
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .build();

    // Shared font size state (user-facing size in points, e.g. 11)
    let font_size: Rc<Cell<i32>> = Rc::new(Cell::new(settings.borrow().font_size));

    // --- LSP Bridge: GTK <-> Tokio ---
    // Channel for sending requests from GTK to the LSP tokio runtime
    let (lsp_request_tx, mut lsp_request_rx) = tokio::sync::mpsc::unbounded_channel::<LspRequest>();
    let lsp_request_tx = Rc::new(lsp_request_tx);

    // Channel for sending responses from the LSP runtime back to GTK
    // We use std::sync::mpsc since glib::MainContext::channel is not available in this version.
    // A glib timeout polls the receiver periodically.
    let (lsp_gtk_tx, lsp_gtk_rx) = std::sync::mpsc::channel::<LspResponse>();
    let lsp_gtk_rx = Rc::new(RefCell::new(lsp_gtk_rx));

    // Spawn the tokio runtime in a background thread
    {
        let initial_dir = if !settings.borrow().last_directory.is_empty()
            && std::path::Path::new(&settings.borrow().last_directory).is_dir()
        {
            settings.borrow().last_directory.clone()
        } else {
            impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string())
        };
        let root_uri = format!("file://{}", initial_dir);
        let gtk_tx = lsp_gtk_tx.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for LSP");

            rt.block_on(async move {
                // Create the event channel from core LspEvents
                let (event_tx, mut event_rx) =
                    tokio::sync::mpsc::unbounded_channel::<impulse_core::lsp::LspEvent>();

                let registry = std::sync::Arc::new(
                    impulse_core::lsp::LspRegistry::new(root_uri, event_tx),
                );

                // Task to forward LspEvents to the GTK main loop
                let gtk_tx_events = gtk_tx.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let response = match event {
                            impulse_core::lsp::LspEvent::Diagnostics { uri, diagnostics } => {
                                let diags = diagnostics
                                    .into_iter()
                                    .map(|d| {
                                        let range = d.range;
                                        let severity = match d.severity {
                                            Some(lsp_types::DiagnosticSeverity::ERROR) => {
                                                crate::lsp_completion::DiagnosticSeverity::Error
                                            }
                                            Some(lsp_types::DiagnosticSeverity::WARNING) => {
                                                crate::lsp_completion::DiagnosticSeverity::Warning
                                            }
                                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => {
                                                crate::lsp_completion::DiagnosticSeverity::Information
                                            }
                                            _ => crate::lsp_completion::DiagnosticSeverity::Hint,
                                        };
                                        crate::lsp_completion::DiagnosticInfo {
                                            line: range.start.line,
                                            character: range.start.character,
                                            end_line: range.end.line,
                                            end_character: range.end.character,
                                            severity,
                                            message: d.message,
                                        }
                                    })
                                    .collect();
                                LspResponse::Diagnostics {
                                    uri,
                                    diagnostics: diags,
                                }
                            }
                            impulse_core::lsp::LspEvent::Initialized { language_id } => {
                                LspResponse::ServerInitialized { language_id }
                            }
                            impulse_core::lsp::LspEvent::ServerError {
                                language_id,
                                message,
                            } => LspResponse::ServerError {
                                language_id,
                                message,
                            },
                            impulse_core::lsp::LspEvent::ServerExited { language_id } => {
                                LspResponse::ServerExited { language_id }
                            }
                        };
                        if gtk_tx_events.send(response).is_err() {
                            break;
                        }
                    }
                });

                // Main request processing loop
                let gtk_tx_req = gtk_tx;
                while let Some(request) = lsp_request_rx.recv().await {
                    let registry = registry.clone();
                    let gtk_tx = gtk_tx_req.clone();
                    tokio::spawn(async move {
                        match request {
                            LspRequest::DidOpen {
                                uri,
                                language_id,
                                version,
                                text,
                            } => {
                                if let Some(client) =
                                    registry.get_client(&language_id).await
                                {
                                    let _ = client.did_open(
                                        &uri,
                                        &language_id,
                                        version,
                                        &text,
                                    );
                                }
                            }
                            LspRequest::DidChange {
                                uri,
                                version,
                                text,
                            } => {
                                // Determine language from URI extension
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    let _ = client.did_change(&uri, version, &text);
                                }
                            }
                            LspRequest::DidSave { uri } => {
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    let _ = client.did_save(&uri);
                                }
                            }
                            LspRequest::DidClose { uri } => {
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    let _ = client.did_close(&uri);
                                }
                            }
                            LspRequest::Completion {
                                uri,
                                line,
                                character,
                            } => {
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    if let Ok(items) =
                                        client.completion(&uri, line, character).await
                                    {
                                        let completions = items
                                            .into_iter()
                                            .map(|item| {
                                                crate::lsp_completion::CompletionInfo {
                                                    label: item.label,
                                                    detail: item.detail,
                                                    insert_text: item.insert_text,
                                                    kind: format!("{:?}", item.kind.unwrap_or(lsp_types::CompletionItemKind::TEXT)),
                                                }
                                            })
                                            .collect();
                                        let _ = gtk_tx.send(LspResponse::CompletionResult {
                                            items: completions,
                                        });
                                    }
                                }
                            }
                            LspRequest::Hover {
                                uri,
                                line,
                                character,
                            } => {
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    if let Ok(Some(hover)) =
                                        client.hover(&uri, line, character).await
                                    {
                                        let content =
                                            crate::lsp_hover::hover_content_to_string(&hover);
                                        let _ = gtk_tx.send(LspResponse::HoverResult {
                                            contents: content,
                                        });
                                    }
                                }
                            }
                            LspRequest::Definition {
                                uri,
                                line,
                                character,
                            } => {
                                let lang = language_from_uri(&uri);
                                if let Some(client) = registry.get_client(&lang).await {
                                    if let Ok(Some(def)) =
                                        client.definition(&uri, line, character).await
                                    {
                                        let location = match def {
                                            lsp_types::GotoDefinitionResponse::Scalar(loc) => {
                                                Some(loc)
                                            }
                                            lsp_types::GotoDefinitionResponse::Array(locs) => {
                                                locs.into_iter().next()
                                            }
                                            lsp_types::GotoDefinitionResponse::Link(links) => {
                                                links.into_iter().next().map(|l| {
                                                    lsp_types::Location {
                                                        uri: l.target_uri,
                                                        range: l.target_selection_range,
                                                    }
                                                })
                                            }
                                        };
                                        if let Some(loc) = location {
                                            let _ =
                                                gtk_tx.send(LspResponse::DefinitionResult {
                                                    uri: loc.uri.to_string(),
                                                    line: loc.range.start.line,
                                                    character: loc.range.start.character,
                                                });
                                        }
                                    }
                                }
                            }
                            LspRequest::Shutdown => {
                                registry.shutdown_all().await;
                            }
                        }
                    });
                }
            });
        });
    }

    // Shared document version counter for LSP
    let lsp_doc_versions: Rc<RefCell<std::collections::HashMap<String, i32>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));

    // Main vertical layout
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Header bar with tab bar
    let header = adw::HeaderBar::new();
    let tab_bar = adw::TabBar::new();
    let tab_view = adw::TabView::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(false);

    // Tab context menu
    let tab_menu = gio::Menu::new();
    tab_menu.append(Some("New Tab"), Some("tab.new"));
    tab_menu.append(Some("Pin/Unpin Tab"), Some("tab.pin"));
    tab_menu.append(Some("Close Tab"), Some("tab.close"));
    tab_menu.append(Some("Close Other Tabs"), Some("tab.close-others"));
    tab_view.set_menu_model(Some(&tab_menu));

    header.set_title_widget(Some(&tab_bar));

    // Toggle sidebar button (leftmost)
    let sidebar_btn = gtk4::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle Sidebar (Ctrl+Shift+B)")
        .active(settings.borrow().sidebar_visible)
        .build();
    sidebar_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&sidebar_btn);

    // New tab button
    let new_tab_btn = gtk4::Button::from_icon_name("tab-new-symbolic");
    new_tab_btn.set_tooltip_text(Some("New Tab (Ctrl+T)"));
    new_tab_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&new_tab_btn);

    // Settings button (right side of header)
    let settings_btn = gtk4::Button::from_icon_name("emblem-system-symbolic");
    settings_btn.set_tooltip_text(Some("Settings"));
    settings_btn.set_cursor_from_name(Some("pointer"));
    {
        let window_ref = window.clone();
        let settings = settings.clone();
        settings_btn.connect_clicked(move |_| {
            crate::settings_page::show_settings_window(&window_ref, &settings, |_s| {});
        });
    }
    header.pack_end(&settings_btn);

    main_box.append(&header);

    // Horizontal pane: sidebar + tab view
    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_position(settings.borrow().sidebar_width);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Sidebar
    let (sidebar_widget, sidebar_state) = sidebar::build_sidebar();
    sidebar_widget.set_visible(settings.borrow().sidebar_visible);
    paned.set_start_child(Some(&sidebar_widget));

    // Terminal search bar (hidden by default)
    let search_revealer = gtk4::Revealer::new();
    search_revealer.set_reveal_child(false);
    search_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);

    let search_bar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    search_bar_box.add_css_class("terminal-search-bar");

    let find_entry = gtk4::SearchEntry::new();
    find_entry.set_placeholder_text(Some("Find in terminal..."));
    find_entry.set_hexpand(true);

    let find_prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    find_prev_btn.set_tooltip_text(Some("Previous Match"));
    let find_next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    find_next_btn.set_tooltip_text(Some("Next Match"));
    let find_close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    find_close_btn.set_tooltip_text(Some("Close"));

    search_bar_box.append(&find_entry);
    search_bar_box.append(&find_prev_btn);
    search_bar_box.append(&find_next_btn);
    search_bar_box.append(&find_close_btn);
    search_revealer.set_child(Some(&search_bar_box));

    // Editor search/replace bar (hidden by default)
    let editor_search_revealer = gtk4::Revealer::new();
    editor_search_revealer.set_reveal_child(false);
    editor_search_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);

    let editor_search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    editor_search_box.add_css_class("terminal-search-bar");

    // Find row
    let editor_find_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let editor_find_entry = gtk4::SearchEntry::new();
    editor_find_entry.set_placeholder_text(Some("Find..."));
    editor_find_entry.set_hexpand(true);
    let editor_find_prev = gtk4::Button::from_icon_name("go-up-symbolic");
    editor_find_prev.set_tooltip_text(Some("Previous Match"));
    let editor_find_next = gtk4::Button::from_icon_name("go-down-symbolic");
    editor_find_next.set_tooltip_text(Some("Next Match"));
    let editor_match_label = gtk4::Label::new(Some(""));
    editor_match_label.add_css_class("dim-label");
    let editor_find_close = gtk4::Button::from_icon_name("window-close-symbolic");
    editor_find_close.set_tooltip_text(Some("Close"));
    editor_find_row.append(&editor_find_entry);
    editor_find_row.append(&editor_match_label);
    editor_find_row.append(&editor_find_prev);
    editor_find_row.append(&editor_find_next);
    editor_find_row.append(&editor_find_close);

    // Replace row (can be toggled with Ctrl+H)
    let editor_replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    editor_replace_row.set_visible(false);
    let editor_replace_entry = gtk4::Entry::new();
    editor_replace_entry.set_placeholder_text(Some("Replace..."));
    editor_replace_entry.set_hexpand(true);
    let replace_btn = gtk4::Button::with_label("Replace");
    replace_btn.set_tooltip_text(Some("Replace current match"));
    let replace_all_btn = gtk4::Button::with_label("All");
    replace_all_btn.set_tooltip_text(Some("Replace all matches"));
    editor_replace_row.append(&editor_replace_entry);
    editor_replace_row.append(&replace_btn);
    editor_replace_row.append(&replace_all_btn);

    editor_search_box.append(&editor_find_row);
    editor_search_box.append(&editor_replace_row);
    editor_search_revealer.set_child(Some(&editor_search_box));

    // Shared editor search state
    let editor_search_settings = sourceview5::SearchSettings::new();
    editor_search_settings.set_wrap_around(true);
    let editor_search_ctx: Rc<RefCell<Option<sourceview5::SearchContext>>> =
        Rc::new(RefCell::new(None));

    // Tab view in the end pane, wrapped with search bars above
    let right_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    right_box.append(&search_revealer);
    right_box.append(&editor_search_revealer);
    right_box.append(&tab_view);
    tab_view.set_vexpand(true);
    paned.set_end_child(Some(&right_box));

    main_box.append(&paned);

    // Status bar
    let status_bar = status_bar::new_shared();
    main_box.append(&status_bar.borrow().widget);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Load initial directory (use last saved directory if available)
    let initial_dir = if !settings.borrow().last_directory.is_empty()
        && std::path::Path::new(&settings.borrow().last_directory).is_dir()
    {
        settings.borrow().last_directory.clone()
    } else {
        impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string())
    };
    sidebar_state.load_directory(&initial_dir);
    status_bar.borrow().update_cwd(&initial_dir);

    // Shared state
    let sidebar_state = Rc::new(sidebar_state);

    // Wire up file activation to open in editor tab
    {
        let tab_view = tab_view.clone();
        let status_bar = status_bar.clone();
        let settings = settings.clone();
        let lsp_tx = lsp_request_tx.clone();
        let doc_versions = lsp_doc_versions.clone();
        *sidebar_state.on_file_activated.borrow_mut() = Some(Box::new(move |path: &str| {
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
                tab_view.set_selected_page(&page);
            } else if !editor::is_binary_file(path) {
                // Open file in new editor tab
                let (editor_widget, buffer) = editor::create_editor(path, &settings.borrow());
                let page = tab_view.append(&editor_widget);
                page.set_title(&filename);

                // Track unsaved changes
                {
                    let page = page.clone();
                    let filename = filename.clone();
                    buffer.connect_modified_changed(move |buf| {
                        if buf.is_modified() {
                            page.set_title(&format!("\u{25CF} {}", filename)); // ● dot prefix
                        } else {
                            page.set_title(&filename);
                        }
                    });
                }

                // Live cursor position updates
                {
                    let status_bar = status_bar.clone();
                    let tab_view = tab_view.clone();
                    let editor_widget_name = editor_widget.widget_name().to_string();
                    buffer.connect_notify_local(Some("cursor-position"), move |buf, _| {
                        // Only update if this editor's tab is currently selected
                        if let Some(page) = tab_view.selected_page() {
                            if page.child().widget_name().as_str() == editor_widget_name {
                                let insert_mark = buf.get_insert();
                                let iter = buf.iter_at_mark(&insert_mark);
                                let line = iter.line();
                                let col = iter.line_offset();
                                status_bar.borrow().update_cursor_position(line, col);
                            }
                        }
                    });
                }

                // LSP: send didOpen
                {
                    let uri = format!("file://{}", path);
                    let language_id = editor::get_editor_language(editor_widget.upcast_ref())
                        .unwrap_or_default()
                        .to_lowercase();
                    let start = buffer.start_iter();
                    let end = buffer.end_iter();
                    let text = buffer.text(&start, &end, true).to_string();
                    let mut versions = doc_versions.borrow_mut();
                    let version = versions.entry(path.to_string()).or_insert(0);
                    *version += 1;
                    let _ = lsp_tx.send(LspRequest::DidOpen {
                        uri,
                        language_id,
                        version: *version,
                        text,
                    });
                }

                // LSP: send didChange on buffer modifications (debounced with auto-save)
                {
                    let lsp_tx = lsp_tx.clone();
                    let doc_versions = doc_versions.clone();
                    let path_for_lsp = path.to_string();
                    let lsp_change_source: Rc<RefCell<Option<gtk4::glib::SourceId>>> =
                        Rc::new(RefCell::new(None));
                    buffer.connect_changed({
                        let buf = buffer.clone();
                        move |_| {
                            // Cancel pending LSP change notification
                            if let Some(source_id) = lsp_change_source.borrow_mut().take() {
                                source_id.remove();
                            }
                            let lsp_tx = lsp_tx.clone();
                            let doc_versions = doc_versions.clone();
                            let path = path_for_lsp.clone();
                            let buf = buf.clone();
                            let lsp_inner = lsp_change_source.clone();
                            let source_id = gtk4::glib::timeout_add_local_once(
                                std::time::Duration::from_millis(500),
                                move || {
                                    let start = buf.start_iter();
                                    let end = buf.end_iter();
                                    let text = buf.text(&start, &end, true).to_string();
                                    let uri = format!("file://{}", path);
                                    let mut versions = doc_versions.borrow_mut();
                                    let version = versions.entry(path.clone()).or_insert(0);
                                    *version += 1;
                                    let _ = lsp_tx.send(LspRequest::DidChange {
                                        uri,
                                        version: *version,
                                        text,
                                    });
                                    *lsp_inner.borrow_mut() = None;
                                },
                            );
                            *lsp_change_source.borrow_mut() = Some(source_id);
                        }
                    });
                }

                // Auto-save after 2 seconds of inactivity
                {
                    let path = path.to_string();
                    let auto_save_source: Rc<RefCell<Option<gtk4::glib::SourceId>>> =
                        Rc::new(RefCell::new(None));
                    let settings = settings.clone();
                    let lsp_tx = lsp_tx.clone();

                    buffer.connect_changed(move |buf| {
                        // Cancel any pending auto-save
                        if let Some(source_id) = auto_save_source.borrow_mut().take() {
                            source_id.remove();
                        }

                        // Only auto-save if modified
                        if !buf.is_modified() {
                            return;
                        }

                        let buf = buf.clone();
                        let path = path.clone();
                        let auto_save_inner = auto_save_source.clone();
                        let settings = settings.clone();
                        let lsp_tx = lsp_tx.clone();

                        let source_id = gtk4::glib::timeout_add_local_once(
                            std::time::Duration::from_secs(2),
                            move || {
                                if buf.is_modified() {
                                    let start = buf.start_iter();
                                    let end = buf.end_iter();
                                    let text = buf.text(&start, &end, true);
                                    if let Err(e) = std::fs::write(&path, text.as_str()) {
                                        log::error!("Auto-save failed for {}: {}", path, e);
                                    } else {
                                        buf.set_modified(false);
                                        log::info!("Auto-saved: {}", path);
                                        // LSP: send didSave
                                        let _ = lsp_tx.send(LspRequest::DidSave {
                                            uri: format!("file://{}", path),
                                        });
                                        // Run commands on save in a background thread
                                        let commands = settings.borrow().commands_on_save.clone();
                                        let save_path = path.clone();
                                        std::thread::spawn(move || {
                                            run_commands_on_save(&save_path, &commands);
                                        });
                                    }
                                }
                                // Clear the source ID
                                *auto_save_inner.borrow_mut() = None;
                            },
                        );

                        *auto_save_source.borrow_mut() = Some(source_id);
                    });
                }

                tab_view.set_selected_page(&page);
            }
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

    // --- Helper: connect signals on a terminal (CWD change + child-exited) ---
    let setup_terminal_signals = {
        let tab_view = tab_view.clone();
        let status_bar = status_bar.clone();
        let sidebar_state = sidebar_state.clone();
        Rc::new(move |term: &vte4::Terminal| {
            // Connect CWD change signal (OSC 7)
            {
                let status_bar = status_bar.clone();
                let sidebar_state = sidebar_state.clone();
                let tab_view = tab_view.clone();
                term.connect_current_directory_uri_notify(move |terminal| {
                    if let Some(uri) = terminal.current_directory_uri() {
                        let uri_str = uri.to_string();
                        // Strip file:// prefix
                        let path = if let Some(rest) = uri_str.strip_prefix("file://") {
                            // Skip hostname
                            if let Some(slash_idx) = rest.find('/') {
                                &rest[slash_idx..]
                            } else {
                                rest
                            }
                        } else {
                            &uri_str
                        };
                        let path = url_decode(path);

                        // Only update sidebar/status bar if this terminal is in the active tab
                        let is_active = tab_view
                            .selected_page()
                            .is_some_and(|p| terminal.is_ancestor(&p.child()));
                        if is_active {
                            status_bar.borrow().update_cwd(&path);
                            sidebar_state.load_directory(&path);
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
            }

            // Connect child-exited to close the tab
            {
                let tab_view = tab_view.clone();
                let term_clone = term.clone();
                term.connect_child_exited(move |_terminal, _status| {
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
            }
        })
    };

    // --- Wire up tab creation ---
    let create_tab = {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        move || {
            let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
            let term = terminal::create_terminal(&settings.borrow(), theme);
            setup_terminal_signals(&term);
            terminal::spawn_shell(&term);

            let container = terminal_container::TerminalContainer::new(&term);
            let page = tab_view.append(&container.widget);
            page.set_title(&impulse_core::shell::get_default_shell_name());
            tab_view.set_selected_page(&page);
            term.grab_focus();
        }
    };

    // Create first tab
    (create_tab.clone())();

    // Restore previously open editor tabs
    for file_path in &settings.borrow().open_files.clone() {
        if std::path::Path::new(file_path).exists() {
            if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                cb(file_path);
            }
        }
    }

    // Switch back to first tab (terminal) after restoring editor tabs
    if tab_view.n_pages() > 0 {
        tab_view.set_selected_page(&tab_view.nth_page(0));
    }

    // New tab button
    {
        let create_tab = create_tab.clone();
        new_tab_btn.connect_clicked(move |_| {
            create_tab();
        });
    }

    // Toggle sidebar
    {
        let sidebar_widget = sidebar_widget.clone();
        sidebar_btn.connect_toggled(move |btn: &gtk4::ToggleButton| {
            sidebar_widget.set_visible(btn.is_active());
        });
    }

    // --- Tab context menu actions ---
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

    // --- Poll LSP responses on the GTK main loop ---
    {
        let tab_view = tab_view.clone();
        let sidebar_state = sidebar_state.clone();
        let lsp_gtk_rx = lsp_gtk_rx.clone();
        gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            let rx = lsp_gtk_rx.borrow();
            while let Ok(response) = rx.try_recv() {
                match response {
                    LspResponse::Diagnostics { uri, diagnostics } => {
                        let file_path = uri_to_file_path(&uri);
                        let n = tab_view.n_pages();
                        for i in 0..n {
                            let page = tab_view.nth_page(i);
                            let child = page.child();
                            if child.widget_name().as_str() == file_path {
                                if let Some(buf) = editor::get_editor_buffer(&child) {
                                    if let Some(view) = editor::get_editor_view(&child) {
                                        crate::lsp_completion::apply_diagnostics(
                                            &buf,
                                            &view,
                                            &diagnostics,
                                        );
                                    }
                                }
                                break;
                            }
                        }
                    }
                    LspResponse::DefinitionResult {
                        uri,
                        line,
                        character,
                    } => {
                        let file_path = uri_to_file_path(&uri);
                        // Open the file via sidebar callback
                        if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                            cb(&file_path);
                        }
                        // Navigate to the position
                        let n = tab_view.n_pages();
                        for i in 0..n {
                            let page = tab_view.nth_page(i);
                            let child = page.child();
                            if child.widget_name().as_str() == file_path {
                                if let Some(buf) = editor::get_editor_buffer(&child) {
                                    if let Some(iter) =
                                        buf.iter_at_line_offset(line as i32, character as i32)
                                    {
                                        buf.place_cursor(&iter);
                                        if let Some(view) = editor::get_editor_view(&child) {
                                            view.scroll_to_iter(
                                                &mut iter.clone(),
                                                0.1,
                                                true,
                                                0.0,
                                                0.5,
                                            );
                                        }
                                    }
                                }
                                tab_view.set_selected_page(&page);
                                break;
                            }
                        }
                    }
                    LspResponse::ServerInitialized { language_id } => {
                        log::info!("LSP server initialized for: {}", language_id);
                    }
                    LspResponse::ServerError {
                        language_id,
                        message,
                    } => {
                        log::warn!("LSP server error for {}: {}", language_id, message);
                    }
                    LspResponse::ServerExited { language_id } => {
                        log::info!("LSP server exited for: {}", language_id);
                    }
                    _ => {}
                }
            }
            gtk4::glib::ControlFlow::Continue
        });
    }

    // --- Keyboard shortcuts ---
    let shortcut_controller = gtk4::ShortcutController::new();
    shortcut_controller.set_scope(gtk4::ShortcutScope::Global);

    // Ctrl+T: New tab
    {
        let create_tab = create_tab.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>t", move || {
            create_tab();
        });
    }

    // Ctrl+W: Close current tab
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>w", move || {
            if let Some(page) = tab_view.selected_page() {
                tab_view.close_page(&page);
            }
        });
    }

    // Ctrl+Shift+B: Toggle sidebar
    {
        let sidebar_btn = sidebar_btn.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>b", move || {
            sidebar_btn.set_active(!sidebar_btn.is_active());
        });
    }

    // Build command list for the command palette
    let commands = {
        let create_tab = create_tab.clone();
        let tab_view = tab_view.clone();
        let sidebar_btn = sidebar_btn.clone();
        let window_ref = window.clone();
        let sidebar_state = sidebar_state.clone();
        let search_revealer = search_revealer.clone();
        let find_entry = find_entry.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();

        vec![
            Command {
                name: "New Terminal Tab".to_string(),
                shortcut: "Ctrl+T".to_string(),
                action: Rc::new({
                    let create_tab = create_tab.clone();
                    move || create_tab()
                }),
            },
            Command {
                name: "Close Tab".to_string(),
                shortcut: "Ctrl+W".to_string(),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            tab_view.close_page(&page);
                        }
                    }
                }),
            },
            Command {
                name: "Toggle Sidebar".to_string(),
                shortcut: "Ctrl+Shift+B".to_string(),
                action: Rc::new({
                    let sidebar_btn = sidebar_btn.clone();
                    move || sidebar_btn.set_active(!sidebar_btn.is_active())
                }),
            },
            Command {
                name: "Quick Open File".to_string(),
                shortcut: "".to_string(),
                action: Rc::new({
                    let window_ref = window_ref.clone();
                    let sidebar_state = sidebar_state.clone();
                    move || show_quick_open(&window_ref, &sidebar_state)
                }),
            },
            Command {
                name: "Find in Terminal".to_string(),
                shortcut: "Ctrl+Shift+F".to_string(),
                action: Rc::new({
                    let search_revealer = search_revealer.clone();
                    let find_entry = find_entry.clone();
                    move || {
                        search_revealer.set_reveal_child(true);
                        find_entry.grab_focus();
                    }
                }),
            },
            Command {
                name: "Toggle Fullscreen".to_string(),
                shortcut: "F11".to_string(),
                action: Rc::new({
                    let window_ref = window_ref.clone();
                    move || {
                        if window_ref.is_fullscreen() {
                            window_ref.unfullscreen();
                        } else {
                            window_ref.fullscreen();
                        }
                    }
                }),
            },
            Command {
                name: "New Window".to_string(),
                shortcut: "Ctrl+Shift+N".to_string(),
                action: Rc::new({
                    let app = app.clone();
                    move || build_window(&app)
                }),
            },
            Command {
                name: "Split Terminal Horizontally".to_string(),
                shortcut: "Ctrl+Shift+E".to_string(),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            let setup = setup_terminal_signals.clone();
                            let s = settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Horizontal,
                                &|term| {
                                    setup(term);
                                },
                                &s,
                                theme,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Split Terminal Vertically".to_string(),
                shortcut: "Ctrl+Shift+O".to_string(),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    move || {
                        if let Some(page) = tab_view.selected_page() {
                            let child = page.child();
                            let setup = setup_terminal_signals.clone();
                            let s = settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Vertical,
                                &|term| {
                                    setup(term);
                                },
                                &s,
                                theme,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Open Settings".to_string(),
                shortcut: "Ctrl+,".to_string(),
                action: Rc::new({
                    let window_ref = window_ref.clone();
                    let settings = settings.clone();
                    move || {
                        crate::settings_page::show_settings_window(&window_ref, &settings, |_s| {
                            // Settings changed callback — could reload theme/etc at runtime
                        });
                    }
                }),
            },
        ]
    };

    // Ctrl+Shift+P: Command palette
    {
        let window_ref = window.clone();
        let commands = commands.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>p", move || {
            show_command_palette(&window_ref, &commands);
        });
    }

    // Ctrl+,: Open Settings
    {
        let window_ref = window.clone();
        let settings = settings.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>comma", move || {
            crate::settings_page::show_settings_window(&window_ref, &settings, |_s| {});
        });
    }

    // Ctrl+Tab / Ctrl+Shift+Tab: Switch tabs
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>Tab", move || {
            let n = tab_view.n_pages();
            if n <= 1 {
                return;
            }
            if let Some(current) = tab_view.selected_page() {
                let pos = tab_view.page_position(&current);
                let next = (pos + 1) % n;
                tab_view.set_selected_page(&tab_view.nth_page(next));
            }
        });
    }
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>Tab", move || {
            let n = tab_view.n_pages();
            if n <= 1 {
                return;
            }
            if let Some(current) = tab_view.selected_page() {
                let pos = tab_view.page_position(&current);
                let prev = if pos == 0 { n - 1 } else { pos - 1 };
                tab_view.set_selected_page(&tab_view.nth_page(prev));
            }
        });
    }

    // Ctrl+1-9: Switch to tab by number
    for i in 1..=9u32 {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, &format!("<Ctrl>{}", i), move || {
            let idx = (i - 1) as i32;
            if idx < tab_view.n_pages() {
                tab_view.set_selected_page(&tab_view.nth_page(idx));
            }
        });
    }

    // Ctrl+Shift+C: Copy selected text
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>c", move || {
            if let Some(page) = tab_view.selected_page() {
                if let Some(term) = terminal_container::get_active_terminal(&page.child()) {
                    term.copy_clipboard_format(vte4::Format::Text);
                }
            }
        });
    }

    // Ctrl+Shift+V: Paste clipboard
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>v", move || {
            if let Some(page) = tab_view.selected_page() {
                if let Some(term) = terminal_container::get_active_terminal(&page.child()) {
                    term.paste_clipboard();
                }
            }
        });
    }

    // Ctrl+Equal / Ctrl+plus: Increase font size
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>equal", move || {
            let new_size = font_size.get() + 1;
            font_size.set(new_size);
            apply_font_size_to_all_terminals(&tab_view, new_size);
        });
    }

    // Ctrl+minus: Decrease font size
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>minus", move || {
            let new_size = font_size.get() - 1;
            if new_size > 0 {
                font_size.set(new_size);
                apply_font_size_to_all_terminals(&tab_view, new_size);
            }
        });
    }

    // Ctrl+0: Reset font size to default
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>0", move || {
            font_size.set(11);
            apply_font_size_to_all_terminals(&tab_view, 11);
        });
    }

    // Ctrl+Shift+F: Find in terminal
    {
        let search_revealer = search_revealer.clone();
        let find_entry = find_entry.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>f", move || {
            let is_visible = search_revealer.reveals_child();
            search_revealer.set_reveal_child(!is_visible);
            if !is_visible {
                find_entry.grab_focus();
            }
        });
    }

    // Ctrl+F: Context-aware find (terminal search for terminals, editor search for editors)
    {
        let tab_view = tab_view.clone();
        let search_revealer = search_revealer.clone();
        let find_entry = find_entry.clone();
        let editor_search_revealer = editor_search_revealer.clone();
        let editor_find_entry = editor_find_entry.clone();
        let editor_replace_row = editor_replace_row.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>f", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    // Editor tab: show editor search bar (hide replace row for Ctrl+F)
                    let is_visible = editor_search_revealer.reveals_child();
                    editor_search_revealer.set_reveal_child(!is_visible);
                    if !is_visible {
                        editor_replace_row.set_visible(false);
                        editor_find_entry.grab_focus();
                    }
                    // Hide terminal search if open
                    search_revealer.set_reveal_child(false);
                } else {
                    // Terminal tab: same as Ctrl+Shift+F
                    let is_visible = search_revealer.reveals_child();
                    search_revealer.set_reveal_child(!is_visible);
                    if !is_visible {
                        find_entry.grab_focus();
                    }
                    // Hide editor search if open
                    editor_search_revealer.set_reveal_child(false);
                }
            }
        });
    }

    // Ctrl+H: Find and replace in editor tabs
    {
        let tab_view = tab_view.clone();
        let editor_search_revealer = editor_search_revealer.clone();
        let editor_find_entry = editor_find_entry.clone();
        let editor_replace_row = editor_replace_row.clone();
        let search_revealer = search_revealer.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>h", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    let is_visible =
                        editor_search_revealer.reveals_child() && editor_replace_row.is_visible();
                    if is_visible {
                        // Already open with replace visible: close it
                        editor_search_revealer.set_reveal_child(false);
                    } else {
                        // Show search bar with replace row
                        editor_replace_row.set_visible(true);
                        editor_search_revealer.set_reveal_child(true);
                        editor_find_entry.grab_focus();
                    }
                    // Hide terminal search if open
                    search_revealer.set_reveal_child(false);
                }
            }
        });
    }

    // Ctrl+S: Save current editor tab
    {
        let tab_view = tab_view.clone();
        let toast_overlay = toast_overlay.clone();
        let lsp_tx = lsp_request_tx.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>s", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    let path = child.widget_name().to_string();
                    if let Some(text) = editor::get_editor_text(&child) {
                        match std::fs::write(&path, &text) {
                            Ok(()) => {
                                // Reset modified flag so tab title reverts
                                if let Some(buf) = editor::get_editor_buffer(&child) {
                                    buf.set_modified(false);
                                }
                                // LSP: send didSave
                                let _ = lsp_tx.send(LspRequest::DidSave {
                                    uri: format!("file://{}", path),
                                });
                                let filename = std::path::Path::new(&path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(&path);
                                let toast = adw::Toast::new(&format!("Saved {}", filename));
                                toast.set_timeout(2);
                                toast_overlay.add_toast(toast);
                            }
                            Err(e) => {
                                let toast = adw::Toast::new(&format!("Error saving: {}", e));
                                toast.set_timeout(4);
                                toast_overlay.add_toast(toast);
                            }
                        }
                    }
                }
            }
        });
    }

    // Ctrl+Shift+E: Split terminal horizontally (side by side)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>e", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                let setup = setup_terminal_signals.clone();
                let s = settings.borrow();
                let theme = crate::theme::get_theme(&s.color_scheme);
                terminal_container::split_terminal(
                    &child,
                    gtk4::Orientation::Horizontal,
                    &|term| {
                        setup(term);
                    },
                    &s,
                    theme,
                );
            }
        });
    }

    // Ctrl+Shift+O: Split terminal vertically (top/bottom)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>o", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                let setup = setup_terminal_signals.clone();
                let s = settings.borrow();
                let theme = crate::theme::get_theme(&s.color_scheme);
                terminal_container::split_terminal(
                    &child,
                    gtk4::Orientation::Vertical,
                    &|term| {
                        setup(term);
                    },
                    &s,
                    theme,
                );
            }
        });
    }

    // Alt+Left: Focus previous split pane
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Alt>Left", move || {
            if let Some(page) = tab_view.selected_page() {
                terminal_container::focus_prev_terminal(&page.child());
            }
        });
    }

    // Alt+Right: Focus next split pane
    {
        let tab_view = tab_view.clone();
        add_shortcut(&shortcut_controller, "<Alt>Right", move || {
            if let Some(page) = tab_view.selected_page() {
                terminal_container::focus_next_terminal(&page.child());
            }
        });
    }

    // --- Terminal search bar wiring ---

    // Search entry text changed -> set regex on active terminal
    {
        let tab_view_ref = tab_view.clone();
        find_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string();
            if let Some(page) = tab_view_ref.selected_page() {
                let child = page.child();
                if let Some(term) = find_vte_terminal(&child) {
                    if text.is_empty() {
                        term.search_set_regex(None::<&vte4::Regex>, 0);
                    } else {
                        let escaped = regex_escape(&text);
                        if let Ok(regex) = vte4::Regex::for_search(&escaped, 0) {
                            term.search_set_regex(Some(&regex), 0);
                            term.search_find_next();
                        }
                    }
                }
            }
        });
    }

    // Next button
    {
        let tab_view_ref = tab_view.clone();
        find_next_btn.connect_clicked(move |_| {
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_find_next();
                }
            }
        });
    }

    // Previous button
    {
        let tab_view_ref = tab_view.clone();
        find_prev_btn.connect_clicked(move |_| {
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_find_previous();
                }
            }
        });
    }

    // Close button
    {
        let search_revealer_ref = search_revealer.clone();
        let tab_view_ref = tab_view.clone();
        find_close_btn.connect_clicked(move |_| {
            search_revealer_ref.set_reveal_child(false);
            if let Some(page) = tab_view_ref.selected_page() {
                if let Some(term) = find_vte_terminal(&page.child()) {
                    term.search_set_regex(None::<&vte4::Regex>, 0);
                    term.grab_focus();
                }
            }
        });
    }

    // Escape in search entry closes the bar
    {
        let search_revealer_ref = search_revealer.clone();
        let tab_view_ref = tab_view.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                search_revealer_ref.set_reveal_child(false);
                if let Some(page) = tab_view_ref.selected_page() {
                    if let Some(term) = find_vte_terminal(&page.child()) {
                        term.search_set_regex(None::<&vte4::Regex>, 0);
                        term.grab_focus();
                    }
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        find_entry.add_controller(key_controller);
    }

    // --- Editor search bar wiring ---

    // Helper: ensure search context exists for the active editor buffer
    let ensure_editor_search_ctx = {
        let editor_search_ctx = editor_search_ctx.clone();
        let editor_search_settings = editor_search_settings.clone();
        let tab_view = tab_view.clone();
        move || -> Option<sourceview5::SearchContext> {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if let Some(buf) = editor::get_editor_buffer(&child) {
                    let mut ctx_ref = editor_search_ctx.borrow_mut();
                    // Re-create context if buffer changed
                    let needs_new = ctx_ref
                        .as_ref()
                        .map(|ctx| ctx.buffer() != buf)
                        .unwrap_or(true);
                    if needs_new {
                        let ctx =
                            sourceview5::SearchContext::new(&buf, Some(&editor_search_settings));
                        ctx.set_highlight(true);
                        *ctx_ref = Some(ctx);
                    }
                    return ctx_ref.clone();
                }
            }
            None
        }
    };

    // Helper: update match count label
    let update_match_label = {
        let editor_match_label = editor_match_label.clone();
        let editor_search_ctx = editor_search_ctx.clone();
        let tab_view = tab_view.clone();
        move || {
            let ctx_ref = editor_search_ctx.borrow();
            if let Some(ctx) = ctx_ref.as_ref() {
                let total = ctx.occurrences_count();
                if total < 0 {
                    // Still counting
                    editor_match_label.set_text("...");
                } else if total == 0 {
                    editor_match_label.set_text("No results");
                } else {
                    // Try to get current position from selection
                    if let Some(page) = tab_view.selected_page() {
                        if let Some(buf) = editor::get_editor_buffer(&page.child()) {
                            let (sel_start, sel_end) =
                                buf.selection_bounds().unwrap_or_else(|| {
                                    let iter = buf.iter_at_mark(&buf.get_insert());
                                    (iter, iter)
                                });
                            let pos = ctx.occurrence_position(&sel_start, &sel_end);
                            if pos > 0 {
                                editor_match_label.set_text(&format!("{} of {}", pos, total));
                            } else {
                                editor_match_label.set_text(&format!("{} matches", total));
                            }
                        }
                    }
                }
            } else {
                editor_match_label.set_text("");
            }
        }
    };

    // Search entry text changed -> update search text and find first match
    {
        let ensure_ctx = ensure_editor_search_ctx.clone();
        let update_label = update_match_label.clone();
        let editor_search_settings = editor_search_settings.clone();
        let tab_view = tab_view.clone();
        editor_find_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string();
            if text.is_empty() {
                editor_search_settings.set_search_text(None);
            } else {
                editor_search_settings.set_search_text(Some(&text));
            }

            if let Some(ctx) = ensure_ctx() {
                if !text.is_empty() {
                    if let Some(page) = tab_view.selected_page() {
                        if let Some(buf) = editor::get_editor_buffer(&page.child()) {
                            let iter = buf.iter_at_mark(&buf.get_insert());
                            if let Some((start, end, _wrapped)) = ctx.forward(&iter) {
                                buf.select_range(&start, &end);
                                // Scroll to the match
                                if let Some(view) = editor::get_editor_view(&page.child()) {
                                    view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                                }
                            }
                        }
                    }
                }
                update_label();
            }
        });
    }

    // Find next button
    {
        let ensure_ctx = ensure_editor_search_ctx.clone();
        let update_label = update_match_label.clone();
        let tab_view = tab_view.clone();
        editor_find_next.connect_clicked(move |_| {
            if let Some(ctx) = ensure_ctx() {
                if let Some(page) = tab_view.selected_page() {
                    if let Some(buf) = editor::get_editor_buffer(&page.child()) {
                        // Start searching from end of current selection
                        let iter = if let Some((_start, end)) = buf.selection_bounds() {
                            end
                        } else {
                            buf.iter_at_mark(&buf.get_insert())
                        };
                        if let Some((start, end, _wrapped)) = ctx.forward(&iter) {
                            buf.select_range(&start, &end);
                            if let Some(view) = editor::get_editor_view(&page.child()) {
                                view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                            }
                        }
                        update_label();
                    }
                }
            }
        });
    }

    // Find previous button
    {
        let ensure_ctx = ensure_editor_search_ctx.clone();
        let update_label = update_match_label.clone();
        let tab_view = tab_view.clone();
        editor_find_prev.connect_clicked(move |_| {
            if let Some(ctx) = ensure_ctx() {
                if let Some(page) = tab_view.selected_page() {
                    if let Some(buf) = editor::get_editor_buffer(&page.child()) {
                        // Start searching from start of current selection
                        let iter = if let Some((start, _end)) = buf.selection_bounds() {
                            start
                        } else {
                            buf.iter_at_mark(&buf.get_insert())
                        };
                        if let Some((start, end, _wrapped)) = ctx.backward(&iter) {
                            buf.select_range(&start, &end);
                            if let Some(view) = editor::get_editor_view(&page.child()) {
                                view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                            }
                        }
                        update_label();
                    }
                }
            }
        });
    }

    // Replace button
    {
        let ensure_ctx = ensure_editor_search_ctx.clone();
        let update_label = update_match_label.clone();
        let editor_replace_entry = editor_replace_entry.clone();
        let tab_view = tab_view.clone();
        replace_btn.connect_clicked(move |_| {
            if let Some(ctx) = ensure_ctx() {
                let replace_text = editor_replace_entry.text().to_string();
                if let Some(page) = tab_view.selected_page() {
                    if let Some(buf) = editor::get_editor_buffer(&page.child()) {
                        if let Some((mut sel_start, mut sel_end)) = buf.selection_bounds() {
                            // Replace current match
                            let _ = ctx.replace(&mut sel_start, &mut sel_end, &replace_text);
                            // Move to next match
                            let iter = buf.iter_at_mark(&buf.get_insert());
                            if let Some((start, end, _wrapped)) = ctx.forward(&iter) {
                                buf.select_range(&start, &end);
                                if let Some(view) = editor::get_editor_view(&page.child()) {
                                    view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                                }
                            }
                        }
                        update_label();
                    }
                }
            }
        });
    }

    // Replace all button
    {
        let ensure_ctx = ensure_editor_search_ctx.clone();
        let update_label = update_match_label.clone();
        let editor_replace_entry = editor_replace_entry.clone();
        replace_all_btn.connect_clicked(move |_| {
            if let Some(ctx) = ensure_ctx() {
                let replace_text = editor_replace_entry.text().to_string();
                let _ = ctx.replace_all(&replace_text);
                update_label();
            }
        });
    }

    // Editor search close button
    {
        let editor_search_revealer = editor_search_revealer.clone();
        let editor_search_ctx = editor_search_ctx.clone();
        let editor_search_settings = editor_search_settings.clone();
        let tab_view = tab_view.clone();
        editor_find_close.connect_clicked(move |_| {
            editor_search_revealer.set_reveal_child(false);
            editor_search_settings.set_search_text(None);
            *editor_search_ctx.borrow_mut() = None;
            // Return focus to editor
            if let Some(page) = tab_view.selected_page() {
                page.child().grab_focus();
            }
        });
    }

    // Escape in editor find entry closes the bar
    {
        let editor_search_revealer_ref = editor_search_revealer.clone();
        let editor_search_ctx = editor_search_ctx.clone();
        let editor_search_settings = editor_search_settings.clone();
        let tab_view_ref = tab_view.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                editor_search_revealer_ref.set_reveal_child(false);
                editor_search_settings.set_search_text(None);
                *editor_search_ctx.borrow_mut() = None;
                if let Some(page) = tab_view_ref.selected_page() {
                    page.child().grab_focus();
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        editor_find_entry.add_controller(key_controller);
    }

    // Escape in editor replace entry closes the bar
    {
        let editor_search_revealer_ref = editor_search_revealer.clone();
        let editor_search_ctx = editor_search_ctx.clone();
        let editor_search_settings = editor_search_settings.clone();
        let tab_view_ref = tab_view.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                editor_search_revealer_ref.set_reveal_child(false);
                editor_search_settings.set_search_text(None);
                *editor_search_ctx.borrow_mut() = None;
                if let Some(page) = tab_view_ref.selected_page() {
                    page.child().grab_focus();
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        editor_replace_entry.add_controller(key_controller);
    }

    // Hide editor search bar when switching to a terminal tab
    {
        let editor_search_revealer = editor_search_revealer.clone();
        let search_revealer = search_revealer.clone();
        let editor_search_ctx = editor_search_ctx.clone();
        let editor_search_settings = editor_search_settings.clone();
        tab_view.connect_selected_page_notify(move |tv| {
            if let Some(page) = tv.selected_page() {
                let child = page.child();
                if !editor::is_editor(&child) {
                    // Switching to a non-editor tab: hide editor search
                    if editor_search_revealer.reveals_child() {
                        editor_search_revealer.set_reveal_child(false);
                        editor_search_settings.set_search_text(None);
                        *editor_search_ctx.borrow_mut() = None;
                    }
                } else {
                    // Switching to an editor tab: hide terminal search
                    if search_revealer.reveals_child() {
                        search_revealer.set_reveal_child(false);
                    }
                }
            }
        });
    }

    // Ctrl+G: Go to line (editor tabs only)
    {
        let tab_view = tab_view.clone();
        let window_ref = window.clone();
        add_shortcut(&shortcut_controller, "<Ctrl>g", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    show_go_to_line_dialog(&window_ref, &child);
                }
            }
        });
    }

    // F12: Go to definition (LSP)
    {
        let tab_view = tab_view.clone();
        let lsp_tx = lsp_request_tx.clone();
        add_shortcut(&shortcut_controller, "F12", move || {
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
                    let path = child.widget_name().to_string();
                    if let Some(buf) = editor::get_editor_buffer(&child) {
                        let insert_mark = buf.get_insert();
                        let iter = buf.iter_at_mark(&insert_mark);
                        let line = iter.line() as u32;
                        let character = iter.line_offset() as u32;
                        let _ = lsp_tx.send(LspRequest::Definition {
                            uri: format!("file://{}", path),
                            line,
                            character,
                        });
                    }
                }
            }
        });
    }

    // Ctrl+Shift+N: New window
    {
        let app_clone = app.clone();
        add_shortcut(&shortcut_controller, "<Ctrl><Shift>n", move || {
            build_window(&app_clone);
        });
    }

    // F11: Toggle fullscreen
    {
        let window_ref = window.clone();
        add_shortcut(&shortcut_controller, "F11", move || {
            if window_ref.is_fullscreen() {
                window_ref.unfullscreen();
            } else {
                window_ref.fullscreen();
            }
        });
    }

    // Register custom keybindings from settings
    {
        let custom_keybindings = settings.borrow().custom_keybindings.clone();
        for kb in custom_keybindings {
            let accel = parse_keybinding_to_accel(&kb.key);
            if accel.is_empty() {
                log::warn!("Invalid keybinding: {}", kb.key);
                continue;
            }
            let command = kb.command.clone();
            let args = kb.args.clone();
            let kb_name = kb.name.clone();
            let tab_view = tab_view.clone();
            add_shortcut(&shortcut_controller, &accel, move || {
                // Get current file path from active editor if available
                let file_path = tab_view.selected_page().and_then(|page| {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        Some(child.widget_name().to_string())
                    } else {
                        None
                    }
                });

                let mut cmd = std::process::Command::new(&command);
                cmd.args(&args);
                if let Some(ref fp) = file_path {
                    cmd.env("IMPULSE_FILE", fp);
                }
                let command_name = kb_name.clone();
                std::thread::spawn(move || match cmd.output() {
                    Ok(output) => {
                        if !output.status.success() {
                            log::warn!(
                                "Custom command '{}' failed: {}",
                                command_name,
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to run custom command '{}': {}", command_name, e)
                    }
                });
            });
        }
    }

    window.add_controller(shortcut_controller);

    // Set the initial active tab for tree state tracking
    if let Some(page) = tab_view.selected_page() {
        sidebar_state.set_active_tab(&page.child());
    }

    // Focus terminal or editor when tab changes
    {
        let status_bar = status_bar.clone();
        let sidebar_state = sidebar_state.clone();
        tab_view.connect_selected_page_notify(move |tv| {
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
                        let path = if let Some(rest) = uri_str.strip_prefix("file://") {
                            if let Some(slash_idx) = rest.find('/') {
                                &rest[slash_idx..]
                            } else {
                                rest
                            }
                        } else {
                            &uri_str
                        };
                        let path = url_decode(path);
                        status_bar.borrow().update_cwd(&path);
                        sidebar_state.switch_to_tab(&child, &path);
                    } else {
                        // New terminal without CWD yet — just set active tab
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
                    // Show cursor position for editor tabs
                    if let Some(buf) = editor::get_editor_buffer(&child) {
                        let insert_mark = buf.get_insert();
                        let iter = buf.iter_at_mark(&insert_mark);
                        let line = iter.line();
                        let col = iter.line_offset();
                        status_bar.borrow().update_cursor_position(line, col);
                    }
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
    }

    // Close tab_view pages: check for unsaved editor changes before closing.
    // Also close the window when the last tab is closed.
    {
        let window_ref = window.clone();
        let sidebar_state = sidebar_state.clone();
        let lsp_tx = lsp_request_tx.clone();
        tab_view.connect_close_page(move |tv, page| {
            sidebar_state.remove_tab_state(&page.child());
            let child = page.child();

            // LSP: send didClose for editor tabs
            if editor::is_editor(&child) {
                let path = child.widget_name().to_string();
                let _ = lsp_tx.send(LspRequest::DidClose {
                    uri: format!("file://{}", path),
                });
            }

            // Check if this is an editor tab with unsaved changes
            if editor::is_editor(&child) {
                if let Some(buf) = editor::get_editor_buffer(&child) {
                    if buf.is_modified() {
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
                        dialog.set_response_appearance(
                            "discard",
                            adw::ResponseAppearance::Destructive,
                        );
                        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                        dialog.set_default_response(Some("save"));
                        dialog.set_close_response("cancel");

                        let tv = tv.clone();
                        let page = page.clone();
                        let child = child.clone();
                        let window = window_ref.clone();
                        dialog.connect_response(None, move |_dialog, response| {
                            match response {
                                "save" => {
                                    // Save then close
                                    let path = child.widget_name().to_string();
                                    if let Some(text) = editor::get_editor_text(&child) {
                                        let _ = std::fs::write(&path, &text);
                                    }
                                    tv.close_page_finish(&page, true);
                                    let tv2 = tv.clone();
                                    let window2 = window.clone();
                                    gtk4::glib::idle_add_local_once(move || {
                                        if tv2.n_pages() == 0 {
                                            window2.close();
                                        }
                                    });
                                }
                                "discard" => {
                                    tv.close_page_finish(&page, true);
                                    let tv2 = tv.clone();
                                    let window2 = window.clone();
                                    gtk4::glib::idle_add_local_once(move || {
                                        if tv2.n_pages() == 0 {
                                            window2.close();
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
                }
            }

            // Terminal tab or unmodified editor: close immediately
            tv.close_page_finish(page, true);
            let tv = tv.clone();
            let window = window_ref.clone();
            gtk4::glib::idle_add_local_once(move || {
                if tv.n_pages() == 0 {
                    window.close();
                }
            });
            gtk4::glib::Propagation::Stop
        });
    }

    // Save settings when window is closed
    {
        let paned = paned.clone();
        let sidebar_btn = sidebar_btn.clone();
        let sidebar_state = sidebar_state.clone();
        let tab_view_ref = tab_view.clone();
        let font_size = font_size.clone();
        let settings = settings.clone();
        let lsp_tx = lsp_request_tx.clone();
        window.connect_close_request(move |window| {
            // Shutdown LSP servers
            let _ = lsp_tx.send(LspRequest::Shutdown);
            // Collect open editor file paths
            let mut open_files = Vec::new();
            let n = tab_view_ref.n_pages();
            for i in 0..n {
                let page = tab_view_ref.nth_page(i);
                let child = page.child();
                if editor::is_editor(&child) {
                    let path = child.widget_name().to_string();
                    if !path.is_empty() && path != "GtkBox" {
                        open_files.push(path);
                    }
                }
            }

            // Merge window state into existing settings (preserve all user prefs)
            {
                let mut s = settings.borrow_mut();
                s.window_width = window.width();
                s.window_height = window.height();
                s.sidebar_visible = sidebar_btn.is_active();
                s.sidebar_width = paned.position();
                s.last_directory = sidebar_state.current_path.borrow().clone();
                s.font_size = font_size.get();
                s.open_files = open_files;
            }
            crate::settings::save(&settings.borrow());
            gtk4::glib::Propagation::Proceed
        });
    }

    window.present();
}

fn apply_font_size_to_all_terminals(tab_view: &adw::TabView, size: i32) {
    let n = tab_view.n_pages();
    for i in 0..n {
        let page = tab_view.nth_page(i);
        for term in terminal_container::collect_terminals(&page.child()) {
            let mut font_desc = gtk4::pango::FontDescription::from_string("Monospace");
            font_desc.set_size(size * 1024);
            term.set_font_desc(Some(&font_desc));
        }
    }
}

fn run_commands_on_save(path: &str, commands: &[crate::settings::CommandOnSave]) {
    for cmd in commands {
        if matches_file_pattern(path, &cmd.file_pattern) {
            let mut command = std::process::Command::new(&cmd.command);
            command.args(&cmd.args);
            command.arg(path);
            match command.output() {
                Ok(output) => {
                    if !output.status.success() {
                        log::warn!(
                            "Command '{}' failed for {}: {}",
                            cmd.name,
                            path,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    } else {
                        log::info!("Command '{}' succeeded for {}", cmd.name, path);
                    }
                }
                Err(e) => log::warn!("Failed to run command '{}': {}", cmd.name, e),
            }
        }
    }
}

fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(ext_pattern) = pattern.strip_prefix("*.") {
        if let Some(ext) = std::path::Path::new(path).extension() {
            return ext.to_string_lossy().eq_ignore_ascii_case(ext_pattern);
        }
        return false;
    }
    // Exact match
    path.ends_with(pattern)
}

fn parse_keybinding_to_accel(key: &str) -> String {
    let parts: Vec<&str> = key.split('+').collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut accel = String::new();
    for part in &parts[..parts.len() - 1] {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => accel.push_str("<Ctrl>"),
            "shift" => accel.push_str("<Shift>"),
            "alt" => accel.push_str("<Alt>"),
            "super" => accel.push_str("<Super>"),
            _ => return String::new(),
        }
    }
    accel.push_str(parts.last().unwrap().trim());
    accel
}

fn add_shortcut(controller: &gtk4::ShortcutController, accel: &str, callback: impl Fn() + 'static) {
    let trigger = gtk4::ShortcutTrigger::parse_string(accel);
    let action = gtk4::CallbackAction::new(move |_widget, _args| {
        callback();
        gtk4::glib::Propagation::Stop
    });
    if let Some(trigger) = trigger {
        let shortcut = gtk4::Shortcut::new(Some(trigger), Some(action));
        controller.add_shortcut(shortcut);
    }
}

fn show_quick_open(window: &adw::ApplicationWindow, sidebar_state: &Rc<sidebar::SidebarState>) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(500)
        .default_height(400)
        .build();
    dialog.add_css_class("quick-open");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Open file..."));
    vbox.append(&entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    scroll.set_child(Some(&list));
    vbox.append(&scroll);

    dialog.set_child(Some(&vbox));

    // Search on type
    let current_path = sidebar_state.current_path.clone();
    {
        let list = list.clone();
        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            let root = current_path.borrow().clone();
            if query.is_empty() || root.is_empty() {
                while let Some(row) = list.row_at_index(0) {
                    list.remove(&row);
                }
                return;
            }
            let list = list.clone();
            gtk4::glib::spawn_future_local(async move {
                let results = gtk4::gio::spawn_blocking(move || {
                    impulse_core::search::search_filenames(&root, &query, 30)
                })
                .await;
                while let Some(row) = list.row_at_index(0) {
                    list.remove(&row);
                }
                if let Ok(Ok(results)) = results {
                    for result in &results {
                        let label = gtk4::Label::new(Some(&result.path));
                        label.set_halign(gtk4::Align::Start);
                        label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
                        list.append(&label);
                    }
                }
            });
        });
    }

    // Escape to close
    let key_controller = gtk4::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(key_controller);

    dialog.present();
    entry.grab_focus();
}

fn show_command_palette(window: &adw::ApplicationWindow, commands: &[Command]) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(500)
        .default_height(400)
        .build();
    dialog.add_css_class("quick-open");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Type a command..."));
    vbox.append(&entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    scroll.set_child(Some(&list));
    vbox.append(&scroll);

    dialog.set_child(Some(&vbox));

    // Populate with all commands
    let commands: Vec<Command> = commands.to_vec();
    populate_command_list(&list, &commands, "");

    // Filter on type
    {
        let list = list.clone();
        let commands = commands.clone();
        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string().to_lowercase();
            populate_command_list(&list, &commands, &query);
        });
    }

    // Activate command on row click
    {
        let dialog = dialog.clone();
        let commands = commands.clone();
        list.connect_row_activated(move |_list, row| {
            if let Some(child) = row.child() {
                let cmd_idx = child
                    .widget_name()
                    .to_string()
                    .parse::<usize>()
                    .unwrap_or(0);
                if cmd_idx < commands.len() {
                    (commands[cmd_idx].action)();
                }
            }
            dialog.close();
        });
    }

    // Enter key activates selected row
    {
        let list = list.clone();
        let dialog = dialog.clone();
        let commands = commands.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                if let Some(row) = list.selected_row() {
                    if let Some(child) = row.child() {
                        let cmd_idx = child
                            .widget_name()
                            .to_string()
                            .parse::<usize>()
                            .unwrap_or(0);
                        if cmd_idx < commands.len() {
                            (commands[cmd_idx].action)();
                        }
                    }
                    dialog.close();
                    return gtk4::glib::Propagation::Stop;
                }
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_controller);
    }

    dialog.present();
    entry.grab_focus();
}

fn populate_command_list(list: &gtk4::ListBox, commands: &[Command], filter: &str) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    for (idx, cmd) in commands.iter().enumerate() {
        if !filter.is_empty() && !cmd.name.to_lowercase().contains(filter) {
            continue;
        }
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_widget_name(&idx.to_string());
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(4);
        row.set_margin_bottom(4);

        let name_label = gtk4::Label::new(Some(&cmd.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        row.append(&name_label);

        if !cmd.shortcut.is_empty() {
            let shortcut_label = gtk4::Label::new(Some(&cmd.shortcut));
            shortcut_label.add_css_class("dim-label");
            row.append(&shortcut_label);
        }

        list.append(&row);
    }

    // Select first row by default
    if let Some(first_row) = list.row_at_index(0) {
        list.select_row(Some(&first_row));
    }
}

fn show_go_to_line_dialog(window: &adw::ApplicationWindow, editor_widget: &gtk4::Widget) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(300)
        .default_height(60)
        .build();
    dialog.add_css_class("quick-open"); // reuse quick-open styling

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(12);
    hbox.set_margin_bottom(12);

    let label = gtk4::Label::new(Some("Go to line:"));
    let entry = gtk4::Entry::new();
    entry.set_hexpand(true);
    entry.set_input_purpose(gtk4::InputPurpose::Digits);

    hbox.append(&label);
    hbox.append(&entry);
    dialog.set_child(Some(&hbox));

    // Get total line count for placeholder
    if let Some(buf) = editor::get_editor_buffer(editor_widget) {
        let total = buf.line_count();
        entry.set_placeholder_text(Some(&format!("1-{}", total)));
    }

    // Enter to go to line
    let editor_widget = editor_widget.clone();
    {
        let dialog = dialog.clone();
        entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if let Ok(line_num) = text.trim().parse::<i32>() {
                let line = (line_num - 1).max(0); // 1-indexed to 0-indexed
                if let Some(buf) = editor::get_editor_buffer(&editor_widget) {
                    if let Some(iter) = buf.iter_at_line(line) {
                        buf.place_cursor(&iter);
                        // Scroll to the line
                        if let Some(view) = editor::get_editor_view(&editor_widget) {
                            view.scroll_to_iter(&mut iter.clone(), 0.1, true, 0.0, 0.5);
                        }
                    }
                }
            }
            dialog.close();
        });
    }

    // Escape to close
    let key_controller = gtk4::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(key_controller);

    dialog.present();
    entry.grab_focus();
}

fn find_vte_terminal(widget: &gtk4::Widget) -> Option<vte4::Terminal> {
    if let Some(term) = widget.downcast_ref::<vte4::Terminal>() {
        return Some(term.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(term) = find_vte_terminal(&c) {
            return Some(term);
        }
        child = c.next_sibling();
    }
    None
}

fn regex_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        match c {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Convert a file:// URI to a local file path.
fn uri_to_file_path(uri: &str) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        url_decode(path)
    } else {
        uri.to_string()
    }
}

/// Determine LSP language ID from a file URI based on extension.
fn language_from_uri(uri: &str) -> String {
    let path = uri_to_file_path(uri);
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust".to_string(),
        "py" | "pyi" => "python".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "javascript".to_string(),
        "ts" | "tsx" => "typescript".to_string(),
        "c" | "h" => "c".to_string(),
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp".to_string(),
        "go" => "go".to_string(),
        "java" => "java".to_string(),
        "rb" => "ruby".to_string(),
        "lua" => "lua".to_string(),
        "zig" => "zig".to_string(),
        _ => ext,
    }
}

fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                } else {
                    result.push('%');
                    result.push_str(&hex);
                }
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(c);
        }
    }
    result
}
