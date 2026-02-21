use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use webkit6::prelude::*;

use impulse_editor::protocol::{
    self, DiffDecoration, EditorCommand, EditorEvent, EditorOptions, MonacoCompletionItem,
    MonacoDiagnostic, MonacoHoverContent, MonacoRange, MonacoTextEdit, MonacoThemeColors,
    MonacoThemeDefinition, MonacoTokenRule,
};

use crate::lsp_completion::{CompletionInfo, DiagnosticInfo, DiagnosticSeverity};
use crate::settings::Settings;
use crate::theme::ThemeColors;

/// Handle for communicating with a Monaco editor running inside a WebView.
pub struct MonacoEditorHandle {
    webview: webkit6::WebView,
    pub file_path: RefCell<String>,
    pub cached_content: Rc<RefCell<String>>,
    pub is_modified: Rc<Cell<bool>>,
    pub is_ready: Rc<Cell<bool>>,
    pub language: RefCell<String>,
    pub version: Rc<Cell<u32>>,
    pub indent_info: RefCell<String>,
    /// When true, the next ContentChanged event will not mark the file as modified.
    /// Used when reloading file content externally (e.g. discard changes).
    pub suppress_next_modify: Rc<Cell<bool>>,
    /// Position to navigate to once the editor becomes ready (for cross-file go-to-definition).
    pending_position: Cell<Option<(u32, u32)>>,
    /// When set, the editor will be put into read-only mode once it becomes ready.
    pending_read_only: Cell<bool>,
    /// Keeps the file watcher alive. Dropping this stops watching.
    _file_watcher: Rc<RefCell<Option<notify::RecommendedWatcher>>>,
    /// Source ID for the file watcher's polling timer.
    _file_watcher_timer: RefCell<Option<glib::SourceId>>,
}

impl MonacoEditorHandle {
    fn send_command(&self, cmd: &EditorCommand) {
        if !self.is_ready.get() {
            log::warn!(
                "Monaco not ready yet, dropping command for {}",
                self.file_path.borrow()
            );
            return;
        }
        let json = match serde_json::to_string(cmd) {
            Ok(j) => j,
            Err(e) => {
                log::error!("Failed to serialize EditorCommand: {}", e);
                return;
            }
        };
        // Escape for embedding in JS string literal
        let escaped = js_string_escape(&json);
        let script = format!("impulseReceiveCommand('{}')", escaped);
        self.webview.evaluate_javascript(
            &script,
            None,
            None,
            None::<&gtk4::gio::Cancellable>,
            |_| {},
        );
    }

    pub fn open_file(&self, file_path: &str, content: &str, language: &str) {
        *self.file_path.borrow_mut() = file_path.to_string();
        *self.cached_content.borrow_mut() = content.to_string();
        *self.language.borrow_mut() = language.to_string();
        self.is_modified.set(false);
        self.version.set(0);

        self.send_command(&EditorCommand::OpenFile {
            file_path: file_path.to_string(),
            content: content.to_string(),
            language: language.to_string(),
        });
    }

    pub fn get_content(&self) -> String {
        self.cached_content.borrow().clone()
    }

    pub fn go_to_position(&self, line: u32, column: u32) {
        if !self.is_ready.get() {
            self.pending_position.set(Some((line, column)));
            return;
        }
        self.send_command(&EditorCommand::GoToPosition { line, column });
    }

    /// Sends any queued go-to-position command (set while the editor wasn't ready).
    pub fn flush_pending_position(&self) {
        if let Some((line, column)) = self.pending_position.take() {
            self.send_command(&EditorCommand::GoToPosition { line, column });
        }
    }

    pub fn apply_diagnostics(&self, diagnostics: &[DiagnosticInfo]) {
        let markers: Vec<MonacoDiagnostic> = diagnostics
            .iter()
            .map(|d| {
                let severity = match d.severity {
                    DiagnosticSeverity::Error => 1,
                    DiagnosticSeverity::Warning => 2,
                    DiagnosticSeverity::Information => 3,
                    DiagnosticSeverity::Hint => 4,
                };
                MonacoDiagnostic {
                    severity: protocol::diagnostic_severity_to_monaco(severity),
                    start_line: d.line,
                    start_column: d.character,
                    end_line: d.end_line,
                    end_column: d.end_character,
                    message: d.message.clone(),
                    source: None,
                }
            })
            .collect();

        let uri = format!("file://{}", self.file_path.borrow());
        self.send_command(&EditorCommand::ApplyDiagnostics { uri, markers });
    }

