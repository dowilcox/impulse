// SPDX-License-Identifier: GPL-3.0-only
//
// Monaco editor communication bridge QObject for QML. Handles asset
// extraction, file I/O, EditorCommand/EditorEvent serialization, and
// preview rendering.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, editor_html)]
        #[qproperty(QString, monaco_base_url)]
        #[qproperty(QString, current_file)]
        #[qproperty(bool, is_modified)]
        #[qproperty(i32, version)]
        type EditorBridge = super::EditorBridgeRust;

        #[qinvokable]
        fn get_editor_html(self: &EditorBridge) -> QString;

        #[qinvokable]
        fn ensure_monaco_extracted(self: Pin<&mut EditorBridge>) -> QString;

        #[qinvokable]
        fn open_file(self: Pin<&mut EditorBridge>, path: &QString) -> QString;

        #[qinvokable]
        fn save_file(self: Pin<&mut EditorBridge>, path: &QString, content: &QString) -> bool;

        #[qinvokable]
        fn make_command_json(
            self: &EditorBridge,
            command_type: &QString,
            params_json: &QString,
        ) -> QString;

        #[qinvokable]
        fn handle_event(self: Pin<&mut EditorBridge>, event_json: &QString);

        #[qinvokable]
        fn render_markdown_preview(
            self: &EditorBridge,
            source: &QString,
            theme_json: &QString,
        ) -> QString;

        #[qinvokable]
        fn render_svg_preview(self: &EditorBridge, source: &QString, bg_color: &QString)
            -> QString;

        #[qinvokable]
        fn is_previewable_file(self: &EditorBridge, path: &QString) -> bool;

        #[qinvokable]
        fn get_monaco_theme_json(self: &EditorBridge, theme_id: &QString) -> QString;

        #[qinvokable]
        fn language_from_path(self: &EditorBridge, path: &QString) -> QString;

        #[qsignal]
        fn editor_event(self: Pin<&mut EditorBridge>, event_type: QString, payload_json: QString);

        #[qsignal]
        fn file_saved(self: Pin<&mut EditorBridge>, path: QString);

        #[qsignal]
        fn content_changed(self: Pin<&mut EditorBridge>, path: QString, content: QString);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;

pub struct EditorBridgeRust {
    editor_html: QString,
    monaco_base_url: QString,
    current_file: QString,
    is_modified: bool,
    version: i32,
    /// Cached path to the extracted Monaco directory.
    monaco_dir: Option<String>,
}

impl Default for EditorBridgeRust {
    fn default() -> Self {
        Self {
            editor_html: QString::default(),
            monaco_base_url: QString::default(),
            current_file: QString::default(),
            is_modified: false,
            version: 0,
            monaco_dir: None,
        }
    }
}

impl qobject::EditorBridge {
    pub fn get_editor_html(&self) -> QString {
        QString::from(impulse_editor::assets::EDITOR_HTML)
    }

    pub fn ensure_monaco_extracted(mut self: Pin<&mut Self>) -> QString {
        match impulse_editor::assets::ensure_monaco_extracted() {
            Ok(path) => {
                let path_str = path.to_string_lossy().to_string();
                // Build a file:// URL for the Monaco base directory
                let url = format!("file://{}/", path_str);
                self.as_mut()
                    .set_monaco_base_url(QString::from(url.as_str()));
                self.as_mut().rust_mut().monaco_dir = Some(path_str.clone());

                // Also set the editor HTML
                let html = impulse_editor::assets::EDITOR_HTML;
                self.as_mut().set_editor_html(QString::from(html));

                QString::from(path_str.as_str())
            }
            Err(e) => {
                log::warn!("Failed to extract Monaco: {}", e);
                QString::from(format!("ERROR:{}", e).as_str())
            }
        }
    }

    pub fn open_file(mut self: Pin<&mut Self>, path: &QString) -> QString {
        let path_str = path.to_string();
        if path_str.is_empty() {
            return QString::from("{\"error\":\"empty path\"}");
        }

        // Read the file content
        let content = match std::fs::read_to_string(&path_str) {
            Ok(c) => c,
            Err(e) => {
                let err = format!("{{\"error\":\"{}\"}}", e);
                return QString::from(err.as_str());
            }
        };

        // Detect language from the file path
        let file_uri = format!("file://{}", path_str);
        let language = impulse_core::util::language_from_uri(&file_uri);

        // Build an EditorCommand::OpenFile
        let cmd = impulse_editor::protocol::EditorCommand::OpenFile {
            file_path: path_str.clone(),
            content,
            language,
        };

        self.as_mut().set_current_file(path.clone());
        self.as_mut().set_is_modified(false);
        self.as_mut().set_version(0);

        let json = serde_json::to_string(&cmd).unwrap_or_else(|_| "{}".to_string());
        QString::from(json.as_str())
    }

