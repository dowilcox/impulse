use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Commands: Rust → Monaco (sent via evaluate_javascript)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EditorCommand {
    OpenFile {
        file_path: String,
        content: String,
        language: String,
    },
    SetTheme {
        theme: Box<MonacoThemeDefinition>,
    },
    UpdateSettings {
        options: EditorOptions,
    },
    ApplyDiagnostics {
        uri: String,
        markers: Vec<MonacoDiagnostic>,
    },
    ResolveCompletions {
        request_id: u64,
        items: Vec<MonacoCompletionItem>,
    },
    ResolveHover {
        request_id: u64,
        contents: Vec<MonacoHoverContent>,
    },
    ResolveDefinition {
        request_id: u64,
        /// None means "no definition found". Some means navigate to this location.
        uri: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
    GoToPosition {
        line: u32,
        column: u32,
    },
    SetReadOnly {
        read_only: bool,
    },
    ApplyDiffDecorations {
        decorations: Vec<DiffDecoration>,
    },
}

// ---------------------------------------------------------------------------
// Events: Monaco → Rust (sent via postMessage)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EditorEvent {
    Ready,
    FileOpened,
    ContentChanged {
        content: String,
        version: u32,
    },
    CursorMoved {
        line: u32,
        column: u32,
    },
    SaveRequested,
    CompletionRequested {
        request_id: u64,
        line: u32,
        character: u32,
    },
    HoverRequested {
        request_id: u64,
        line: u32,
        character: u32,
    },
    DefinitionRequested {
        request_id: u64,
        line: u32,
        character: u32,
    },
    /// Fired when Monaco wants to open a different file (e.g. cross-file
    /// go-to-definition via Cmd+click). The host should open the file and
    /// navigate to the given position.
    OpenFileRequested {
        uri: String,
        line: u32,
        character: u32,
    },
    FocusChanged {
        focused: bool,
    },
}