    pub fn resolve_completions(&self, request_id: u64, items: &[CompletionInfo]) {
        let monaco_items: Vec<MonacoCompletionItem> = items
            .iter()
            .map(|item| {
                let insert_text_rules = item.insert_text_format.and_then(|fmt| {
                    if fmt == lsp_types::InsertTextFormat::SNIPPET {
                        Some(4) // CompletionItemInsertTextRule.InsertAsSnippet
                    } else {
                        None
                    }
                });

                let range = item.text_edit.as_ref().map(|te| MonacoRange {
                    start_line: te.start_line,
                    start_column: te.start_character,
                    end_line: te.end_line,
                    end_column: te.end_character,
                });

                let additional_text_edits: Vec<MonacoTextEdit> = item
                    .additional_text_edits
                    .iter()
                    .map(|te| MonacoTextEdit {
                        range: MonacoRange {
                            start_line: te.start_line,
                            start_column: te.start_character,
                            end_line: te.end_line,
                            end_column: te.end_character,
                        },
                        text: te.new_text.clone(),
                    })
                    .collect();

                MonacoCompletionItem {
                    label: item.label.clone(),
                    kind: protocol::lsp_completion_kind_to_monaco(&item.kind),
                    detail: item.detail.clone(),
                    insert_text: item
                        .insert_text
                        .clone()
                        .or_else(|| item.text_edit.as_ref().map(|te| te.new_text.clone()))
                        .unwrap_or_else(|| item.label.clone()),
                    insert_text_rules,
                    range,
                    additional_text_edits,
                }
            })
            .collect();

        self.send_command(&EditorCommand::ResolveCompletions {
            request_id,
            items: monaco_items,
        });
    }

    pub fn resolve_hover(&self, request_id: u64, content: &str) {
        let contents = if content.is_empty() {
            vec![]
        } else {
            vec![MonacoHoverContent {
                value: format!("```\n{}\n```", content),
                is_trusted: false,
            }]
        };
        self.send_command(&EditorCommand::ResolveHover {
            request_id,
            contents,
        });
    }

    pub fn apply_settings(&self, settings: &Settings) {
        let options = settings_to_editor_options(settings);
        self.send_command(&EditorCommand::UpdateSettings { options });
    }

    pub fn set_theme(&self, theme: &ThemeColors) {
        let definition = theme_to_monaco(theme);
        self.send_command(&EditorCommand::SetTheme {
            theme: Box::new(definition),
        });
    }

    pub fn apply_diff_decorations(&self, decorations: Vec<DiffDecoration>) {
        self.send_command(&EditorCommand::ApplyDiffDecorations { decorations });
    }

    pub fn set_read_only(&self, read_only: bool) {
        if self.is_ready.get() {
            self.send_command(&EditorCommand::SetReadOnly { read_only });
        } else {
            self.pending_read_only.set(read_only);
        }
    }

    /// Resolve a pending definition request in Monaco. Monaco will show an
    /// underline on hover and navigate on Ctrl+click.
    pub fn send_resolve_definition(
        &self,
        request_id: u64,
        uri: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    ) {
        self.send_command(&EditorCommand::ResolveDefinition {
            request_id,
            uri,
            line,
            column,
        });
    }

    /// Release resources held by this editor handle. Must be called before the
    /// tab is removed to break the reference cycle between the GLib timer, the
    /// WebView, and the signal closures that hold `Rc<MonacoEditorHandle>`.
    pub fn cleanup(&self) {
        // Cancel the file-watcher polling timer. This frees the timer closure,
        // which drops its WebView clone, which in turn allows the WebView (and
        // its signal closures holding Rc<Self>) to be deallocated.
        if let Some(id) = self._file_watcher_timer.borrow_mut().take() {
            id.remove();
        }
        // Drop the filesystem watcher (closes the inotify fd).
        self._file_watcher.borrow_mut().take();
        // Unregister the JS→Rust message handler so the UCM signal closure
        // (which holds an Rc<Self>) is disconnected.
        if let Some(ucm) = self.webview.user_content_manager() {
            ucm.unregister_script_message_handler("impulse", None);
        }
    }

