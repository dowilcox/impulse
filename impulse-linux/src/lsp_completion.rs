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
