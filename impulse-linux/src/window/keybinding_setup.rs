use super::tab_management;
use gtk4::prelude::*;
use libadwaita as adw;
use vte4::prelude::*;

use std::rc::Rc;

use crate::editor;
use crate::editor_webview::MonacoEditorHandle;
use crate::keybindings;
use crate::lsp_completion::LspRequest;
use crate::terminal;
use crate::terminal_container;

use super::{
    add_shortcut, build_window, ensure_file_uri, get_active_cwd, language_from_uri,
    send_diff_decorations, show_go_to_line_dialog, uri_to_file_path, Command,
};

use super::sidebar_signals::dispatch_lsp_request;

/// Install the capture-phase EventControllerKey on the window.
///
/// This handles keybindings that VTE/WebView would otherwise consume
/// before the bubble-phase ShortcutController can see them: custom
/// keybindings, Ctrl+Shift+B (toggle sidebar), split-terminal shortcuts,
/// Ctrl+Shift+V (paste), Ctrl+W (close tab), Ctrl+T (new tab), and
/// Ctrl+1-9 (switch tab).
pub(super) fn setup_capture_phase_keys(
    ctx: &super::context::WindowContext,
    term_ctx: &super::context::TerminalContext,
    sidebar_btn: &gtk4::ToggleButton,
    setup_terminal_signals: &Rc<dyn Fn(&vte4::Terminal)>,
    create_tab: &(impl Fn() + Clone + 'static),
    reopen_tab: &Rc<dyn Fn()>,
) {
    let window = &ctx.window;
    let settings = &ctx.settings;
    let sidebar_state = &ctx.sidebar_state;
    let tab_view = ctx.tab_view.clone();
    let create_tab_capture = create_tab.clone();
    let sidebar_btn_capture = sidebar_btn.clone();
    let reopen_tab_capture = reopen_tab.clone();

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
                let copy_on_select = term_ctx.copy_on_select.clone();
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
                            copy_on_select.clone(),
                        );
                        setup_terminal_signals(&term);
                        terminal::spawn_command(&term, &command, &args, cwd.as_deref());
                        let container = terminal_container::TerminalContainer::new(&term);
                        let page = tab_management::insert_after_selected(&tab_view, &container.widget);
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

    let md_preview_accel = keybindings::parse_accel(&keybindings::get_accel(
        "toggle_markdown_preview",
        &capture_kb_overrides,
    ));
    let md_preview_settings = settings.clone();
    let md_preview_tab_view = tab_view.clone();
    let md_preview_status_bar = ctx.status_bar.clone();

    let capture_split = super::make_split_terminal(
        &tab_view,
        setup_terminal_signals,
        settings,
        &term_ctx.copy_on_select,
        &term_ctx.shell_cache,
    );

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

        // Toggle Preview (Ctrl+Shift+M by default, WebView eats it)
        if let Some(ref accel) = md_preview_accel {
            if keybindings::matches_key(accel, key, modifiers) {
                if let Some(page) = md_preview_tab_view.selected_page() {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        let s = md_preview_settings.borrow();
                        let theme = crate::theme::get_theme(&s.color_scheme);
                        if let Some(is_previewing) =
                            editor::toggle_preview(child.upcast_ref(), theme)
                        {
                            md_preview_status_bar
                                .borrow()
                                .show_preview_button(is_previewing);
                        }
                        return gtk4::glib::Propagation::Stop;
                    }
                }
            }
        }

        // Split terminal keybindings (VTE eats Ctrl+Shift+E/O)
        if let Some(page) = tab_view.selected_page() {
            let child = page.child();
            if terminal_container::get_active_terminal(&child).is_some() {
                if let Some(ref accel) = split_h_accel {
                    if keybindings::matches_key(accel, key, modifiers) {
                        capture_split(gtk4::Orientation::Vertical);
                        return gtk4::glib::Propagation::Stop;
                    }
                }
                if let Some(ref accel) = split_v_accel {
                    if keybindings::matches_key(accel, key, modifiers) {
                        capture_split(gtk4::Orientation::Horizontal);
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

            // Ctrl+Shift+T: reopen last closed tab (VTE/WebView eat this)
            if ctrl && shift && (key == gtk4::gdk::Key::t || key == gtk4::gdk::Key::T) {
                reopen_tab_capture();
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

/// Build and register the global ShortcutController with all keyboard shortcuts.
#[allow(clippy::too_many_arguments)]
pub(super) fn setup_shortcut_controller(
    ctx: &super::context::WindowContext,
    term_ctx: &super::context::TerminalContext,
    app: &adw::Application,
    sidebar_btn: &gtk4::ToggleButton,
    setup_terminal_signals: &Rc<dyn Fn(&vte4::Terminal)>,
    open_settings: &Rc<dyn Fn()>,
    search_revealer: &gtk4::Revealer,
    find_entry: &gtk4::SearchEntry,
    commands: &[Command],
    create_tab: &(impl Fn() + Clone + 'static),
    reopen_tab: &Rc<dyn Fn()>,
) {
    let window = &ctx.window;
    let tab_view = &ctx.tab_view;
    let sidebar_state = &ctx.sidebar_state;
    let settings = &ctx.settings;
    let toast_overlay = &ctx.toast_overlay;
    let lsp_request_tx = &ctx.lsp.request_tx;
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

    // Ctrl+N: New untitled editor tab
    {
        let tab_view = tab_view.clone();
        let settings = settings.clone();
        let status_bar = ctx.status_bar.clone();
        let icon_cache = sidebar_state.icon_cache.clone();
        let toast_overlay = toast_overlay.clone();
        let lsp_tx = lsp_request_tx.clone();
        let doc_versions = ctx.lsp.doc_versions.clone();
        let lsp_request_seq = ctx.lsp.request_seq.clone();
        let latest_completion_req = ctx.lsp.latest_completion_req.clone();
        let latest_hover_req = ctx.lsp.latest_hover_req.clone();
        let latest_definition_req = ctx.lsp.latest_definition_req.clone();
        let definition_monaco_ids = ctx.lsp.definition_monaco_ids.clone();
        let latest_formatting_req = ctx.lsp.latest_formatting_req.clone();
        let latest_signature_help_req = ctx.lsp.latest_signature_help_req.clone();
        let latest_references_req = ctx.lsp.latest_references_req.clone();
        let latest_code_action_req = ctx.lsp.latest_code_action_req.clone();
        let latest_rename_req = ctx.lsp.latest_rename_req.clone();
        let sidebar_state_for_new = sidebar_state.clone();
        let open_editor_paths = ctx.open_editor_paths.clone();
        let editor_tab_pages = ctx.editor_tab_pages.clone();
        let window_for_new = window.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("new_file", &kb_overrides),
            move || {
                let cwd = get_active_cwd(&tab_view);
                let theme = crate::theme::get_theme(&settings.borrow().color_scheme);
                let (editor_widget, _handle) = editor::create_untitled_editor(
                    &settings.borrow(),
                    theme,
                    cwd.clone(),
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
                        let sidebar_state = sidebar_state_for_new.clone();
                        let toast_overlay = toast_overlay.clone();
                        let editor_tab_pages = editor_tab_pages.clone();
                        let open_editor_paths = open_editor_paths.clone();
                        let window = window_for_new.clone();
                        let icon_cache = icon_cache.clone();
                        move |handle, event| {
                            let path = handle.file_path.borrow().clone();
                            let is_untitled = editor::is_untitled_path(&path);
                            match event {
                                impulse_editor::protocol::EditorEvent::Ready => {}
                                impulse_editor::protocol::EditorEvent::FileOpened => {
                                    handle.flush_pending_position();
                                    if !is_untitled {
                                        let uri = ensure_file_uri(&path);
                                        let language_id = language_from_uri(&uri);
                                        let content = handle.get_content();
                                        let mut versions = doc_versions.borrow_mut();
                                        let version = versions.entry(path.clone()).or_insert(0);
                                        *version += 1;
                                        if let Err(e) = lsp_tx.try_send(LspRequest::DidOpen {
                                            uri, language_id, version: *version, text: content,
                                        }) {
                                            log::warn!("LSP request channel full: {}", e);
                                        }
                                        send_diff_decorations(&path);
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::ContentChanged { content, version: _ } => {
                                    if let Some(page) = editor_tab_pages.borrow().get(&path) {
                                        if is_untitled {
                                            if handle.is_modified.get() {
                                                page.set_title("\u{25CF} Untitled");
                                            } else {
                                                page.set_title("Untitled");
                                            }
                                        } else {
                                            let filename = std::path::Path::new(&path)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or(&path);
                                            if handle.is_modified.get() {
                                                page.set_title(&format!("\u{25CF} {}", filename));
                                            } else {
                                                page.set_title(filename);
                                            }
                                        }
                                    }
                                    if !is_untitled {
                                        let uri = ensure_file_uri(&path);
                                        let mut versions = doc_versions.borrow_mut();
                                        let version = versions.entry(path.clone()).or_insert(0);
                                        *version += 1;
                                        if let Err(e) = lsp_tx.try_send(LspRequest::DidChange {
                                            uri, version: *version, text: content,
                                        }) {
                                            log::warn!("LSP request channel full: {}", e);
                                        }
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::CursorMoved { line, column } => {
                                    status_bar.borrow().update_cursor_position(line as i32 - 1, column as i32 - 1);
                                    if !is_untitled {
                                        // Git blame (same debounce pattern as sidebar_signals.rs)
                                        // Omitted for untitled files — no file to blame.
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::SaveRequested => {
                                    if is_untitled {
                                        show_save_dialog_for_untitled(
                                            &window, handle, &tab_view,
                                            &editor_tab_pages, &open_editor_paths,
                                            &lsp_tx, &doc_versions,
                                            &sidebar_state, &toast_overlay,
                                            &icon_cache, &settings,
                                        );
                                    } else {
                                        let content = handle.get_content();
                                        if let Err(e) = super::atomic_write(&path, &content) {
                                            log::error!("Failed to save {}: {}", path, e);
                                            let toast = adw::Toast::new(&format!("Error saving: {}", e));
                                            toast.set_timeout(4);
                                            toast_overlay.add_toast(toast);
                                        } else {
                                            handle.is_modified.set(false);
                                            if let Some(page) = editor_tab_pages.borrow().get(&path) {
                                                let filename = std::path::Path::new(&path)
                                                    .file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or(&path);
                                                page.set_title(filename);
                                            }
                                            let uri = ensure_file_uri(&path);
                                            if let Err(e) = lsp_tx.try_send(LspRequest::DidSave { uri }) {
                                                log::warn!("LSP request channel full: {}", e);
                                            }
                                            send_diff_decorations(&path);
                                            sidebar_state.refresh_git_only();
                                            let commands = settings.borrow().commands_on_save.clone();
                                            super::spawn_commands_on_save(path.clone(), commands);
                                        }
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::FocusChanged { focused } => {
                                    if !is_untitled && !focused && settings.borrow().auto_save && handle.is_modified.get() {
                                        let content = handle.get_content();
                                        if let Err(e) = super::atomic_write(&path, &content) {
                                            log::error!("Auto-save failed for {}: {}", path, e);
                                        } else {
                                            handle.is_modified.set(false);
                                            if let Some(page) = editor_tab_pages.borrow().get(&path) {
                                                let filename = std::path::Path::new(&path)
                                                    .file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or(&path);
                                                page.set_title(filename);
                                            }
                                            let uri = ensure_file_uri(&path);
                                            if let Err(e) = lsp_tx.try_send(LspRequest::DidSave { uri }) {
                                                log::warn!("LSP request channel full: {}", e);
                                            }
                                            send_diff_decorations(&path);
                                            sidebar_state.refresh_git_only();
                                        }
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::CompletionRequested { request_id: _, line, character } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_completion_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::Completion { request_id: seq, uri, version, line, character });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::HoverRequested { request_id: _, line, character } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_hover_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::Hover { request_id: seq, uri, version, line, character });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::DefinitionRequested { request_id: monaco_id, line, character } => {
                                    if !is_untitled {
                                        let seq = dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_definition_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::Definition { request_id: seq, uri, version, line, character });
                                        definition_monaco_ids.borrow_mut().insert(seq, monaco_id);
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::OpenFileRequested { uri, line, character } => {
                                    if !uri.starts_with("file://") && uri.contains("://") {
                                        log::warn!("Blocked opening non-file URI: {}", uri);
                                    } else {
                                        let file_path = uri_to_file_path(&uri);
                                        if let Some(cb) = sidebar_state.on_file_activated.borrow().as_ref() {
                                            cb(&file_path);
                                        }
                                        if let Some(page) = editor_tab_pages.borrow().get(&file_path) {
                                            editor::go_to_position(&page.child(), line + 1, character + 1);
                                            tab_view.set_selected_page(page);
                                        }
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::FormattingRequested { request_id: _, tab_size, insert_spaces } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_formatting_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::Formatting { request_id: seq, uri, version, tab_size, insert_spaces });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::SignatureHelpRequested { request_id: _, line, character } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_signature_help_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::SignatureHelp { request_id: seq, uri, version, line, character });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::ReferencesRequested { request_id: _, line, character } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_references_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::References { request_id: seq, uri, version, line, character });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::CodeActionRequested { request_id: _, start_line, start_column, end_line, end_column, diagnostics } => {
                                    if !is_untitled {
                                        let diag_infos: Vec<crate::lsp_completion::DiagnosticInfo> = diagnostics.into_iter().map(|d| {
                                            crate::lsp_completion::DiagnosticInfo {
                                                line: d.start_line, character: d.start_column,
                                                end_line: d.end_line, end_character: d.end_column,
                                                severity: match d.severity {
                                                    8 => crate::lsp_completion::DiagnosticSeverity::Error,
                                                    4 => crate::lsp_completion::DiagnosticSeverity::Warning,
                                                    2 => crate::lsp_completion::DiagnosticSeverity::Information,
                                                    _ => crate::lsp_completion::DiagnosticSeverity::Hint,
                                                },
                                                message: d.message,
                                            }
                                        }).collect();
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_code_action_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::CodeAction {
                                                request_id: seq, uri, version, start_line, start_column, end_line, end_column, diagnostics: diag_infos,
                                            });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::RenameRequested { request_id: _, line, character, new_name } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_rename_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::Rename { request_id: seq, uri, version, line, character, new_name });
                                    }
                                }
                                impulse_editor::protocol::EditorEvent::PrepareRenameRequested { request_id: _, line, character } => {
                                    if !is_untitled {
                                        dispatch_lsp_request(&path, &lsp_request_seq, &doc_versions, &latest_rename_req, &lsp_tx,
                                            |seq, uri, version| LspRequest::PrepareRename { request_id: seq, uri, version, line, character });
                                    }
                                }
                            }
                        }
                    },
                );
                let page = tab_management::insert_after_selected(&tab_view, &editor_widget);
                page.set_title("Untitled");
                if let Some(texture) = icon_cache.borrow().get_toolbar_icon("console") {
                    page.set_icon(Some(texture));
                }
                // Track the sentinel path in the dedup/page maps so Ctrl+S can find the page.
                let sentinel = editor_widget.widget_name().to_string();
                editor_tab_pages.borrow_mut().insert(sentinel.clone(), page.clone());
                tab_view.set_selected_page(&page);
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

    // Ctrl+Shift+T: Reopen last closed tab
    {
        let reopen_tab = reopen_tab.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("reopen_tab", &kb_overrides),
            move || {
                reopen_tab();
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

    // Ctrl+Shift+P: Command palette
    {
        let window_ref = window.clone();
        let commands = commands.to_vec();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("command_palette", &kb_overrides),
            move || {
                super::show_command_palette(&window_ref, &commands);
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
    // on the window (see setup_capture_phase_keys), which runs before VTE's
    // internal handler.

    // Ctrl+Equal / Ctrl+plus: Increase font size
    {
        let tab_view = tab_view.clone();
        let font_size = term_ctx.font_size.clone();
        let settings = settings.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_increase", &kb_overrides),
            move || {
                let new_size = font_size.get() + 1;
                if (6..=72).contains(&new_size) {
                    font_size.set(new_size);
                    let family = settings.borrow().terminal_font_family.clone();
                    super::apply_font_size_to_all_terminals(&tab_view, new_size, &family);
                }
            },
        );
    }

    // Ctrl+minus: Decrease font size
    {
        let tab_view = tab_view.clone();
        let font_size = term_ctx.font_size.clone();
        let settings = settings.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_decrease", &kb_overrides),
            move || {
                let new_size = font_size.get() - 1;
                if (6..=72).contains(&new_size) {
                    font_size.set(new_size);
                    let family = settings.borrow().terminal_font_family.clone();
                    super::apply_font_size_to_all_terminals(&tab_view, new_size, &family);
                }
            },
        );
    }

    // Ctrl+0: Reset font size to default (from settings)
    {
        let tab_view = tab_view.clone();
        let font_size = term_ctx.font_size.clone();
        let settings = settings.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_reset", &kb_overrides),
            move || {
                let s = settings.borrow();
                let default_size = s.terminal_font_size;
                let family = s.terminal_font_family.clone();
                drop(s);
                font_size.set(default_size);
                super::apply_font_size_to_all_terminals(&tab_view, default_size, &family);
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
        let window_for_save = window.clone();
        let editor_tab_pages_save = ctx.editor_tab_pages.clone();
        let open_editor_paths_save = ctx.open_editor_paths.clone();
        let doc_versions_save = ctx.lsp.doc_versions.clone();
        let sidebar_state_save = sidebar_state.clone();
        let icon_cache_save = sidebar_state.icon_cache.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("save", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        let path = child.widget_name().to_string();
                        // Untitled files: show save-as dialog instead
                        if editor::is_untitled_path(&path) {
                            if let Some(handle) = editor::get_handle(&path) {
                                show_save_dialog_for_untitled(
                                    &window_for_save, &handle, &tab_view,
                                    &editor_tab_pages_save, &open_editor_paths_save,
                                    &lsp_tx, &doc_versions_save,
                                    &sidebar_state_save, &toast_overlay,
                                    &icon_cache_save, &settings,
                                );
                            }
                            return;
                        }
                        if let Some(text) = editor::get_editor_text(&child) {
                            match super::atomic_write(&path, &text) {
                                Ok(()) => {
                                    editor::set_unmodified(&child);
                                    // Revert tab title
                                    let filename = std::path::Path::new(&path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(&path);
                                    page.set_title(filename);
                                    // LSP: send didSave
                                    if let Err(e) = lsp_tx.try_send(LspRequest::DidSave {
                                        uri: ensure_file_uri(&path),
                                    }) {
                                        log::warn!(
                                            "LSP request channel full, dropping request: {}",
                                            e
                                        );
                                    }
                                    let toast = adw::Toast::new(&format!("Saved {}", filename));
                                    toast.set_timeout(2);
                                    toast_overlay.add_toast(toast);
                                    // Run commands-on-save in a background thread
                                    let commands = settings.borrow().commands_on_save.clone();
                                    super::spawn_commands_on_save(path.clone(), commands);
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

    // Ctrl+Shift+E: Split terminal horizontally (top/bottom)
    {
        let split = super::make_split_terminal(
            tab_view,
            setup_terminal_signals,
            settings,
            &term_ctx.copy_on_select,
            &term_ctx.shell_cache,
        );
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("split_horizontal", &kb_overrides),
            move || split(gtk4::Orientation::Vertical),
        );
    }

    // Ctrl+Shift+O: Split terminal vertically (side by side)
    {
        let split = super::make_split_terminal(
            tab_view,
            setup_terminal_signals,
            settings,
            &term_ctx.copy_on_select,
            &term_ctx.shell_cache,
        );
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("split_vertical", &kb_overrides),
            move || split(gtk4::Orientation::Horizontal),
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

    // Ctrl+Shift+M: Toggle Preview (also handled in capture phase for WebView)
    {
        let tab_view = tab_view.clone();
        let settings = settings.clone();
        let status_bar = ctx.status_bar.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("toggle_markdown_preview", &kb_overrides),
            move || {
                if let Some(page) = tab_view.selected_page() {
                    let child = page.child();
                    if editor::is_editor(&child) {
                        let s = settings.borrow();
                        let theme = crate::theme::get_theme(&s.color_scheme);
                        if let Some(is_previewing) =
                            editor::toggle_preview(child.upcast_ref(), theme)
                        {
                            status_bar.borrow().show_preview_button(is_previewing);
                        }
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
                build_window(&app_clone, None);
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
            let copy_on_select_flag = term_ctx.copy_on_select.clone();
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
                let page = tab_management::insert_after_selected(&tab_view, &container.widget);
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
}

/// Show a save-as dialog for an untitled editor, then transition it to a
/// file-backed editor on successful save.
#[allow(clippy::too_many_arguments)]
fn show_save_dialog_for_untitled(
    window: &adw::ApplicationWindow,
    handle: &Rc<MonacoEditorHandle>,
    tab_view: &adw::TabView,
    editor_tab_pages: &Rc<std::cell::RefCell<std::collections::HashMap<String, adw::TabPage>>>,
    open_editor_paths: &Rc<std::cell::RefCell<std::collections::HashSet<String>>>,
    lsp_tx: &Rc<tokio::sync::mpsc::Sender<LspRequest>>,
    doc_versions: &Rc<std::cell::RefCell<std::collections::HashMap<String, i32>>>,
    sidebar_state: &Rc<crate::sidebar::SidebarState>,
    toast_overlay: &adw::ToastOverlay,
    icon_cache: &Rc<std::cell::RefCell<crate::file_icons::IconCache>>,
    settings: &Rc<std::cell::RefCell<crate::settings::Settings>>,
) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Save As");
    dialog.set_initial_name(Some("Untitled"));
    if let Some(cwd) = handle.untitled_cwd.borrow().as_deref() {
        dialog.set_initial_folder(Some(&gtk4::gio::File::for_path(cwd)));
    }

    let handle = handle.clone();
    let tab_view = tab_view.clone();
    let editor_tab_pages = editor_tab_pages.clone();
    let open_editor_paths = open_editor_paths.clone();
    let lsp_tx = lsp_tx.clone();
    let doc_versions = doc_versions.clone();
    let sidebar_state = sidebar_state.clone();
    let toast_overlay = toast_overlay.clone();
    let icon_cache = icon_cache.clone();
    let settings = settings.clone();

    dialog.save(
        Some(window),
        gtk4::gio::Cancellable::NONE,
        move |result| {
            let file = match result {
                Ok(f) => f,
                Err(_) => return, // user cancelled
            };
            let chosen_path = match file.path() {
                Some(p) => p.to_string_lossy().to_string(),
                None => return,
            };

            // Write content to disk
            let content = handle.get_content();
            if let Err(e) = super::atomic_write(&chosen_path, &content) {
                let toast = adw::Toast::new(&format!("Error saving: {}", e));
                toast.set_timeout(4);
                toast_overlay.add_toast(toast);
                return;
            }

            // Transition: unregister old sentinel, register new path
            let old_sentinel = handle.file_path.borrow().clone();
            editor::unregister_handle(&old_sentinel);
            editor_tab_pages.borrow_mut().remove(&old_sentinel);

            // Update the handle to point to the new path
            *handle.file_path.borrow_mut() = chosen_path.clone();
            *handle.untitled_cwd.borrow_mut() = None;
            handle.is_modified.set(false);

            // Detect language and re-open in Monaco with correct URI + language
            let uri = ensure_file_uri(&chosen_path);
            let language_id = language_from_uri(&uri);
            *handle.language.borrow_mut() = language_id.clone();
            handle.open_file(&chosen_path, &content, &language_id);

            // Register the handle at the new path
            editor::register_handle(&chosen_path, handle.clone());

            // Update the widget_name on the container so is_editor() and get_handle_for_widget() work
            if let Some(page) = tab_view.selected_page() {
                let child = page.child();
                child.set_widget_name(&chosen_path);

                // Update tab title and icon
                let filename = std::path::Path::new(&chosen_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&chosen_path);
                page.set_title(filename);
                if let Some(texture) = icon_cache.borrow().get(filename, false, false) {
                    page.set_icon(Some(texture));
                }

                // Track in dedup set and page map
                open_editor_paths.borrow_mut().insert(chosen_path.clone());
                editor_tab_pages.borrow_mut().insert(chosen_path.clone(), page.clone());
            }

            // Setup file watcher for the new path
            handle.setup_file_watcher();

            // LSP: send didOpen
            {
                let content = handle.get_content();
                let mut versions = doc_versions.borrow_mut();
                let version = versions.entry(chosen_path.clone()).or_insert(0);
                *version += 1;
                if let Err(e) = lsp_tx.try_send(LspRequest::DidOpen {
                    uri: ensure_file_uri(&chosen_path),
                    language_id,
                    version: *version,
                    text: content,
                }) {
                    log::warn!("LSP request channel full: {}", e);
                }
            }

            // Send diff decorations
            send_diff_decorations(&chosen_path);
            sidebar_state.refresh_git_only();

            // Run commands-on-save
            let commands = settings.borrow().commands_on_save.clone();
            super::spawn_commands_on_save(chosen_path.clone(), commands);

            // Toast
            let filename = std::path::Path::new(&chosen_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&chosen_path);
            let toast = adw::Toast::new(&format!("Saved {}", filename));
            toast.set_timeout(2);
            toast_overlay.add_toast(toast);
        },
    );
}