    /// Set up a filesystem watcher that reloads the editor content when the file
    /// is modified externally (only if there are no unsaved changes).
    /// Also handles atomic writes (temp → rename) by matching Create/Modify/Rename
    /// events and restarting the watcher after a successful reload.
    pub fn setup_file_watcher(&self) {
        use notify::{RecursiveMode, Watcher};

        let file_path = self.file_path.borrow().clone();
        if file_path.is_empty() {
            return;
        }

        // Cancel previous watcher timer
        if let Some(id) = self._file_watcher_timer.borrow_mut().take() {
            id.remove();
        }

        // Use a shared AtomicBool so the watcher can be restarted (after
        // atomic writes) while the timer keeps polling the same flag.
        let changed = Arc::new(AtomicBool::new(false));
        let changed_for_watcher = changed.clone();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    ) {
                        changed_for_watcher.store(true, Ordering::Relaxed);
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("Failed to create file watcher: {}", e);
                    return;
                }
            };

        if let Err(e) = watcher.watch(
            std::path::Path::new(&file_path),
            RecursiveMode::NonRecursive,
        ) {
            log::warn!("Failed to watch file {}: {}", file_path, e);
            return;
        }

        let is_modified = self.is_modified.clone();
        let suppress_next_modify = self.suppress_next_modify.clone();
        let cached_content = self.cached_content.clone();
        let file_path_cell = self.file_path.clone();
        let language = self.language.clone();
        let webview = self.webview.clone();
        let is_ready = self.is_ready.clone();
        let watcher_cell = self._file_watcher.clone();

        let timer_id = glib::timeout_add_local(Duration::from_millis(500), move || {
            if !changed.swap(false, Ordering::Relaxed) {
                return glib::ControlFlow::Continue;
            }
            if is_modified.get() || !is_ready.get() {
                return glib::ControlFlow::Continue;
            }
            let fp = file_path_cell.borrow().clone();
            if let Ok(new_content) = std::fs::read_to_string(&fp) {
                if new_content != *cached_content.borrow() {
                    *cached_content.borrow_mut() = new_content.clone();
                    suppress_next_modify.set(true);
                    let lang = language.borrow().clone();
                    let cmd = EditorCommand::OpenFile {
                        file_path: fp.clone(),
                        content: new_content,
                        language: lang,
                    };
                    if let Ok(json) = serde_json::to_string(&cmd) {
                        let escaped = js_string_escape(&json);
                        let script = format!("impulseReceiveCommand('{}')", escaped);
                        webview.evaluate_javascript(
                            &script,
                            None,
                            None,
                            None::<&gtk4::gio::Cancellable>,
                            |_| {},
                        );
                    }
                }
                // Restart watcher: after an atomic write the inotify watch may
                // be on a stale inode. Create a new watcher writing to the same
                // `changed` flag so this timer picks up future events.
                let changed_restart = changed.clone();
                if let Ok(mut new_watcher) =
                    notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                        if let Ok(event) = res {
                            if matches!(
                                event.kind,
                                notify::EventKind::Create(_)
                                    | notify::EventKind::Modify(_)
                                    | notify::EventKind::Remove(_)
                            ) {
                                changed_restart.store(true, Ordering::Relaxed);
                            }
                        }
                    })
                {
                    if new_watcher
                        .watch(std::path::Path::new(&fp), RecursiveMode::NonRecursive)
                        .is_ok()
                    {
                        *watcher_cell.borrow_mut() = Some(new_watcher);
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        *self._file_watcher.borrow_mut() = Some(watcher);
        *self._file_watcher_timer.borrow_mut() = Some(timer_id);
    }
}

// ---------------------------------------------------------------------------
// WebView pre-warming pool
// ---------------------------------------------------------------------------

/// A pre-warmed WebView with Monaco loaded and ready to accept commands.
struct WarmWebView {
    webview: webkit6::WebView,
    user_content_manager: webkit6::UserContentManager,
    is_ready: Rc<Cell<bool>>,
    signal_handler_id: glib::SignalHandlerId,
}

thread_local! {
    static WARM_POOL: RefCell<Option<WarmWebView>> = const { RefCell::new(None) };
}

/// Pre-extract Monaco assets and start loading a WebView in the background.
/// When the WebView is ready, it can be claimed by `create_monaco_editor` for
/// an instant editor open. Call this once at app startup.
pub fn warm_up_editor() {
    WARM_POOL.with(|cell| {
        if cell.borrow().is_some() {
            return;
        }

        let monaco_dir = match impulse_editor::assets::ensure_monaco_extracted() {
            Ok(dir) => dir,
            Err(e) => {
                log::warn!("Failed to extract Monaco for pre-warm: {}", e);
                return;
            }
        };

        let ucm = webkit6::UserContentManager::new();
        let webview = webkit6::WebView::builder()
            .user_content_manager(&ucm)
            .hexpand(true)
            .vexpand(true)
            .build();

        webview.set_background_color(&gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));

        if let Some(wk_settings) = webkit6::prelude::WebViewExt::settings(&webview) {
            wk_settings.set_enable_javascript(true);
            if std::env::var("IMPULSE_DEVTOOLS").ok().is_some_and(|v| v == "1") {
                wk_settings.set_enable_developer_extras(true);
            }
            wk_settings.set_allow_file_access_from_file_urls(false);
        }

        let is_ready = Rc::new(Cell::new(false));
        let is_ready_clone = is_ready.clone();

        ucm.register_script_message_handler("impulse", None);
        let signal_id = ucm.connect_script_message_received(Some("impulse"), move |_ucm, value| {
            let json_str = value.to_str().to_string();
            if let Ok(event) = serde_json::from_str::<EditorEvent>(&json_str) {
                if matches!(event, EditorEvent::Ready) {
                    is_ready_clone.set(true);
                    log::info!("Pre-warmed editor WebView is ready");
                }
            }
        });

        let uri = format!("file://{}/editor.html", monaco_dir.display());
        webview.load_uri(&uri);

        log::info!("Started pre-warming editor WebView");

        *cell.borrow_mut() = Some(WarmWebView {
            webview,
            user_content_manager: ucm,
            is_ready,
            signal_handler_id: signal_id,
        });
    });
}

