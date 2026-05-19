use impulse_editor::protocol::MonacoContentChange;

#[derive(Debug)]
pub enum LspRequest {
    Completion {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    Hover {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    Definition {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    DidOpen {
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    },
    DidChange {
        uri: String,
        version: i32,
        text: Option<String>,
        changes: Vec<lsp_types::TextDocumentContentChangeEvent>,
    },
    DidSave {
        uri: String,
    },
    DidClose {
        uri: String,
    },
    Formatting {
        request_id: u64,
        uri: String,
        version: i32,
        tab_size: u32,
        insert_spaces: bool,
    },
    SignatureHelp {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    References {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    CodeAction {
        request_id: u64,
        uri: String,
        version: i32,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
        diagnostics: Vec<DiagnosticInfo>,
    },
    Rename {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
        new_name: String,
    },
    PrepareRename {
        request_id: u64,
        uri: String,
        version: i32,
        line: u32,
        character: u32,
    },
    Shutdown,
}

pub fn lsp_content_changes(
    changes: &[MonacoContentChange],
) -> Vec<lsp_types::TextDocumentContentChangeEvent> {
    changes
        .iter()
        .map(|change| lsp_types::TextDocumentContentChangeEvent {
            range: Some(lsp_types::Range {
                start: lsp_types::Position {
                    line: change.range.start_line.saturating_sub(1),
                    character: change.range.start_column.saturating_sub(1),
                },
                end: lsp_types::Position {
                    line: change.range.end_line.saturating_sub(1),
                    character: change.range.end_column.saturating_sub(1),
                },
            }),
            range_length: Some(change.range_length),
            text: change.text.clone(),
        })
        .collect()
}

pub fn apply_lsp_content_changes(
    content: &mut String,
    changes: &[lsp_types::TextDocumentContentChangeEvent],
) {
    if changes.is_empty() {
        return;
    }

    for change in changes.iter().rev() {
        let Some(range) = change.range else {
            *content = change.text.clone();
            continue;
        };
        let start = lsp_position_to_byte_offset(content, range.start);
        let end = lsp_position_to_byte_offset(content, range.end);
        if start <= end && end <= content.len() {
            content.replace_range(start..end, &change.text);
        }
    }
}

fn lsp_position_to_byte_offset(content: &str, position: lsp_types::Position) -> usize {
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (byte_index, ch) in content.char_indices() {
        if line == position.line {
            break;
        }
        if ch == '\n' {
            line = line.saturating_add(1);
            line_start = byte_index + ch.len_utf8();
        }
    }

    if line != position.line {
        return content.len();
    }

    let mut utf16_units = 0u32;
    for (relative, ch) in content[line_start..].char_indices() {
        if ch == '\n' || utf16_units >= position.character {
            return line_start + relative;
        }
        utf16_units = utf16_units.saturating_add(ch.len_utf16() as u32);
        if utf16_units > position.character {
            return line_start + relative;
        }
    }
    content.len()
}

#[derive(Debug)]
pub enum LspResponse {
    Diagnostics {
        uri: String,
        version: Option<i32>,
        diagnostics: Vec<DiagnosticInfo>,
    },
    CompletionResult {
        request_id: u64,
        uri: String,
        version: i32,
        items: Vec<CompletionInfo>,
    },
    HoverResult {
        request_id: u64,
        uri: String,
        version: i32,
        contents: String,
    },
    DefinitionResult {
        request_id: u64,
        source_uri: String,
        source_version: i32,
        uri: String,
        line: u32,
        character: u32,
    },
    ServerInitialized {
        client_key: String,
        server_id: String,
    },
    ServerError {
        client_key: String,
        server_id: String,
        message: String,
    },
    ServerExited {
        client_key: String,
        server_id: String,
    },
    FormattingResult {
        request_id: u64,
        uri: String,
        version: i32,
        edits: Vec<TextEditInfo>,
    },
    SignatureHelpResult {
        request_id: u64,
        uri: String,
        version: i32,
        signature_help: Option<SignatureHelpInfo>,
    },
    ReferencesResult {
        request_id: u64,
        uri: String,
        version: i32,
        locations: Vec<LocationInfo>,
    },
    CodeActionResult {
        request_id: u64,
        uri: String,
        version: i32,
        actions: Vec<CodeActionInfo>,
    },
    RenameResult {
        request_id: u64,
        uri: String,
        version: i32,
        edits: Vec<WorkspaceTextEditInfo>,
    },
    PrepareRenameResult {
        request_id: u64,
        uri: String,
        version: i32,
        range: Option<RangeInfo>,
        placeholder: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone)]
pub struct TextEditInfo {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub new_text: String,
}

#[derive(Debug, Clone)]
pub struct CompletionInfo {
    pub label: String,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub insert_text_format: Option<lsp_types::InsertTextFormat>,
    pub text_edit: Option<TextEditInfo>,
    pub additional_text_edits: Vec<TextEditInfo>,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct SignatureHelpInfo {
    pub signatures: Vec<SignatureInfo>,
    pub active_signature: u32,
    pub active_parameter: u32,
}

#[derive(Debug, Clone)]
pub struct SignatureInfo {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<ParameterInfo>,
}

#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub label: String,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocationInfo {
    pub uri: String,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

#[derive(Debug, Clone)]
pub struct CodeActionInfo {
    pub title: String,
    pub kind: Option<String>,
    pub edits: Vec<WorkspaceTextEditInfo>,
    pub is_preferred: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceTextEditInfo {
    pub uri: String,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub new_text: String,
}

#[derive(Debug, Clone)]
pub struct RangeInfo {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}