// ---------------------------------------------------------------------------
// Supporting Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_spaces: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimap_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_whitespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_line_highlight: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rulers: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticky_scroll: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bracket_pair_colorization: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_guides: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ligatures: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folding: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_beyond_last_line: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smooth_scrolling: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_blinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_closing_brackets: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoDiagnostic {
    pub severity: u8,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoCompletionItem {
    pub label: String,
    pub kind: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub insert_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text_rules: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<MonacoRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_text_edits: Vec<MonacoTextEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoHoverContent {
    pub value: String,
    #[serde(default)]
    pub is_trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoRange {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoTextEdit {
    pub range: MonacoRange,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Diff decorations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffDecoration {
    /// 1-based line number
    pub line: u32,
    /// "added", "modified", or "deleted"
    pub status: String,
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoThemeDefinition {
    pub base: String,
    pub inherit: bool,
    pub rules: Vec<MonacoTokenRule>,
    pub colors: MonacoThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoTokenRule {
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonacoThemeColors {
    #[serde(rename = "editor.background")]
    pub editor_background: String,
    #[serde(rename = "editor.foreground")]
    pub editor_foreground: String,
    #[serde(rename = "editor.lineHighlightBackground")]
    pub editor_line_highlight_background: String,
    #[serde(rename = "editor.selectionBackground")]
    pub editor_selection_background: String,
    #[serde(rename = "editorCursor.foreground")]
    pub editor_cursor_foreground: String,
    #[serde(rename = "editorLineNumber.foreground")]
    pub editor_line_number_foreground: String,
    #[serde(rename = "editorLineNumber.activeForeground")]
    pub editor_line_number_active_foreground: String,
    #[serde(rename = "editorWidget.background")]
    pub editor_widget_background: String,
    #[serde(rename = "editorSuggestWidget.background")]
    pub editor_suggest_widget_background: String,
    #[serde(rename = "editorSuggestWidget.selectedBackground")]
    pub editor_suggest_widget_selected_background: String,
    #[serde(rename = "editorHoverWidget.background")]
    pub editor_hover_widget_background: String,
    #[serde(rename = "editorGutter.background")]
    pub editor_gutter_background: String,
    #[serde(rename = "minimap.background")]
    pub minimap_background: String,
    #[serde(rename = "scrollbarSlider.background")]
    pub scrollbar_slider_background: String,
    #[serde(rename = "scrollbarSlider.hoverBackground")]
    pub scrollbar_slider_hover_background: String,
    #[serde(rename = "impulse.diffAddedColor")]
    pub diff_added_color: String,
    #[serde(rename = "impulse.diffModifiedColor")]
    pub diff_modified_color: String,
    #[serde(rename = "impulse.diffDeletedColor")]
    pub diff_deleted_color: String,
}

// ---------------------------------------------------------------------------
// Completion kind mapping (Monaco CompletionItemKind values)
// ---------------------------------------------------------------------------

pub fn lsp_completion_kind_to_monaco(kind: &str) -> u32 {
    match kind {
        "Method" | "method" => 0,
        "Function" | "function" => 1,
        "Constructor" | "constructor" => 2,
        "Field" | "field" => 3,
        "Variable" | "variable" => 4,
        "Class" | "class" => 5,
        "Struct" | "struct" => 6,
        "Interface" | "interface" => 7,
        "Module" | "module" => 8,
        "Property" | "property" => 9,
        "Event" | "event" => 10,
        "Operator" | "operator" => 11,
        "Unit" | "unit" => 12,
        "Value" | "value" => 13,
        "Constant" | "constant" => 14,
        "Enum" | "enum" => 15,
        "EnumMember" | "enum-member" => 16,
        "Keyword" | "keyword" => 17,
        "Snippet" | "snippet" => 27,
        "Text" | "text" => 18,
        "Color" | "color" => 19,
        "File" | "file" => 20,
        "Reference" | "reference" => 21,
        "Folder" | "folder" => 23,
        "TypeParameter" | "type-parameter" => 24,
        _ => 18, // default to Text
    }
}

// ---------------------------------------------------------------------------
// Diagnostic severity mapping (Monaco MarkerSeverity values)
// ---------------------------------------------------------------------------

pub fn diagnostic_severity_to_monaco(severity: u8) -> u8 {
    match severity {
        1 => 8, // Error -> MarkerSeverity.Error
        2 => 4, // Warning -> MarkerSeverity.Warning
        3 => 2, // Information -> MarkerSeverity.Info
        4 => 1, // Hint -> MarkerSeverity.Hint
        _ => 2, // default to Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_command_roundtrip_open_file() {
        let cmd = EditorCommand::OpenFile {
            file_path: "/tmp/test.rs".to_string(),
            content: "fn main() {}".to_string(),
            language: "rust".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::OpenFile {
                file_path,
                content,
                language,
            } => {
                assert_eq!(file_path, "/tmp/test.rs");
                assert_eq!(content, "fn main() {}");
                assert_eq!(language, "rust");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_command_roundtrip_go_to_position() {
        let cmd = EditorCommand::GoToPosition {
            line: 42,
            column: 10,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::GoToPosition { line, column } => {
                assert_eq!(line, 42);
                assert_eq!(column, 10);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_content_changed() {
        let event = EditorEvent::ContentChanged {
            content: "hello".to_string(),
            version: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::ContentChanged { content, version } => {
                assert_eq!(content, "hello");
                assert_eq!(version, 5);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_cursor_moved() {
        let event = EditorEvent::CursorMoved { line: 1, column: 1 };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::CursorMoved { line, column } => {
                assert_eq!(line, 1);
                assert_eq!(column, 1);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_completion_requested() {
        let event = EditorEvent::CompletionRequested {
            request_id: 99,
            line: 10,
            character: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::CompletionRequested {
                request_id,
                line,
                character,
            } => {
                assert_eq!(request_id, 99);
                assert_eq!(line, 10);
                assert_eq!(character, 5);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_tagged_serialization() {
        let event = EditorEvent::Ready;
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"Ready\""));
    }

    #[test]
    fn editor_command_tagged_serialization() {
        let cmd = EditorCommand::SetReadOnly { read_only: true };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"SetReadOnly\""));
        assert!(json.contains("\"read_only\":true"));
    }

    #[test]
    fn editor_options_skip_none_fields() {
        let opts = EditorOptions {
            font_size: Some(14),
            font_family: None,
            tab_size: None,
            insert_spaces: None,
            word_wrap: None,
            minimap_enabled: None,
            line_numbers: None,
            render_whitespace: None,
            render_line_highlight: None,
            rulers: None,
            sticky_scroll: None,
            bracket_pair_colorization: None,
            indent_guides: None,
            font_ligatures: None,
            folding: None,
            scroll_beyond_last_line: None,
            smooth_scrolling: None,
            cursor_style: None,
            cursor_blinking: None,
            line_height: None,
            auto_closing_brackets: None,
        };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("\"font_size\":14"));
        assert!(!json.contains("font_family"));
    }

    #[test]
    fn lsp_completion_kind_to_monaco_known() {
        assert_eq!(lsp_completion_kind_to_monaco("Method"), 0);
        assert_eq!(lsp_completion_kind_to_monaco("Function"), 1);
        assert_eq!(lsp_completion_kind_to_monaco("Variable"), 4);
        assert_eq!(lsp_completion_kind_to_monaco("Keyword"), 17);
    }

    #[test]
    fn lsp_completion_kind_to_monaco_unknown_defaults_to_text() {
        assert_eq!(lsp_completion_kind_to_monaco("Unknown"), 18);
    }

    #[test]
    fn diagnostic_severity_mapping() {
        assert_eq!(diagnostic_severity_to_monaco(1), 8); // Error
        assert_eq!(diagnostic_severity_to_monaco(2), 4); // Warning
        assert_eq!(diagnostic_severity_to_monaco(3), 2); // Info
        assert_eq!(diagnostic_severity_to_monaco(4), 1); // Hint
        assert_eq!(diagnostic_severity_to_monaco(255), 2); // Unknown -> Info
    }

    #[test]
    fn diff_decoration_roundtrip() {
        let dec = DiffDecoration {
            line: 42,
            status: "added".to_string(),
        };
        let json = serde_json::to_string(&dec).unwrap();
        let parsed: DiffDecoration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.line, 42);
        assert_eq!(parsed.status, "added");
    }

    #[test]
    fn editor_command_roundtrip_set_theme() {
        let cmd = EditorCommand::SetTheme {
            theme: Box::new(MonacoThemeDefinition {
                base: "vs-dark".to_string(),
                inherit: true,
                rules: vec![MonacoTokenRule {
                    token: "comment".to_string(),
                    foreground: Some("6a9955".to_string()),
                    font_style: Some("italic".to_string()),
                }],
                colors: MonacoThemeColors {
                    editor_background: "#1a1b26".to_string(),
                    editor_foreground: "#c0caf5".to_string(),
                    editor_line_highlight_background: "#292e42".to_string(),
                    editor_selection_background: "#33467c".to_string(),
                    editor_cursor_foreground: "#c0caf5".to_string(),
                    editor_line_number_foreground: "#3b4261".to_string(),
                    editor_line_number_active_foreground: "#737aa2".to_string(),
                    editor_widget_background: "#1a1b26".to_string(),
                    editor_suggest_widget_background: "#1a1b26".to_string(),
                    editor_suggest_widget_selected_background: "#292e42".to_string(),
                    editor_hover_widget_background: "#1a1b26".to_string(),
                    editor_gutter_background: "#1a1b26".to_string(),
                    minimap_background: "#1a1b26".to_string(),
                    scrollbar_slider_background: "#3b4261".to_string(),
                    scrollbar_slider_hover_background: "#545c7e".to_string(),
                    diff_added_color: "#9ece6a".to_string(),
                    diff_modified_color: "#e0af68".to_string(),
                    diff_deleted_color: "#f7768e".to_string(),
                },
            }),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::SetTheme { theme } => {
                assert_eq!(theme.base, "vs-dark");
                assert!(theme.inherit);
                assert_eq!(theme.rules.len(), 1);
                assert_eq!(theme.rules[0].token, "comment");
                assert_eq!(theme.rules[0].foreground.as_deref(), Some("6a9955"));
                assert_eq!(theme.rules[0].font_style.as_deref(), Some("italic"));
                assert_eq!(theme.colors.editor_background, "#1a1b26");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_command_roundtrip_update_settings() {
        let cmd = EditorCommand::UpdateSettings {
            options: EditorOptions {
                font_size: Some(16),
                font_family: Some("Fira Code".to_string()),
                tab_size: Some(2),
                insert_spaces: Some(true),
                word_wrap: Some("on".to_string()),
                minimap_enabled: None,
                line_numbers: None,
                render_whitespace: None,
                render_line_highlight: None,
                rulers: None,
                sticky_scroll: None,
                bracket_pair_colorization: None,
                indent_guides: None,
                font_ligatures: None,
                folding: None,
                scroll_beyond_last_line: None,
                smooth_scrolling: None,
                cursor_style: None,
                cursor_blinking: None,
                line_height: None,
                auto_closing_brackets: None,
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::UpdateSettings { options } => {
                assert_eq!(options.font_size, Some(16));
                assert_eq!(options.font_family.as_deref(), Some("Fira Code"));
                assert_eq!(options.tab_size, Some(2));
                assert_eq!(options.insert_spaces, Some(true));
                assert_eq!(options.word_wrap.as_deref(), Some("on"));
                assert!(options.minimap_enabled.is_none());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_command_roundtrip_apply_diagnostics() {
        let cmd = EditorCommand::ApplyDiagnostics {
            uri: "file:///tmp/test.rs".to_string(),
            markers: vec![MonacoDiagnostic {
                severity: 1,
                start_line: 0,
                start_column: 5,
                end_line: 0,
                end_column: 10,
                message: "unused variable".to_string(),
                source: Some("rustc".to_string()),
            }],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::ApplyDiagnostics { uri, markers } => {
                assert_eq!(uri, "file:///tmp/test.rs");
                assert_eq!(markers.len(), 1);
                assert_eq!(markers[0].severity, 1);
                assert_eq!(markers[0].start_line, 0);
                assert_eq!(markers[0].start_column, 5);
                assert_eq!(markers[0].end_line, 0);
                assert_eq!(markers[0].end_column, 10);
                assert_eq!(markers[0].message, "unused variable");
                assert_eq!(markers[0].source.as_deref(), Some("rustc"));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_command_roundtrip_resolve_hover() {
        let cmd = EditorCommand::ResolveHover {
            request_id: 42,
            contents: vec![MonacoHoverContent {
                value: "```rust\nfn main()\n```".to_string(),
                is_trusted: false,
            }],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::ResolveHover {
                request_id,
                contents,
            } => {
                assert_eq!(request_id, 42);
                assert_eq!(contents.len(), 1);
                assert_eq!(contents[0].value, "```rust\nfn main()\n```");
                assert!(!contents[0].is_trusted);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_command_roundtrip_apply_diff_decorations() {
        let cmd = EditorCommand::ApplyDiffDecorations {
            decorations: vec![
                DiffDecoration {
                    line: 1,
                    status: "added".to_string(),
                },
                DiffDecoration {
                    line: 5,
                    status: "modified".to_string(),
                },
                DiffDecoration {
                    line: 10,
                    status: "deleted".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: EditorCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorCommand::ApplyDiffDecorations { decorations } => {
                assert_eq!(decorations.len(), 3);
                assert_eq!(decorations[0].line, 1);
                assert_eq!(decorations[0].status, "added");
                assert_eq!(decorations[1].line, 5);
                assert_eq!(decorations[1].status, "modified");
                assert_eq!(decorations[2].line, 10);
                assert_eq!(decorations[2].status, "deleted");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_file_opened() {
        let event = EditorEvent::FileOpened;
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"FileOpened\""));
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::FileOpened => {}
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_save_requested() {
        let event = EditorEvent::SaveRequested;
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"SaveRequested\""));
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::SaveRequested => {}
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_hover_requested() {
        let event = EditorEvent::HoverRequested {
            request_id: 7,
            line: 20,
            character: 15,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::HoverRequested {
                request_id,
                line,
                character,
            } => {
                assert_eq!(request_id, 7);
                assert_eq!(line, 20);
                assert_eq!(character, 15);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_definition_requested() {
        let event = EditorEvent::DefinitionRequested {
            request_id: 7,
            line: 30,
            character: 8,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::DefinitionRequested {
                request_id,
                line,
                character,
            } => {
                assert_eq!(request_id, 7);
                assert_eq!(line, 30);
                assert_eq!(character, 8);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn editor_event_roundtrip_focus_changed() {
        let event = EditorEvent::FocusChanged { focused: true };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EditorEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            EditorEvent::FocusChanged { focused } => {
                assert!(focused);
            }
            _ => panic!("Wrong variant"),
        }

        let event2 = EditorEvent::FocusChanged { focused: false };
        let json2 = serde_json::to_string(&event2).unwrap();
        let parsed2: EditorEvent = serde_json::from_str(&json2).unwrap();
        match parsed2 {
            EditorEvent::FocusChanged { focused } => {
                assert!(!focused);
            }
            _ => panic!("Wrong variant"),
        }
    }
}