/// Try to claim a pre-warmed WebView. Returns `Some` only if one is ready.
fn claim_warm_editor() -> Option<WarmWebView> {
    WARM_POOL.with(|cell| {
        let is_ready = cell.borrow().as_ref().is_some_and(|w| w.is_ready.get());
        if is_ready {
            cell.borrow_mut().take()
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------

/// Create a Monaco editor widget inside a WebView.
///
/// Returns the container `gtk4::Box` (with `widget_name` set to `file_path`)
/// and a handle for communicating with the editor.
///
/// The `on_event` callback is invoked for each `EditorEvent` the JS bridge
/// sends. The caller uses this to wire up LSP requests, status bar updates,
/// file saving, etc.
pub fn create_monaco_editor<F>(
    file_path: &str,
    content: &str,
    language: &str,
    settings: &Settings,
    theme: &ThemeColors,
    on_event: F,
) -> (gtk4::Box, Rc<MonacoEditorHandle>)
where
    F: Fn(&MonacoEditorHandle, EditorEvent) + 'static,
{
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.set_widget_name(file_path);

    // Detect indentation from content, then apply per-file-type overrides
    let (mut use_spaces, mut indent_width) = detect_indentation(content);
    for ovr in &settings.file_type_overrides {
        if crate::settings::matches_file_pattern(file_path, &ovr.pattern) {
            if let Some(tw) = ovr.tab_width {
                indent_width = tw;
            }
            if let Some(us) = ovr.use_spaces {
                use_spaces = us;
            }
            break;
        }
    }
    let indent_info = if use_spaces {
        format!("Spaces: {}", indent_width)
    } else {
        format!("Tab Size: {}", indent_width)
    };

    // Try to claim a pre-warmed WebView for instant editor opening.
    if let Some(warm) = claim_warm_editor() {
        let webview = warm.webview;
        let ucm = warm.user_content_manager;

        // Disconnect the warm-up signal handler.
        ucm.disconnect(warm.signal_handler_id);

        // Update the background to match the current theme.
        let bg_rgba =
            gtk4::gdk::RGBA::parse(theme.bg).unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
        webview.set_background_color(&bg_rgba);

        // Create the handle with is_ready already set — Monaco is loaded.
        let handle = Rc::new(MonacoEditorHandle {
            webview: webview.clone(),
            file_path: RefCell::new(file_path.to_string()),
            cached_content: Rc::new(RefCell::new(content.to_string())),
            is_modified: Rc::new(Cell::new(false)),
            is_ready: Rc::new(Cell::new(true)),
            language: RefCell::new(language.to_string()),
            version: Rc::new(Cell::new(0)),
            suppress_next_modify: Rc::new(Cell::new(false)),
            pending_position: Cell::new(None),
            pending_read_only: Cell::new(false),
            indent_info: RefCell::new(indent_info),
            _file_watcher: Rc::new(RefCell::new(None)),
            _file_watcher_timer: RefCell::new(None),
        });

        // Connect the real signal handler for ongoing events.
        let handle_for_signal = handle.clone();
        ucm.connect_script_message_received(Some("impulse"), move |_ucm, value| {
            let json_str = value.to_str().to_string();

            let event: EditorEvent = match serde_json::from_str(&json_str) {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("Failed to parse EditorEvent: {} (json: {})", e, json_str);
                    return;
                }
            };

            // Update cached state for content/cursor events.
            if let EditorEvent::ContentChanged { content, version } = &event {
                *handle_for_signal.cached_content.borrow_mut() = content.clone();
                handle_for_signal.version.set(*version);
                if handle_for_signal.suppress_next_modify.get() {
                    handle_for_signal.suppress_next_modify.set(false);
                } else {
                    handle_for_signal.is_modified.set(true);
                }
            }

            on_event(&handle_for_signal, event);
        });

        // Immediately send theme, settings, and file content.
        handle.send_command(&EditorCommand::SetTheme {
            theme: Box::new(theme_to_monaco(theme)),
        });

        let mut options = settings_to_editor_options(settings);
        options.tab_size = Some(indent_width);
        options.insert_spaces = Some(use_spaces);
        handle.send_command(&EditorCommand::UpdateSettings { options });

        handle.send_command(&EditorCommand::OpenFile {
            file_path: file_path.to_string(),
            content: content.to_string(),
            language: language.to_string(),
        });

        container.append(&webview);

        // Start warming the next WebView.
        glib::idle_add_local_once(warm_up_editor);

        return (container, handle);
    }

    // -- Fallback: create a fresh WebView --

    // Create the UserContentManager and register our message handler
    let user_content_manager = webkit6::UserContentManager::new();

    // Build the WebView
    let webview = webkit6::WebView::builder()
        .user_content_manager(&user_content_manager)
        .hexpand(true)
        .vexpand(true)
        .build();

    // Set the WebView background to match the theme so no black flashes during scroll/load
    let bg_rgba =
        gtk4::gdk::RGBA::parse(theme.bg).unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
    webview.set_background_color(&bg_rgba);

    // Configure WebView settings
    if let Some(wk_settings) = webkit6::prelude::WebViewExt::settings(&webview) {
        wk_settings.set_enable_javascript(true);
        if std::env::var("IMPULSE_DEVTOOLS").ok().is_some_and(|v| v == "1") {
            wk_settings.set_enable_developer_extras(true);
        }
        wk_settings.set_allow_file_access_from_file_urls(false);
    }

    // Create the handle
    let handle = Rc::new(MonacoEditorHandle {
        webview: webview.clone(),
        file_path: RefCell::new(file_path.to_string()),
        cached_content: Rc::new(RefCell::new(content.to_string())),
        is_modified: Rc::new(Cell::new(false)),
        is_ready: Rc::new(Cell::new(false)),
        language: RefCell::new(language.to_string()),
        version: Rc::new(Cell::new(0)),
        suppress_next_modify: Rc::new(Cell::new(false)),
        pending_position: Cell::new(None),
        pending_read_only: Cell::new(false),
        indent_info: RefCell::new(indent_info),
        _file_watcher: Rc::new(RefCell::new(None)),
        _file_watcher_timer: RefCell::new(None),
    });

    // Store initial content, language, settings, and theme to send after Ready
    let initial_file_path = file_path.to_string();
    let initial_content = content.to_string();
    let initial_language = language.to_string();
    let initial_settings = settings.clone();
    let initial_theme = theme_to_monaco(theme);
    let initial_indent_width = indent_width;
    let initial_use_spaces = use_spaces;

    // Connect JS→Rust message handler
    let handle_for_signal = handle.clone();
    let sent_initial = Rc::new(Cell::new(false));
    user_content_manager.register_script_message_handler("impulse", None);
    user_content_manager.connect_script_message_received(Some("impulse"), move |_ucm, value| {
        let json_str = value.to_str().to_string();

        let event: EditorEvent = match serde_json::from_str(&json_str) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to parse EditorEvent: {} (json: {})", e, json_str);
                return;
            }
        };

        // Handle Ready specially: send initial content, theme, settings
        if matches!(event, EditorEvent::Ready) {
            handle_for_signal.is_ready.set(true);

            if !sent_initial.get() {
                sent_initial.set(true);

                // Set theme first
                handle_for_signal.send_command(&EditorCommand::SetTheme {
                    theme: Box::new(initial_theme.clone()),
                });

                // Set settings (including indent from file detection)
                let mut options = settings_to_editor_options(&initial_settings);
                options.tab_size = Some(initial_indent_width);
                options.insert_spaces = Some(initial_use_spaces);
                handle_for_signal.send_command(&EditorCommand::UpdateSettings { options });

                // Open the file
                handle_for_signal.send_command(&EditorCommand::OpenFile {
                    file_path: initial_file_path.clone(),
                    content: initial_content.clone(),
                    language: initial_language.clone(),
                });

                // Apply deferred read-only mode (e.g. for large files)
                if handle_for_signal.pending_read_only.get() {
                    handle_for_signal.send_command(&EditorCommand::SetReadOnly {
                        read_only: true,
                    });
                }
            }
        }

        // Update cached state for content/cursor events
        if let EditorEvent::ContentChanged { content, version } = &event {
            *handle_for_signal.cached_content.borrow_mut() = content.clone();
            handle_for_signal.version.set(*version);
            if handle_for_signal.suppress_next_modify.get() {
                handle_for_signal.suppress_next_modify.set(false);
            } else {
                handle_for_signal.is_modified.set(true);
            }
        }

        // Forward to caller's event handler
        on_event(&handle_for_signal, event);
    });

    // Extract Monaco assets and load editor from local filesystem
    match impulse_editor::assets::ensure_monaco_extracted() {
        Ok(monaco_dir) => {
            let uri = format!("file://{}/editor.html", monaco_dir.display());
            webview.load_uri(&uri);
        }
        Err(e) => {
            log::error!("Failed to extract Monaco assets: {}", e);
            let safe_error = e
                .to_string()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let error_html = format!(
                "<html><body style='background:#1a1b26;color:#a9b1d6;font-family:monospace;padding:2em'>\
                 <h3>Editor failed to load</h3><p>{}</p></body></html>",
                safe_error
            );
            webview.load_html(&error_html, None);
        }
    }

    container.append(&webview);

    // Start warming the next WebView (for subsequent tabs).
    glib::idle_add_local_once(warm_up_editor);

    (container, handle)
}

// ---------------------------------------------------------------------------
// JS string escaping
// ---------------------------------------------------------------------------

/// Properly escape a string for embedding in a JavaScript single-quoted string literal.
/// This handles backslashes, quotes, newlines, and other special characters that
/// could break out of the string or cause injection.
fn js_string_escape(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '\u{2028}' => out.push_str("\\u2028"), // line separator
            '\u{2029}' => out.push_str("\\u2029"), // paragraph separator
            c if c < '\u{0020}' && c != '\n' && c != '\r' && c != '\t' => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn settings_to_editor_options(settings: &Settings) -> EditorOptions {
    EditorOptions {
        font_size: Some((settings.font_size as u32).max(8)),
        font_family: Some(if settings.font_family.is_empty() {
            "monospace".to_string()
        } else {
            settings.font_family.clone()
        }),
        tab_size: Some(settings.tab_width),
        insert_spaces: Some(settings.use_spaces),
        word_wrap: Some(if settings.word_wrap {
            "on".to_string()
        } else {
            "off".to_string()
        }),
        minimap_enabled: Some(settings.minimap_enabled),
        line_numbers: Some(if settings.show_line_numbers {
            "on".to_string()
        } else {
            "off".to_string()
        }),
        render_whitespace: Some(settings.render_whitespace.clone()),
        render_line_highlight: Some(if settings.highlight_current_line {
            "all".to_string()
        } else {
            "none".to_string()
        }),
        rulers: Some(if settings.show_right_margin {
            vec![settings.right_margin_position]
        } else {
            vec![]
        }),
        sticky_scroll: Some(settings.sticky_scroll),
        bracket_pair_colorization: Some(settings.bracket_pair_colorization),
        indent_guides: Some(settings.indent_guides),
        font_ligatures: Some(settings.font_ligatures),
        folding: Some(settings.folding),
        scroll_beyond_last_line: Some(settings.scroll_beyond_last_line),
        smooth_scrolling: Some(settings.smooth_scrolling),
        cursor_style: Some(settings.editor_cursor_style.clone()),
        cursor_blinking: Some(settings.editor_cursor_blinking.clone()),
        line_height: if settings.editor_line_height > 0 {
            Some(settings.editor_line_height)
        } else {
            None
        },
        auto_closing_brackets: Some(settings.editor_auto_closing_brackets.clone()),
    }
}

fn theme_to_monaco(theme: &ThemeColors) -> MonacoThemeDefinition {
    // Strip '#' prefix from colors for Monaco (which expects bare hex)
    let strip = |c: &str| c.trim_start_matches('#').to_string();

    MonacoThemeDefinition {
        base: "vs-dark".to_string(),
        inherit: true,
        rules: vec![
            // Comments (italic)
            MonacoTokenRule { token: "comment".to_string(), foreground: Some(strip(theme.comment)), font_style: Some("italic".to_string()) },
            MonacoTokenRule { token: "comment.doc".to_string(), foreground: Some(strip(theme.comment)), font_style: Some("italic".to_string()) },
            // Keywords (magenta)
            MonacoTokenRule { token: "keyword".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.control".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.declaration".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.type".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.other".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.flow".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.block".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.try".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.catch".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.choice".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            MonacoTokenRule { token: "keyword.modifier".to_string(), foreground: Some(strip(theme.magenta)), font_style: None },
            // Constants & numbers (orange)
            MonacoTokenRule { token: "keyword.constant".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "number".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "number.hex".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "number.float".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "number.binary".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "number.octal".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "constant".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            MonacoTokenRule { token: "string.escape".to_string(), foreground: Some(strip(theme.orange)), font_style: None },
            // Strings (green)
            MonacoTokenRule { token: "string".to_string(), foreground: Some(strip(theme.green)), font_style: None },
            MonacoTokenRule { token: "string.heredoc".to_string(), foreground: Some(strip(theme.green)), font_style: None },
            MonacoTokenRule { token: "string.raw".to_string(), foreground: Some(strip(theme.green)), font_style: None },
            MonacoTokenRule { token: "attribute.value".to_string(), foreground: Some(strip(theme.green)), font_style: None },
            // Operators, special strings, predefined (cyan)
            MonacoTokenRule { token: "string.key".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            MonacoTokenRule { token: "string.link".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            MonacoTokenRule { token: "operator".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            MonacoTokenRule { token: "keyword.operator".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            MonacoTokenRule { token: "variable.predefined".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            MonacoTokenRule { token: "predefined".to_string(), foreground: Some(strip(theme.cyan)), font_style: None },
            // Types, classes, annotations (yellow)
            MonacoTokenRule { token: "type".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "type.identifier".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "class".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "annotation".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "namespace".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "constructor".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            MonacoTokenRule { token: "attribute.name".to_string(), foreground: Some(strip(theme.yellow)), font_style: None },
            // Functions (blue)
            MonacoTokenRule { token: "function".to_string(), foreground: Some(strip(theme.blue)), font_style: None },
            MonacoTokenRule { token: "function.declaration".to_string(), foreground: Some(strip(theme.blue)), font_style: None },
            MonacoTokenRule { token: "function.call".to_string(), foreground: Some(strip(theme.blue)), font_style: None },
            MonacoTokenRule { token: "predefined.function".to_string(), foreground: Some(strip(theme.blue)), font_style: None },
            // Tags, invalid, regexp (red)
            MonacoTokenRule { token: "string.escape.invalid".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            MonacoTokenRule { token: "string.invalid".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            MonacoTokenRule { token: "regexp".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            MonacoTokenRule { token: "tag".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            MonacoTokenRule { token: "metatag".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            MonacoTokenRule { token: "invalid".to_string(), foreground: Some(strip(theme.red)), font_style: None },
            // Variables, emphasis (fg)
            MonacoTokenRule { token: "variable".to_string(), foreground: Some(strip(theme.fg)), font_style: None },
            MonacoTokenRule { token: "emphasis".to_string(), foreground: Some(strip(theme.fg)), font_style: Some("italic".to_string()) },
            // Delimiters (fg_dark)
            MonacoTokenRule { token: "delimiter".to_string(), foreground: Some(strip(theme.fg_dark)), font_style: None },
            // Strong (orange + bold)
            MonacoTokenRule { token: "strong".to_string(), foreground: Some(strip(theme.orange)), font_style: Some("bold".to_string()) },
        ],
        colors: MonacoThemeColors {
            editor_background: format!("#{}", strip(theme.bg)),
            editor_foreground: format!("#{}", strip(theme.fg)),
            editor_line_highlight_background: format!("#{}", strip(theme.bg_highlight)),
            editor_selection_background: format!("#{}80", strip(theme.blue)),
            editor_cursor_foreground: format!("#{}", strip(theme.fg)),
            editor_line_number_foreground: format!("#{}", strip(theme.comment)),
            editor_line_number_active_foreground: format!("#{}", strip(theme.fg)),
            editor_widget_background: format!("#{}", strip(theme.bg_dark)),
            editor_suggest_widget_background: format!("#{}", strip(theme.bg_dark)),
            editor_suggest_widget_selected_background: format!("#{}", strip(theme.bg_highlight)),
            editor_hover_widget_background: format!("#{}", strip(theme.bg_dark)),
            editor_gutter_background: format!("#{}", strip(theme.bg)),
            minimap_background: format!("#{}", strip(theme.bg_dark)),
            scrollbar_slider_background: format!("#{}40", strip(theme.comment)),
            scrollbar_slider_hover_background: format!("#{}60", strip(theme.comment)),
            diff_added_color: format!("#{}", strip(theme.green)),
            diff_modified_color: format!("#{}", strip(theme.yellow)),
            diff_deleted_color: format!("#{}", strip(theme.red)),
        },
    }
}

fn detect_indentation(content: &str) -> (bool, u32) {
    let mut space_lines = 0;
    let mut tab_lines = 0;
    let mut indent_widths = HashMap::new();

    for line in content.lines().take(100) {
        if line.starts_with('\t') {
            tab_lines += 1;
        } else if line.starts_with(' ') {
            space_lines += 1;
            let spaces = line.len() - line.trim_start_matches(' ').len();
            if spaces >= 2 {
                for width in &[2u32, 4, 8] {
                    if spaces % (*width as usize) == 0 {
                        *indent_widths.entry(*width).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let use_spaces = space_lines >= tab_lines;
    let indent_width = if use_spaces {
        indent_widths
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(width, _)| width)
            .unwrap_or(4)
    } else {
        4
    };

    (use_spaces, indent_width)
}
