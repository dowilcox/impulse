use gtk4::prelude::*;
use libadwaita as adw;
use vte4::prelude::*;

use std::rc::Rc;

use crate::editor;
use crate::keybindings;
use crate::lsp_completion::LspRequest;
use crate::terminal;
use crate::terminal_container;

use super::{
    add_shortcut, build_window, file_path_to_uri, get_active_cwd, run_commands_on_save,
    send_diff_decorations, show_go_to_line_dialog, Command,
};

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
    let split_copy_flag = term_ctx.copy_on_select.clone();
    let split_shell_cache = term_ctx.shell_cache.clone();

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
                if let Some(ref accel) = split_v_accel {
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
            if ctrl
                && shift
                && (key == gtk4::gdk::Key::t || key == gtk4::gdk::Key::T)
            {
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
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_increase", &kb_overrides),
            move || {
                let new_size = font_size.get() + 1;
                font_size.set(new_size);
                super::apply_font_size_to_all_terminals(&tab_view, new_size);
            },
        );
    }

    // Ctrl+minus: Decrease font size
    {
        let tab_view = tab_view.clone();
        let font_size = term_ctx.font_size.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_decrease", &kb_overrides),
            move || {
                let new_size = font_size.get() - 1;
                if new_size > 0 {
                    font_size.set(new_size);
                    super::apply_font_size_to_all_terminals(&tab_view, new_size);
                }
            },
        );
    }

    // Ctrl+0: Reset font size to default
    {
        let tab_view = tab_view.clone();
        let font_size = term_ctx.font_size.clone();
        add_shortcut(
            &shortcut_controller,
            &keybindings::get_accel("font_reset", &kb_overrides),
            move || {
                font_size.set(11);
                super::apply_font_size_to_all_terminals(&tab_view, 11);
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
                                    if let Err(e) = lsp_tx.try_send(LspRequest::DidSave {
                                        uri: file_path_to_uri(std::path::Path::new(&path))
                                            .unwrap_or_else(|| format!("file://{}", path)),
                                    }) {
                                        log::warn!("LSP request channel full, dropping request: {}", e);
                                    }
                                    let toast = adw::Toast::new(&format!("Saved {}", filename));
                                    toast.set_timeout(2);
                                    toast_overlay.add_toast(toast);
                                    // Run commands-on-save in a background thread
                                    let commands = settings.borrow().commands_on_save.clone();
                                    let save_path = path.clone();
                                    std::thread::spawn(move || {
                                        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                                        })) {
                                            log::error!("Background thread panicked: {:?}", e);
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

    // Ctrl+Shift+E: Split terminal horizontally (top/bottom)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        let copy_on_select_flag = term_ctx.copy_on_select.clone();
        let shell_cache = term_ctx.shell_cache.clone();
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

    // Ctrl+Shift+O: Split terminal vertically (side by side)
    {
        let tab_view = tab_view.clone();
        let setup_terminal_signals = setup_terminal_signals.clone();
        let settings = settings.clone();
        let copy_on_select_flag = term_ctx.copy_on_select.clone();
        let shell_cache = term_ctx.shell_cache.clone();
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
}
