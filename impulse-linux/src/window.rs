use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use vte4::prelude::*;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use url::Url;

use crate::editor;
use crate::keybindings;
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
    // Pre-warm a WebView with Monaco so the first editor tab opens instantly.
    crate::editor_webview::warm_up_editor();

    let settings = Rc::new(RefCell::new(crate::settings::load()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Impulse")
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .build();

    // Shared font size state (user-facing size in points, e.g. 11)
    let font_size: Rc<Cell<i32>> = Rc::new(Cell::new(settings.borrow().font_size));

    // Shared copy-on-select flag checked by terminal selection-changed signal handlers
    let copy_on_select_flag: Rc<Cell<bool>> =
        Rc::new(Cell::new(settings.borrow().terminal_copy_on_select));

    // Pre-compute shell spawn parameters once (shell path, env vars, temp files)
    let shell_cache = Rc::new(terminal::ShellSpawnCache::new());

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
        let root_uri = file_path_to_uri(std::path::Path::new(&initial_dir))
            .unwrap_or_else(|| "file:///".to_string());
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
                let registry_for_exit = registry.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        let response = match event {
                            impulse_core::lsp::LspEvent::Diagnostics {
                                uri,
                                version,
                                diagnostics,
                            } => {
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
                                    version,
                                    diagnostics: diags,
                                }
                            }
                            impulse_core::lsp::LspEvent::Initialized {
                                client_key,
                                server_id,
                            } => LspResponse::ServerInitialized {
                                client_key,
                                server_id,
                            },
                            impulse_core::lsp::LspEvent::ServerError {
                                client_key,
                                server_id,
                                message,
                            } => LspResponse::ServerError {
                                client_key,
                                server_id,
                                message,
                            },
                            impulse_core::lsp::LspEvent::ServerExited {
                                client_key,
                                server_id,
                            } => {
                                registry_for_exit.remove_client(&client_key).await;
                                LspResponse::ServerExited {
                                    client_key,
                                    server_id,
                                }
                            }
                        };
                        if gtk_tx_events.send(response).is_err() {
                            break;
                        }
                    }
                });

                // Main request processing loop.
                // Requests are processed in-order to keep didOpen/didChange/completion
                // sequencing deterministic per document.
                let gtk_tx_req = gtk_tx;
                while let Some(request) = lsp_request_rx.recv().await {
                    let gtk_tx = gtk_tx_req.clone();
                    match request {
                        LspRequest::DidOpen {
                            uri,
                            language_id,
                            version,
                            text,
                        } => {
                            let clients = registry.get_clients(&language_id, &uri).await;
                            for client in clients {
                                let _ = client.did_open(&uri, &language_id, version, &text);
                            }
                        }
                        LspRequest::DidChange { uri, version, text } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_change(&uri, version, &text);
                            }
                        }
                        LspRequest::DidSave { uri } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_save(&uri);
                            }
                        }
                        LspRequest::DidClose { uri } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                let _ = client.did_close(&uri);
                            }
                        }
                        LspRequest::Completion {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            let mut seen = std::collections::HashSet::<String>::new();
                            let mut completions = Vec::new();
                            for client in clients {
                                if let Ok(items) = client.completion(&uri, line, character).await {
                                    for item in items {
                                        let dedupe_key = format!(
                                            "{}|{}|{}",
                                            item.label,
                                            item.detail.clone().unwrap_or_default(),
                                            item.insert_text.clone().unwrap_or_default()
                                        );
                                        if seen.insert(dedupe_key) {
                                            completions.push(completion_item_to_info(item));
                                        }
                                    }
                                }
                            }
                            let _ = gtk_tx.send(LspResponse::CompletionResult {
                                request_id,
                                uri,
                                version,
                                items: completions,
                            });
                        }
                        LspRequest::Hover {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(Some(hover)) = client.hover(&uri, line, character).await {
                                    let content = crate::lsp_hover::hover_content_to_string(&hover);
                                    let _ = gtk_tx.send(LspResponse::HoverResult {
                                        request_id,
                                        uri: uri.clone(),
                                        version,
                                        contents: content,
                                    });
                                    break;
                                }
                            }
                        }
                        LspRequest::Definition {
                            request_id,
                            uri,
                            version,
                            line,
                            character,
                        } => {
                            let lang = language_from_uri(&uri);
                            let clients = registry.get_clients(&lang, &uri).await;
                            for client in clients {
                                if let Ok(Some(def)) = client.definition(&uri, line, character).await {
                                    let location = match def {
                                        lsp_types::GotoDefinitionResponse::Scalar(loc) => Some(loc),
                                        lsp_types::GotoDefinitionResponse::Array(locs) => {
                                            locs.into_iter().next()
                                        }
                                        lsp_types::GotoDefinitionResponse::Link(links) => links
                                            .into_iter()
                                            .next()
                                            .map(|l| lsp_types::Location {
                                                uri: l.target_uri,
                                                range: l.target_selection_range,
                                            }),
                                    };
                                    if let Some(loc) = location {
                                        let _ = gtk_tx.send(LspResponse::DefinitionResult {
                                            request_id,
                                            source_uri: uri.clone(),
                                            source_version: version,
                                            uri: loc.uri.to_string(),
                                            line: loc.range.start.line,
                                            character: loc.range.start.character,
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                        LspRequest::Shutdown => {
                            registry.shutdown_all().await;
                        }
                    }
                }
            });
        });
    }

    // Shared document version counter for LSP
    let lsp_doc_versions: Rc<RefCell<std::collections::HashMap<String, i32>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let lsp_request_seq: Rc<Cell<u64>> = Rc::new(Cell::new(1));
    let latest_completion_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_hover_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let latest_definition_req: Rc<RefCell<std::collections::HashMap<String, u64>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let lsp_error_toast_dedupe: Rc<RefCell<HashSet<String>>> =
        Rc::new(RefCell::new(HashSet::new()));
    let (lsp_install_result_tx, lsp_install_result_rx) =
        std::sync::mpsc::channel::<Result<String, String>>();
    let lsp_install_result_rx = Rc::new(RefCell::new(lsp_install_result_rx));

    // Track the current CSS provider so we can swap themes at runtime
    let css_provider: Rc<RefCell<gtk4::CssProvider>> = {
        let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
        Rc::new(RefCell::new(crate::theme::load_css(theme)))
    };

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
        .tooltip_text("Toggle Sidebar (Ctrl+Shift+B)")
        .active(settings.borrow().sidebar_visible)
        .build();
    sidebar_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&sidebar_btn);

    // New tab button
    let new_tab_btn = gtk4::Button::new();
    new_tab_btn.set_tooltip_text(Some("New Tab (Ctrl+T)"));
    new_tab_btn.set_cursor_from_name(Some("pointer"));
    header.pack_end(&new_tab_btn);

    // Settings button (right side of header, click handler wired below after tab_view setup)
    let settings_btn = gtk4::Button::new();
    settings_btn.set_tooltip_text(Some("Settings"));
    settings_btn.set_cursor_from_name(Some("pointer"));
    header.pack_start(&settings_btn);

    main_box.append(&header);

    // Horizontal pane: sidebar + tab view
    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_position(settings.borrow().sidebar_width);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Sidebar
    let initial_theme = crate::theme::get_theme(&settings.borrow().color_scheme);
    let (sidebar_widget, sidebar_state) = sidebar::build_sidebar(&settings, initial_theme);
    sidebar_widget.set_visible(settings.borrow().sidebar_visible);
    paned.set_start_child(Some(&sidebar_widget));

    // Set header button icons from shared SVG icon cache
    {
        let cache = sidebar_state.icon_cache.borrow();
        if let Some(t) = cache.get_toolbar_icon("toolbar-sidebar") {
            sidebar_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
        if let Some(t) = cache.get_toolbar_icon("toolbar-plus") {
            new_tab_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
        if let Some(t) = cache.get_toolbar_icon("settings") {
            settings_btn.set_child(Some(&gtk4::Image::from_paintable(Some(t))));
        }
    }

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

    // Editor search/replace is handled by Monaco's built-in Ctrl+F/Ctrl+H.

    // Tab view in the end pane, wrapped with terminal search bar above
    let right_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    right_box.append(&search_revealer);
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

    // Initialize project search root to current directory
    *sidebar_state.project_search.current_root.borrow_mut() = initial_dir.clone();

    // Shared state
    let sidebar_state = Rc::new(sidebar_state);

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
        let icon_cache = sidebar_state.icon_cache.clone();
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
                            let sidebar_state = sidebar_state_for_editor.clone();
                            let path = path.to_string();
                            move |handle, event| {
                                match event {
                                    impulse_editor::protocol::EditorEvent::Ready => {
                                        // No-op: initialization now happens on FileOpened
                                    }
                                    impulse_editor::protocol::EditorEvent::FileOpened => {
                                        // Send LSP didOpen
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let language_id = language_from_uri(&uri);
                                        let content = handle.get_content();
                                        let mut versions = doc_versions.borrow_mut();
                                        let version = versions.entry(path.clone()).or_insert(0);
                                        *version += 1;
                                        let _ = lsp_tx.send(LspRequest::DidOpen {
                                            uri,
                                            language_id,
                                            version: *version,
                                            text: content,
                                        });
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
                                        let _ = lsp_tx.send(LspRequest::DidChange {
                                            uri,
                                            version: *version,
                                            text: content,
                                        });
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
                                            let _ = lsp_tx.send(LspRequest::DidSave { uri });
                                            // Refresh diff decorations after save
                                            send_diff_decorations(handle, &path);
                                            // Refresh sidebar to update git status badges
                                            sidebar_state.refresh();
                                            // Run commands-on-save in a background thread
                                            let commands = settings.borrow().commands_on_save.clone();
                                            let save_path = path.clone();
                                            std::thread::spawn(move || {
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
                                        let _ = lsp_tx.send(LspRequest::Completion {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        });
                                    }
                                    impulse_editor::protocol::EditorEvent::HoverRequested { request_id: _, line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_hover_req.borrow_mut().insert(path.clone(), seq);
                                        let _ = lsp_tx.send(LspRequest::Hover {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        });
                                    }
                                    impulse_editor::protocol::EditorEvent::DefinitionRequested { line, character } => {
                                        let uri = file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path));
                                        let version = doc_versions.borrow().get(&path).copied().unwrap_or(1);
                                        let seq = lsp_request_seq.get() + 1;
                                        lsp_request_seq.set(seq);
                                        latest_definition_req.borrow_mut().insert(path.clone(), seq);
                                        let _ = lsp_tx.send(LspRequest::Definition {
                                            request_id: seq,
                                            uri,
                                            version,
                                            line,
                                            character,
                                        });
                                    }
                                    _ => {}
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

    // --- Helper: connect signals on a terminal (CWD change + child-exited) ---
    let setup_terminal_signals = {
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
    };

    // --- Wire up tab creation ---
    let create_tab = {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        let copy_on_select_flag = copy_on_select_flag.clone();
        let shell_cache = shell_cache.clone();
        let icon_cache = sidebar_state.icon_cache.clone();
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

    // Capture-phase key handler so shortcuts work even when VTE, WebView, or
    // the sidebar has focus (those widgets consume keys before the bubble-phase
    // ShortcutController can see them).
    {
        let tab_view = tab_view.clone();
        let create_tab_capture = create_tab.clone();
        let sidebar_btn_capture = sidebar_btn.clone();

        // Build parsed accels + callbacks for custom keybindings so they work
        // even when VTE or WebView has focus (those widgets consume key events
        // before the bubble-phase ShortcutController sees them).
        struct CustomKbAction {
            parsed: keybindings::ParsedAccel,
            action: Rc<dyn Fn()>,
        }
        let mut custom_kb_actions: Vec<CustomKbAction> = Vec::new();
        {
            let custom_keybindings = settings.borrow().custom_keybindings.clone();
            for kb in custom_keybindings {
                let accel = keybindings::parse_keybinding_to_accel(&kb.key);
                if accel.is_empty() {
                    continue;
                }
                if let Some(parsed) = keybindings::parse_accel(&accel) {
                    let command = kb.command.clone();
                    let args = kb.args.clone();
                    let kb_name = kb.name.clone();
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    let copy_on_select_flag = copy_on_select_flag.clone();
                    let icon_cache = sidebar_state.icon_cache.clone();
                    custom_kb_actions.push(CustomKbAction {
                        parsed,
                        action: Rc::new(move || {
                            // Get CWD from the active terminal or editor tab
                            let cwd = get_active_cwd(&tab_view);

                            let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
                            let term = terminal::create_terminal(
                                &settings.borrow(),
                                theme,
                                copy_on_select_flag.clone(),
                            );
                            setup_terminal_signals(&term);
                            terminal::spawn_command(&term, &command, &args, cwd.as_deref());
                            let container = terminal_container::TerminalContainer::new(&term);
                            let page = tab_view.append(&container.widget);
                            page.set_title(&kb_name);
                            if let Some(texture) = icon_cache.borrow().get_toolbar_icon("console") {
                                page.set_icon(Some(texture));
                            }
                            tab_view.set_selected_page(&page);
                            term.grab_focus();
                        }),
                    });
                }
            }
        }

        // Parse split-terminal accels for capture-phase matching (VTE eats
        // these before the Global ShortcutController can see them).
        let capture_kb_overrides = settings.borrow().keybinding_overrides.clone();
        let split_h_accel = keybindings::parse_accel(&keybindings::get_accel(
            "split_horizontal",
            &capture_kb_overrides,
        ));
        let split_v_accel = keybindings::parse_accel(&keybindings::get_accel(
            "split_vertical",
            &capture_kb_overrides,
        ));
        let focus_prev_accel = keybindings::parse_accel(&keybindings::get_accel(
            "focus_prev_split",
            &capture_kb_overrides,
        ));
        let focus_next_accel = keybindings::parse_accel(&keybindings::get_accel(
            "focus_next_split",
            &capture_kb_overrides,
        ));

        let split_setup = setup_terminal_signals.clone();
        let split_settings = settings.clone();
        let split_copy_flag = copy_on_select_flag.clone();
        let split_shell_cache = shell_cache.clone();

        let capture_key_ctrl = gtk4::EventControllerKey::new();
        capture_key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
        capture_key_ctrl.connect_key_pressed(move |_, key, _keycode, modifiers| {
            let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
            let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);

            // Check custom keybindings first (always, regardless of focus)
            for ckb in &custom_kb_actions {
                if keybindings::matches_key(&ckb.parsed, key, modifiers) {
                    (ckb.action)();
                    return gtk4::glib::Propagation::Stop;
                }
            }

            // Ctrl+Shift+B: toggle sidebar (VTE/WebView eat this before
            // the Global ShortcutController can see it)
            if ctrl && shift && (key == gtk4::gdk::Key::b || key == gtk4::gdk::Key::B) {
                sidebar_btn_capture.set_active(!sidebar_btn_capture.is_active());
                return gtk4::glib::Propagation::Stop;
            }

            // Split terminal keybindings (VTE eats Ctrl+Shift+E/O)
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                if terminal_container::get_active_terminal(&child).is_some() {
                    if let Some(ref accel) = split_h_accel {
                        if keybindings::matches_key(accel, key, modifiers) {
                            let setup = split_setup.clone();
                            let s = split_settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Horizontal,
                                &|term| setup(term),
                                &s,
                                theme,
                                split_copy_flag.clone(),
                                &split_shell_cache,
                            );
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                    if let Some(ref accel) = split_v_accel {
                        if keybindings::matches_key(accel, key, modifiers) {
                            let setup = split_setup.clone();
                            let s = split_settings.borrow();
                            let theme = crate::theme::get_theme(&s.color_scheme);
                            terminal_container::split_terminal(
                                &child,
                                gtk4::Orientation::Vertical,
                                &|term| setup(term),
                                &s,
                                theme,
                                split_copy_flag.clone(),
                                &split_shell_cache,
                            );
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                    if let Some(ref accel) = focus_prev_accel {
                        if keybindings::matches_key(accel, key, modifiers) {
                            terminal_container::focus_prev_terminal(&child);
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                    if let Some(ref accel) = focus_next_accel {
                        if keybindings::matches_key(accel, key, modifiers) {
                            terminal_container::focus_next_terminal(&child);
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                }
            }

            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                let is_terminal = terminal_container::get_active_terminal(&child).is_some();
                // Ctrl+Shift+V: paste into terminal
                if ctrl && shift && (key == gtk4::gdk::Key::v || key == gtk4::gdk::Key::V) {
                    if let Some(term) = terminal_container::get_active_terminal(&child) {
                        terminal::paste_from_clipboard(&term);
                        return gtk4::glib::Propagation::Stop;
                    }
                }

                // Ctrl+W: close tab (VTE eats this as "delete word backward")
                if ctrl
                    && !shift
                    && (key == gtk4::gdk::Key::w || key == gtk4::gdk::Key::W)
                    && is_terminal
                {
                    tab_view.close_page(&page);
                    return gtk4::glib::Propagation::Stop;
                }

                // Ctrl+T: new tab (VTE eats this as "transpose chars")
                if ctrl
                    && !shift
                    && (key == gtk4::gdk::Key::t || key == gtk4::gdk::Key::T)
                    && is_terminal
                {
                    create_tab_capture();
                    return gtk4::glib::Propagation::Stop;
                }

                // Ctrl+1-9: switch tab by number (VTE swallows these)
                if ctrl && !shift && is_terminal {
                    let digit = match key {
                        gtk4::gdk::Key::_1 => Some(0),
                        gtk4::gdk::Key::_2 => Some(1),
                        gtk4::gdk::Key::_3 => Some(2),
                        gtk4::gdk::Key::_4 => Some(3),
                        gtk4::gdk::Key::_5 => Some(4),
                        gtk4::gdk::Key::_6 => Some(5),
                        gtk4::gdk::Key::_7 => Some(6),
                        gtk4::gdk::Key::_8 => Some(7),
                        gtk4::gdk::Key::_9 => Some(8),
                        _ => None,
                    };
                    if let Some(idx) = digit {
                        if idx < tab_view.n_pages() {
                            tab_view.set_selected_page(&tab_view.nth_page(idx));
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                }
            }
            gtk4::glib::Propagation::Proceed
        });
        window.add_controller(capture_key_ctrl);
    }

    // --- Keyboard shortcuts ---
    let shortcut_controller = gtk4::ShortcutController::new();
    shortcut_controller.set_scope(gtk4::ShortcutScope::Global);
    let kb_overrides = settings.borrow().keybinding_overrides.clone();

    // Ctrl+T: New tab
    {
        let create_tab = create_tab.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("new_tab", &kb_overrides),
            move || {
                create_tab();
            },
        );
    }

    // Ctrl+W: Close current tab
    {
        let tab_view = tab_view.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("close_tab", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    tab_view.close_page(&page);
                }
            },
        );
    }

    // Ctrl+Shift+B: Toggle sidebar
    {
        let sidebar_btn = sidebar_btn.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("toggle_sidebar", &kb_overrides),
            move || {
                sidebar_btn.set_active(!sidebar_btn.is_active());
            },
        );
    }

    // Shared closure to open settings and apply changes live
    let open_settings: Rc<dyn Fn()> = {
        let window_ref = window.clone();
        let settings = settings.clone();
        let tab_view = tab_view.clone();
        let css_provider = css_provider.clone();
        let copy_on_select_flag = copy_on_select_flag.clone();
        let font_size = font_size.clone();
        let sidebar_state = sidebar_state.clone();
        Rc::new(move || {
            let tab_view = tab_view.clone();
            let css_provider = css_provider.clone();
            let copy_on_select_flag = copy_on_select_flag.clone();
            let font_size = font_size.clone();
            let sidebar_state = sidebar_state.clone();
            crate::settings_page::show_settings_window(&window_ref, &settings, move |s| {
                // Keep the font_size Cell in sync so the close handler
                // doesn't overwrite the user's settings-page changes.
                font_size.set(s.font_size);
                // Swap theme CSS
                let new_theme = crate::theme::get_theme(&s.color_scheme);
                let display = gtk4::gdk::Display::default().expect("No display");
                gtk4::style_context_remove_provider_for_display(&display, &*css_provider.borrow());
                let new_provider = crate::theme::load_css(new_theme);
                *css_provider.borrow_mut() = new_provider;

                // Update sidebar file icons for the new theme
                sidebar_state.update_theme(new_theme);

                // Apply to all open tabs
                for i in 0..tab_view.n_pages() {
                    let page = tab_view.nth_page(i);
                    let child = page.child();
                    if let Some(term) = crate::terminal_container::get_active_terminal(&child) {
                        crate::terminal::apply_settings(&term, s, new_theme, &copy_on_select_flag);
                    } else if crate::editor::is_editor(&child) {
                        crate::editor::apply_settings(child.upcast_ref::<gtk4::Widget>(), s);
                        crate::editor::apply_theme(child.upcast_ref::<gtk4::Widget>(), new_theme);
                    }
                }
            });
        })
    };

    // Wire the settings button
    {
        let open_settings = open_settings.clone();
        settings_btn.connect_clicked(move |_| {
            open_settings();
        });
    }

    // Build command list for the command palette
    let commands = {
        let create_tab = create_tab.clone();
        let tab_view = tab_view.clone();
        let sidebar_btn = sidebar_btn.clone();
        let window_ref = window.clone();
        let sidebar_state = sidebar_state.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let toast_overlay = toast_overlay.clone();
        let lsp_install_result_tx = lsp_install_result_tx.clone();

        vec![
            Command {
                name: "New Terminal Tab".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "new_tab",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let create_tab = create_tab.clone();
                    move || create_tab()
                }),
            },
            Command {
                name: "Close Tab".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "close_tab",
                    &kb_overrides,
                )),
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
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "toggle_sidebar",
                    &kb_overrides,
                )),
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
                name: "Find in Project".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "project_search",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let sidebar_btn = sidebar_btn.clone();
                    let sidebar_state = sidebar_state.clone();
                    move || {
                        // Show sidebar and switch to search tab
                        if !sidebar_btn.is_active() {
                            sidebar_btn.set_active(true);
                        }
                        sidebar_state.search_btn.set_active(true);
                        sidebar_state.project_search.search_entry.grab_focus();
                    }
                }),
            },
            Command {
                name: "Toggle Fullscreen".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "fullscreen",
                    &kb_overrides,
                )),
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
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "new_window",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let app = app.clone();
                    move || build_window(&app)
                }),
            },
            Command {
                name: "Split Terminal Horizontally".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "split_horizontal",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    let copy_on_select_flag = copy_on_select_flag.clone();
                    let shell_cache = shell_cache.clone();
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
                                copy_on_select_flag.clone(),
                                &shell_cache,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Split Terminal Vertically".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "split_vertical",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let tab_view = tab_view.clone();
                    let setup_terminal_signals = setup_terminal_signals.clone();
                    let settings = settings.clone();
                    let copy_on_select_flag = copy_on_select_flag.clone();
                    let shell_cache = shell_cache.clone();
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
                                copy_on_select_flag.clone(),
                                &shell_cache,
                            );
                        }
                    }
                }),
            },
            Command {
                name: "Open Settings".to_string(),
                shortcut: keybindings::accel_to_display(&keybindings::get_accel(
                    "open_settings",
                    &kb_overrides,
                )),
                action: Rc::new({
                    let open_settings = open_settings.clone();
                    move || {
                        open_settings();
                    }
                }),
            },
            Command {
                name: "Install Web LSP Servers".to_string(),
                shortcut: "".to_string(),
                action: Rc::new({
                    let toast_overlay = toast_overlay.clone();
                    let lsp_install_result_tx = lsp_install_result_tx.clone();
                    move || {
                        let start_toast = adw::Toast::new(
                            "Installing web LSP servers (TypeScript, PHP, HTML/CSS, etc.)...",
                        );
                        start_toast.set_timeout(3);
                        toast_overlay.add_toast(start_toast);

                        let tx = lsp_install_result_tx.clone();
                        std::thread::spawn(move || {
                            let result = impulse_core::lsp::install_managed_web_lsp_servers().map(
                                |bin_dir| {
                                    format!(
                                        "Installed managed LSP servers to {}",
                                        bin_dir.display()
                                    )
                                },
                            );
                            let _ = tx.send(result);
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
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("command_palette", &kb_overrides),
            move || {
                show_command_palette(&window_ref, &commands);
            },
        );
    }

    // Ctrl+,: Open Settings
    {
        let open_settings = open_settings.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("open_settings", &kb_overrides),
            move || {
                open_settings();
            },
        );
    }

    // Ctrl+Tab / Ctrl+Shift+Tab: Switch tabs
    {
        let tab_view = tab_view.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("next_tab", &kb_overrides),
            move || {
                let n = tab_view.n_pages();
                if n <= 1 {
                    return;
                }
                if let Some(current) = tab_view.selected_page() {
                    let pos = tab_view.page_position(&current);
                    let next = (pos + 1) % n;
                    tab_view.set_selected_page(&tab_view.nth_page(next));
                }
            },
        );
    }
    {
        let tab_view = tab_view.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("prev_tab", &kb_overrides),
            move || {
                let n = tab_view.n_pages();
                if n <= 1 {
                    return;
                }
                if let Some(current) = tab_view.selected_page() {
                    let pos = tab_view.page_position(&current);
                    let prev = if pos == 0 { n - 1 } else { pos - 1 };
                    tab_view.set_selected_page(&tab_view.nth_page(prev));
                }
            },
        );
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
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("copy", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    if let Some(term) = terminal_container::get_active_terminal(&page.child()) {
                        term.copy_clipboard_format(vte4::Format::Text);
                    }
                }
            },
        );
    }

    // Ctrl+Shift+V paste is handled by the capture-phase EventControllerKey
    // on the window (see above), which runs before VTE's internal handler.

    // Ctrl+Equal / Ctrl+plus: Increase font size
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_increase", &kb_overrides),
            move || {
                let new_size = font_size.get() + 1;
                font_size.set(new_size);
                apply_font_size_to_all_terminals(&tab_view, new_size);
            },
        );
    }

    // Ctrl+minus: Decrease font size
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_decrease", &kb_overrides),
            move || {
                let new_size = font_size.get() - 1;
                if new_size > 0 {
                    font_size.set(new_size);
                    apply_font_size_to_all_terminals(&tab_view, new_size);
                }
            },
        );
    }

    // Ctrl+0: Reset font size to default
    {
        let tab_view = tab_view.clone();
        let font_size = font_size.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_reset", &kb_overrides),
            move || {
                font_size.set(11);
                apply_font_size_to_all_terminals(&tab_view, 11);
            },
        );
    }

    // Ctrl+Shift+F: Project-wide find and replace (open sidebar search tab)
    {
        let sidebar_btn = sidebar_btn.clone();
        let sidebar_state = sidebar_state.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("project_search", &kb_overrides),
            move || {
                // Show sidebar and switch to search tab
                if !sidebar_btn.is_active() {
                    sidebar_btn.set_active(true);
                }
                sidebar_state.search_btn.set_active(true);
                sidebar_state.project_search.search_entry.grab_focus();
            },
        );
    }

    // Ctrl+F: Toggle terminal search bar (Monaco handles Ctrl+F for editor tabs)
    {
        let tab_view = tab_view.clone();
        let search_revealer = search_revealer.clone();
        let find_entry = find_entry.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("find", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    let child = page.child();
                    if !editor::is_editor(&child) {
                        // Terminal tab: toggle terminal search bar
                        let is_visible = search_revealer.reveals_child();
                        search_revealer.set_reveal_child(!is_visible);
                        if !is_visible {
                            find_entry.grab_focus();
                        }
                    }
                    // Editor tabs: Ctrl+F is handled by Monaco's built-in search
                }
            },
        );
    }

    // Ctrl+H: Monaco handles find-and-replace for editor tabs natively

    // Ctrl+S: Save current editor tab
    {
        let tab_view = tab_view.clone();
        let toast_overlay = toast_overlay.clone();
        let lsp_tx = lsp_request_tx.clone();
        let settings = settings.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("save", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        let path = child.widget_name().to_string();
                        if let Some(text) = editor::get_editor_text(&child) {
                            match std::fs::write(&path, &text) {
                                Ok(()) => {
                                    editor::set_unmodified(&child);
                                    // Revert tab title
                                    let filename = std::path::Path::new(&path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(&path);
                                    page.set_title(filename);
                                    // LSP: send didSave
                                    let _ = lsp_tx.send(LspRequest::DidSave {
                                        uri: file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path)),
                                    });
                                    let toast = adw::Toast::new(&format!("Saved {}", filename));
                                    toast.set_timeout(2);
                                    toast_overlay.add_toast(toast);
                                    // Run commands-on-save in a background thread
                                    let commands = settings.borrow().commands_on_save.clone();
                                    let save_path = path.clone();
                                    std::thread::spawn(move || {
                                        let needs_reload =
                                            run_commands_on_save(&save_path, &commands);
                                        if needs_reload {
                                            let reload_path = save_path.clone();
                                            gtk4::glib::MainContext::default().invoke(move || {
                                                if let Some(handle) =
                                                    crate::editor::get_handle(&reload_path)
                                                {
                                                    if let Ok(new_content) =
                                                        std::fs::read_to_string(&reload_path)
                                                    {
                                                        let lang = handle.language.borrow().clone();
                                                        handle.suppress_next_modify.set(true);
                                                        handle.open_file(
                                                            &reload_path,
                                                            &new_content,
                                                            &lang,
                                                        );
                                                        send_diff_decorations(
                                                            &handle,
                                                            &reload_path,
                                                        );
                                                    }
                                                }
                                            });
                                        }
                                    });
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
            },
        );
    }

    // Ctrl+Shift+E: Split terminal horizontally (side by side)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        let copy_on_select_flag = copy_on_select_flag.clone();
        let shell_cache = shell_cache.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("split_horizontal", &kb_overrides),
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
                        copy_on_select_flag.clone(),
                        &shell_cache,
                    );
                }
            },
        );
    }

    // Ctrl+Shift+O: Split terminal vertically (top/bottom)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        let copy_on_select_flag = copy_on_select_flag.clone();
        let shell_cache = shell_cache.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("split_vertical", &kb_overrides),
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
                        copy_on_select_flag.clone(),
                        &shell_cache,
                    );
                }
            },
        );
    }

    // Alt+Left: Focus previous split pane
    {
        let tab_view = tab_view.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("focus_prev_split", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    terminal_container::focus_prev_terminal(&page.child());
                }
            },
        );
    }

    // Alt+Right: Focus next split pane
    {
        let tab_view = tab_view.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("focus_next_split", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    terminal_container::focus_next_terminal(&page.child());
                }
            },
        );
    }

    // --- Terminal search bar wiring ---

    // Search entry text changed -> set regex on active terminal
    {
        let tab_view_ref = tab_view.clone();
        find_entry.connect_search_changed(move |entry| {
            run_guarded_ui("terminal-search-changed", || {
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

    // Hide terminal search bar when switching to an editor tab
    {
        let search_revealer = search_revealer.clone();
        tab_view.connect_selected_page_notify(move |tv| {
            if let Some(page) = tv.selected_page() {
                let child = page.child();
                if editor::is_editor(&child) {
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
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("go_to_line", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        show_go_to_line_dialog(&window_ref, &child);
                    }
                }
            },
        );
    }

    // F12, Ctrl+Space, Ctrl+Shift+I: These are now handled by Monaco's
    // built-in providers, which fire EditorEvent callbacks (DefinitionRequested,
    // CompletionRequested, HoverRequested) handled in the create_editor event callback.

    // Ctrl+Shift+N: New window
    {
        let app_clone = app.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("new_window", &kb_overrides),
            move || {
                build_window(&app_clone);
            },
        );
    }

    // F11: Toggle fullscreen
    {
        let window_ref = window.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("fullscreen", &kb_overrides),
            move || {
                if window_ref.is_fullscreen() {
                    window_ref.unfullscreen();
                } else {
                    window_ref.fullscreen();
                }
            },
        );
    }

    // Register custom keybindings from settings
    {
        let custom_keybindings = settings.borrow().custom_keybindings.clone();
        for kb in custom_keybindings {
            let accel = keybindings::parse_keybinding_to_accel(&kb.key);
            if accel.is_empty() {
                log::warn!("Invalid keybinding: {}", kb.key);
                continue;
            }
            let command = kb.command.clone();
            let args = kb.args.clone();
            let kb_name = kb.name.clone();
            let tab_view = tab_view.clone();
            let setup_terminal_signals = setup_terminal_signals.clone();
            let settings = settings.clone();
            let copy_on_select_flag = copy_on_select_flag.clone();
            let icon_cache = sidebar_state.icon_cache.clone();
            add_shortcut(&shortcut_controller, &accel, move || {
                // Open a new terminal tab running the command in the active CWD
                let cwd = get_active_cwd(&tab_view);

                let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
                let term = terminal::create_terminal(
                    &settings.borrow(),
                    theme,
                    copy_on_select_flag.clone(),
                );
                setup_terminal_signals(&term);
                terminal::spawn_command(&term, &command, &args, cwd.as_deref());

                let container = terminal_container::TerminalContainer::new(&term);
                let page = tab_view.append(&container.widget);
                page.set_title(&kb_name);
                if let Some(texture) = icon_cache.borrow().get_toolbar_icon("console") {
                    page.set_icon(Some(texture));
                }
                tab_view.set_selected_page(&page);
                term.grab_focus();
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

    // Close tab_view pages: check for unsaved editor changes before closing.
    // Open a new terminal tab when the last tab is closed.
    {
        let window_ref = window.clone();
        let sidebar_state = sidebar_state.clone();
        let lsp_tx = lsp_request_tx.clone();
        let create_tab_on_empty = create_tab.clone();
        tab_view.connect_close_page(move |tv, page| {
            sidebar_state.remove_tab_state(&page.child());
            let child = page.child();

            // Check if this is an editor tab with unsaved changes
            if editor::is_editor(&child) {
                if editor::is_modified(&child) {
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
                                        let _ =
                                            lsp_tx.send(LspRequest::DidSave { uri: uri.clone() });
                                    }
                                }
                                editor::unregister_handle(&path);
                                let _ = lsp_tx.send(LspRequest::DidClose { uri });
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
                                let _ = lsp_tx.send(LspRequest::DidClose { uri });
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
            }

            // Terminal tab or unmodified editor: close immediately
            if editor::is_editor(&child) {
                let path = child.widget_name().to_string();
                editor::unregister_handle(&path);
                let _ = lsp_tx.send(LspRequest::DidClose {
                    uri: file_path_to_uri(std::path::Path::new(&path))
                        .unwrap_or_else(|| format!("file://{}", path)),
                });
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

pub fn send_diff_decorations(handle: &crate::editor_webview::MonacoEditorHandle, file_path: &str) {
    let decorations = match impulse_core::git::get_file_diff(file_path) {
        Ok(diff) => {
            let mut decos: Vec<impulse_editor::protocol::DiffDecoration> = diff
                .changed_lines
                .iter()
                .filter_map(|(&line, status)| {
                    let status_str = match status {
                        impulse_core::git::DiffLineStatus::Added => "added",
                        impulse_core::git::DiffLineStatus::Modified => "modified",
                        impulse_core::git::DiffLineStatus::Unchanged => return None,
                    };
                    Some(impulse_editor::protocol::DiffDecoration {
                        line,
                        status: status_str.to_string(),
                    })
                })
                .collect();
            for &line in &diff.deleted_lines {
                decos.push(impulse_editor::protocol::DiffDecoration {
                    line,
                    status: "deleted".to_string(),
                });
            }
            decos
        }
        Err(_) => vec![],
    };
    handle.apply_diff_decorations(decorations);
}

/// Runs all matching commands-on-save for the given file path.
/// Returns `true` if any successful command had `reload_file` set.
fn run_commands_on_save(path: &str, commands: &[crate::settings::CommandOnSave]) -> bool {
    let mut needs_reload = false;
    for cmd in commands {
        if crate::settings::matches_file_pattern(path, &cmd.file_pattern) {
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
                        if cmd.reload_file {
                            needs_reload = true;
                        }
                    }
                }
                Err(e) => log::warn!("Failed to run command '{}': {}", cmd.name, e),
            }
        }
    }
    needs_reload
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
            run_guarded_ui("quick-open-search-changed", || {
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
            run_guarded_ui("command-palette-search-changed", || {
                let query = entry.text().to_string().to_lowercase();
                populate_command_list(&list, &commands, &query);
            });
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
    if let Some(handle) = editor::get_handle_for_widget(editor_widget) {
        let content = handle.get_content();
        let total = content.lines().count();
        entry.set_placeholder_text(Some(&format!("1-{}", total)));
    }

    // Enter to go to line
    let editor_widget = editor_widget.clone();
    {
        let dialog = dialog.clone();
        entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if let Ok(line_num) = text.trim().parse::<u32>() {
                let line = line_num.max(1); // Monaco uses 1-based lines
                editor::go_to_position(&editor_widget, line, 1);
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

fn run_guarded_ui<F: FnOnce()>(label: &str, f: F) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "non-string panic payload"
        };
        log::error!("UI callback panic in '{}': {}", label, msg);
    }
}

fn completion_text_edit_to_info(
    edit: lsp_types::CompletionTextEdit,
) -> crate::lsp_completion::TextEditInfo {
    match edit {
        lsp_types::CompletionTextEdit::Edit(edit) => crate::lsp_completion::TextEditInfo {
            start_line: edit.range.start.line,
            start_character: edit.range.start.character,
            end_line: edit.range.end.line,
            end_character: edit.range.end.character,
            new_text: edit.new_text,
        },
        lsp_types::CompletionTextEdit::InsertAndReplace(edit) => {
            crate::lsp_completion::TextEditInfo {
                start_line: edit.replace.start.line,
                start_character: edit.replace.start.character,
                end_line: edit.replace.end.line,
                end_character: edit.replace.end.character,
                new_text: edit.new_text,
            }
        }
    }
}

fn completion_item_to_info(
    item: lsp_types::CompletionItem,
) -> crate::lsp_completion::CompletionInfo {
    let text_edit = item.text_edit.map(completion_text_edit_to_info);
    let additional_text_edits = item
        .additional_text_edits
        .unwrap_or_default()
        .into_iter()
        .map(|edit| crate::lsp_completion::TextEditInfo {
            start_line: edit.range.start.line,
            start_character: edit.range.start.character,
            end_line: edit.range.end.line,
            end_character: edit.range.end.character,
            new_text: edit.new_text,
        })
        .collect();

    crate::lsp_completion::CompletionInfo {
        label: item.label,
        detail: item.detail,
        insert_text: item.insert_text,
        insert_text_format: item.insert_text_format,
        text_edit,
        additional_text_edits,
        kind: format!(
            "{:?}",
            item.kind.unwrap_or(lsp_types::CompletionItemKind::TEXT)
        ),
    }
}

/// Get the working directory from the active tab: the terminal's CWD (via OSC 7)
/// or the parent directory of the file in an editor tab.
fn get_active_cwd(tab_view: &adw::TabView) -> Option<String> {
    let page = tab_view.selected_page()?;
    let child = page.child();

    // Try terminal CWD first
    if let Some(term) = terminal_container::get_active_terminal(&child) {
        if let Some(uri) = term.current_directory_uri() {
            let path = uri_to_file_path(&uri.to_string());
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // Try editor file's parent directory
    if editor::is_editor(&child) {
        let file_path = child.widget_name().to_string();
        if let Some(parent) = std::path::Path::new(&file_path).parent() {
            return Some(parent.to_string_lossy().to_string());
        }
    }

    None
}

/// Convert a local path to a file:// URI.
fn file_path_to_uri(path: &std::path::Path) -> Option<String> {
    if path.is_dir() {
        Url::from_directory_path(path).ok().map(|u| u.to_string())
    } else {
        Url::from_file_path(path).ok().map(|u| u.to_string())
    }
}

/// Convert a file:// URI to a local file path.
fn uri_to_file_path(uri: &str) -> String {
    if let Ok(parsed) = Url::parse(uri) {
        if parsed.scheme() == "file" {
            if let Ok(path) = parsed.to_file_path() {
                return path.to_string_lossy().to_string();
            }

            // Host-form file URIs (e.g. file://hostname/path) may fail
            // to_file_path() on some platforms; fall back to URI path.
            let decoded = url_decode(parsed.path());
            if !decoded.is_empty() {
                return decoded;
            }
        }
    }

    // Fallback for non-standard file URI strings.
    if let Some(rest) = uri.strip_prefix("file://") {
        if let Some(slash_idx) = rest.find('/') {
            return url_decode(&rest[slash_idx..]);
        }
        return url_decode(rest);
    }

    uri.to_string()
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

/// Determine LSP language ID from a file URI based on extension.
fn language_from_uri(uri: &str) -> String {
    let path = uri_to_file_path(uri);
    let path_obj = std::path::Path::new(&path);
    if let Some(name) = path_obj.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case("dockerfile") {
            return "dockerfile".to_string();
        }
    }
    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust".to_string(),
        "py" | "pyi" => "python".to_string(),
        "js" | "mjs" | "cjs" => "javascript".to_string(),
        "jsx" => "javascriptreact".to_string(),
        "ts" => "typescript".to_string(),
        "tsx" => "typescriptreact".to_string(),
        "c" | "h" => "c".to_string(),
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp".to_string(),
        "html" | "htm" => "html".to_string(),
        "css" => "css".to_string(),
        "scss" => "scss".to_string(),
        "less" => "less".to_string(),
        "json" => "json".to_string(),
        "jsonc" => "jsonc".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        "vue" => "vue".to_string(),
        "svelte" => "svelte".to_string(),
        "graphql" | "gql" => "graphql".to_string(),
        "sh" | "bash" | "zsh" | "fish" => "shellscript".to_string(),
        "dockerfile" => "dockerfile".to_string(),
        "go" => "go".to_string(),
        "java" => "java".to_string(),
        "rb" => "ruby".to_string(),
        "lua" => "lua".to_string(),
        "zig" => "zig".to_string(),
        "php" => "php".to_string(),
        _ => ext,
    }
}
