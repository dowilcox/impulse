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
        theme: MonacoThemeDefinition,
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