    pub fn save_file(mut self: Pin<&mut Self>, path: &QString, content: &QString) -> bool {
        let path_str = path.to_string();
        let content_str = content.to_string();

        if path_str.is_empty() {
            return false;
        }

        match std::fs::write(&path_str, &content_str) {
            Ok(()) => {
                self.as_mut().set_is_modified(false);
                let saved_path = path.clone();
                self.as_mut().file_saved(saved_path);
                true
            }
            Err(e) => {
                log::warn!("Failed to save file '{}': {}", path_str, e);
                false
            }
        }
    }

    pub fn make_command_json(
        &self,
        command_type: &QString,
        params_json: &QString,
    ) -> QString {
        let cmd_type = command_type.to_string();
        let params = params_json.to_string();

        // Parse the params JSON if provided
        let params_value: serde_json::Value = if params.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&params).unwrap_or(serde_json::Value::Null)
        };

        // Build the command based on the type
        let cmd = match cmd_type.as_str() {
            "SetTheme" => {
                // params_json should be a full MonacoThemeDefinition JSON
                match serde_json::from_value::<impulse_editor::protocol::MonacoThemeDefinition>(
                    params_value,
                ) {
                    Ok(theme_def) => {
                        impulse_editor::protocol::EditorCommand::SetTheme {
                            theme: Box::new(theme_def),
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to parse SetTheme params: {}", e);
                        return QString::from("{}");
                    }
                }
            }
            "UpdateSettings" => {
                match serde_json::from_value::<impulse_editor::protocol::EditorOptions>(
                    params_value,
                ) {
                    Ok(options) => {
                        impulse_editor::protocol::EditorCommand::UpdateSettings { options }
                    }
                    Err(e) => {
                        log::warn!("Failed to parse UpdateSettings params: {}", e);
                        return QString::from("{}");
                    }
                }
            }
            "GoToPosition" => {
                let line = params_value
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let column = params_value
                    .get("column")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                impulse_editor::protocol::EditorCommand::GoToPosition { line, column }
            }
            "SetReadOnly" => {
                let read_only = params_value
                    .get("read_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                impulse_editor::protocol::EditorCommand::SetReadOnly { read_only }
            }
            "ApplyDiagnostics" => {
                // Pass through the raw JSON
                match serde_json::from_value::<serde_json::Value>(params_value.clone()) {
                    Ok(v) => {
                        let uri = v
                            .get("uri")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();
                        let markers: Vec<impulse_editor::protocol::MonacoDiagnostic> = v
                            .get("markers")
                            .and_then(|m| serde_json::from_value(m.clone()).ok())
                            .unwrap_or_default();
                        impulse_editor::protocol::EditorCommand::ApplyDiagnostics { uri, markers }
                    }
                    Err(_) => return QString::from("{}"),
                }
            }
            "ApplyDiffDecorations" => {
                let decorations: Vec<impulse_editor::protocol::DiffDecoration> =
                    serde_json::from_value(
                        params_value
                            .get("decorations")
                            .cloned()
                            .unwrap_or(serde_json::Value::Array(vec![])),
                    )
                    .unwrap_or_default();
                impulse_editor::protocol::EditorCommand::ApplyDiffDecorations { decorations }
            }
            // For commands that are full JSON already (completions, hover, etc.),
            // try to deserialize the whole params as the command.
            _ => {
                match serde_json::from_str::<impulse_editor::protocol::EditorCommand>(&params) {
                    Ok(cmd) => cmd,
                    Err(e) => {
                        log::warn!(
                            "Unknown or unparseable command type '{}': {}",
                            cmd_type,
                            e
                        );
                        return QString::from("{}");
                    }
                }
            }
        };

        let json = serde_json::to_string(&cmd).unwrap_or_else(|_| "{}".to_string());
        QString::from(json.as_str())
    }

    pub fn handle_event(mut self: Pin<&mut Self>, event_json: &QString) {
        let json_str = event_json.to_string();
        if json_str.is_empty() {
            return;
        }

        let event: impulse_editor::protocol::EditorEvent = match serde_json::from_str(&json_str) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to parse editor event: {}", e);
                return;
            }
        };

        match event {
            impulse_editor::protocol::EditorEvent::Ready => {
                self.as_mut().editor_event(
                    QString::from("Ready"),
                    QString::from("{}"),
                );
            }
            impulse_editor::protocol::EditorEvent::FileOpened => {
                self.as_mut().editor_event(
                    QString::from("FileOpened"),
                    QString::from("{}"),
                );
            }
            impulse_editor::protocol::EditorEvent::ContentChanged { content, version } => {
                self.as_mut().set_is_modified(true);
                let ver = version as i32;
                self.as_mut().set_version(ver);

                let current = self.as_ref().current_file().clone();
                let content_qs = QString::from(content.as_str());
                self.as_mut()
                    .content_changed(current, content_qs);
            }
            impulse_editor::protocol::EditorEvent::CursorMoved { line, column } => {
                let payload = serde_json::json!({
                    "line": line,
                    "column": column,
                });
                self.as_mut().editor_event(
                    QString::from("CursorMoved"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::SaveRequested => {
                self.as_mut().editor_event(
                    QString::from("SaveRequested"),
                    QString::from("{}"),
                );
            }
            impulse_editor::protocol::EditorEvent::FocusChanged { focused } => {
                let payload = serde_json::json!({ "focused": focused });
                self.as_mut().editor_event(
                    QString::from("FocusChanged"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            // For LSP-related events, forward as JSON for the lsp_bridge to handle
            impulse_editor::protocol::EditorEvent::CompletionRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("CompletionRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::HoverRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("HoverRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::DefinitionRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("DefinitionRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::OpenFileRequested { uri, line, character } => {
                let payload = serde_json::json!({
                    "uri": uri,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("OpenFileRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::FormattingRequested {
                request_id,
                tab_size,
                insert_spaces,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "tab_size": tab_size,
                    "insert_spaces": insert_spaces,
                });
                self.as_mut().editor_event(
                    QString::from("FormattingRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::SignatureHelpRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("SignatureHelpRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::ReferencesRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("ReferencesRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::CodeActionRequested {
                request_id,
                start_line,
                start_column,
                end_line,
                end_column,
                diagnostics,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "start_line": start_line,
                    "start_column": start_column,
                    "end_line": end_line,
                    "end_column": end_column,
                    "diagnostics": diagnostics,
                });
                self.as_mut().editor_event(
                    QString::from("CodeActionRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::RenameRequested {
                request_id,
                line,
                character,
                new_name,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                    "new_name": new_name,
                });
                self.as_mut().editor_event(
                    QString::from("RenameRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
            impulse_editor::protocol::EditorEvent::PrepareRenameRequested {
                request_id,
                line,
                character,
            } => {
                let payload = serde_json::json!({
                    "request_id": request_id,
                    "line": line,
                    "character": character,
                });
                self.as_mut().editor_event(
                    QString::from("PrepareRenameRequested"),
                    QString::from(payload.to_string().as_str()),
                );
            }
        }
    }

    pub fn render_markdown_preview(
        &self,
        source: &QString,
        theme_json: &QString,
    ) -> QString {
        let source_str = source.to_string();
        let theme_str = theme_json.to_string();

        let theme_colors: impulse_editor::markdown::MarkdownThemeColors =
            match serde_json::from_str(&theme_str) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("Failed to parse markdown theme: {}", e);
                    return QString::default();
                }
            };

        // Build highlight.js path from the Monaco directory
        let hljs_path = match &self.monaco_dir {
            Some(dir) => format!("file://{}/highlight/highlight.min.js", dir),
            None => String::new(),
        };

        match impulse_editor::markdown::render_markdown_preview(
            &source_str,
            &theme_colors,
            &hljs_path,
        ) {
            Some(html) => QString::from(html.as_str()),
            None => QString::default(),
        }
    }

    pub fn render_svg_preview(
        &self,
        source: &QString,
        bg_color: &QString,
    ) -> QString {
        let source_str = source.to_string();
        let bg = bg_color.to_string();

        match impulse_editor::svg::render_svg_preview(&source_str, &bg) {
            Some(html) => QString::from(html.as_str()),
            None => QString::default(),
        }
    }

    pub fn is_previewable_file(&self, path: &QString) -> bool {
        let path_str = path.to_string();
        impulse_editor::markdown::is_markdown_file(&path_str)
            || impulse_editor::svg::is_svg_file(&path_str)
    }

    pub fn get_monaco_theme_json(&self, theme_id: &QString) -> QString {
        let id = theme_id.to_string();
        let theme = impulse_core::theme::get_theme(&id);
        let monaco_def = impulse_editor::protocol::theme_to_monaco(&theme);
        let json = serde_json::to_string(&monaco_def).unwrap_or_else(|_| "{}".to_string());
        QString::from(json.as_str())
    }

    pub fn language_from_path(&self, path: &QString) -> QString {
        let path_str = path.to_string();
        let file_uri = if path_str.starts_with("file://") {
            path_str
        } else {
            format!("file://{}", path_str)
        };
        let lang = impulse_core::util::language_from_uri(&file_uri);
        QString::from(lang.as_str())
    }
}
