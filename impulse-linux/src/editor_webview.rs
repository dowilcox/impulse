use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use webkit6::prelude::*;

use impulse_editor::protocol::{
    self, EditorCommand, EditorEvent, EditorOptions, MonacoCompletionItem, MonacoDiagnostic,
    MonacoHoverContent, MonacoRange, MonacoTextEdit, MonacoThemeColors, MonacoThemeDefinition,
    MonacoTokenRule,
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
        let escaped = json.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!("impulseReceiveCommand('{}')", escaped);
        self.webview.evaluate_javascript(
            &script,
            None,
            None,
            None::<&gtk4::gio::Cancellable>,
            |_| {},
        );
    }

    #[allow(dead_code)]
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
        self.send_command(&EditorCommand::GoToPosition { line, column });
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
                is_trusted: true,
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
        self.send_command(&EditorCommand::SetTheme { theme: definition });
    }
}

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

    // Detect indentation from content
    let (use_spaces, indent_width) = detect_indentation(content);
    let indent_info = if use_spaces {
        format!("Spaces: {}", indent_width)
    } else {
        format!("Tab Size: {}", indent_width)
    };

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
        wk_settings.set_enable_developer_extras(true);
        wk_settings.set_allow_file_access_from_file_urls(true);
        wk_settings.set_allow_universal_access_from_file_urls(true);
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
        indent_info: RefCell::new(indent_info),
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
                    theme: initial_theme.clone(),
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
            }
        }

        // Update cached state for content/cursor events
        match &event {
            EditorEvent::ContentChanged { content, version } => {
                *handle_for_signal.cached_content.borrow_mut() = content.clone();
                handle_for_signal.version.set(*version);
                handle_for_signal.is_modified.set(true);
            }
            _ => {}
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
            let error_html = format!(
                "<html><body style='background:#1a1b26;color:#a9b1d6;font-family:monospace;padding:2em'>\
                 <h3>Editor failed to load</h3><p>{}</p></body></html>",
                e
            );
            webview.load_html(&error_html, None);
        }
    }

    container.append(&webview);

    (container, handle)
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
        minimap_enabled: Some(false),
        line_numbers: Some(if settings.show_line_numbers {
            "on".to_string()
        } else {
            "off".to_string()
        }),
        render_whitespace: Some("selection".to_string()),
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
    }
}

fn theme_to_monaco(theme: &ThemeColors) -> MonacoThemeDefinition {
    // Strip '#' prefix from colors for Monaco (which expects bare hex)
    let strip = |c: &str| c.trim_start_matches('#').to_string();

    MonacoThemeDefinition {
        base: "vs-dark".to_string(),
        inherit: true,
        rules: vec![
            MonacoTokenRule {
                token: "comment".to_string(),
                foreground: Some(strip(theme.comment)),
                font_style: Some("italic".to_string()),
            },
            MonacoTokenRule {
                token: "keyword".to_string(),
                foreground: Some(strip(theme.magenta)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "keyword.control".to_string(),
                foreground: Some(strip(theme.magenta)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "string".to_string(),
                foreground: Some(strip(theme.green)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "number".to_string(),
                foreground: Some(strip(theme.orange)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "type".to_string(),
                foreground: Some(strip(theme.cyan)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "type.identifier".to_string(),
                foreground: Some(strip(theme.cyan)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "function".to_string(),
                foreground: Some(strip(theme.blue)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "function.declaration".to_string(),
                foreground: Some(strip(theme.blue)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "variable".to_string(),
                foreground: Some(strip(theme.fg)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "constant".to_string(),
                foreground: Some(strip(theme.orange)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "operator".to_string(),
                foreground: Some(strip(theme.cyan)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "delimiter".to_string(),
                foreground: Some(strip(theme.fg_dark)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "tag".to_string(),
                foreground: Some(strip(theme.red)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "attribute.name".to_string(),
                foreground: Some(strip(theme.yellow)),
                font_style: None,
            },
            MonacoTokenRule {
                token: "attribute.value".to_string(),
                foreground: Some(strip(theme.green)),
                font_style: None,
            },
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
