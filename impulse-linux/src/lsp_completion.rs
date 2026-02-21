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
        text: String,
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
