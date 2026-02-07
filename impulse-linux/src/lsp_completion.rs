use gtk4::prelude::*;
use sourceview5::prelude::*;

/// A bridge that connects LSP completion to GtkSourceView's completion system.
/// Instead of implementing CompletionProvider (which requires complex GObject subclassing),
/// we use the simpler approach of showing completions in a custom popup.
///
/// This struct manages the connection between the LSP completion results and the editor.
pub struct LspCompletionBridge {
    /// Sender to request completions from the LSP runtime
    request_tx: Option<tokio::sync::mpsc::UnboundedSender<LspRequest>>,
}

/// Requests that can be sent from GTK to the LSP runtime.
#[derive(Debug)]
pub enum LspRequest {
    Completion {
        uri: String,
        line: u32,
        character: u32,
    },
    Hover {
        uri: String,
        line: u32,
        character: u32,
    },
    Definition {
        uri: String,
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

/// Responses sent from the LSP runtime back to GTK.
#[derive(Debug)]
pub enum LspResponse {
    Diagnostics {
        uri: String,
        diagnostics: Vec<DiagnosticInfo>,
    },
    CompletionResult {
        items: Vec<CompletionInfo>,
    },
    HoverResult {
        contents: String,
    },
    DefinitionResult {
        uri: String,
        line: u32,
        character: u32,
    },
    ServerInitialized {
        language_id: String,
    },
    ServerError {
        language_id: String,
        message: String,
    },
    ServerExited {
        language_id: String,
    },
}

/// Simplified diagnostic info for the frontend.
#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

/// Severity level for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Simplified completion item for the frontend.
#[derive(Debug, Clone)]
pub struct CompletionInfo {
    pub label: String,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub kind: String,
}

impl LspCompletionBridge {
    pub fn new() -> Self {
        Self { request_tx: None }
    }

    pub fn set_request_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<LspRequest>) {
        self.request_tx = Some(tx);
    }

    pub fn send_request(&self, request: LspRequest) {
        if let Some(ref tx) = self.request_tx {
            let _ = tx.send(request);
        }
    }
}

/// Apply diagnostic marks and underlines to a sourceview buffer.
pub fn apply_diagnostics(
    buffer: &sourceview5::Buffer,
    _view: &sourceview5::View,
    diagnostics: &[DiagnosticInfo],
) {
    // Clear existing diagnostic marks
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_source_marks(&start, &end, Some("error"));
    buffer.remove_source_marks(&start, &end, Some("warning"));
    buffer.remove_source_marks(&start, &end, Some("info"));

    // Remove old diagnostic tags
    if let Some(tag) = buffer.tag_table().lookup("diagnostic-error") {
        buffer.remove_tag(&tag, &start, &end);
    }
    if let Some(tag) = buffer.tag_table().lookup("diagnostic-warning") {
        buffer.remove_tag(&tag, &start, &end);
    }
    if let Some(tag) = buffer.tag_table().lookup("diagnostic-info") {
        buffer.remove_tag(&tag, &start, &end);
    }

    // Ensure diagnostic tags exist
    ensure_diagnostic_tags(buffer);

    for diag in diagnostics {
        let line = diag.line as i32;
        if line >= buffer.line_count() {
            continue;
        }

        // Add source mark on the line
        let iter = match buffer.iter_at_line(line) {
            Some(it) => it,
            None => buffer.start_iter(),
        };
        let category = match diag.severity {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
            _ => "info",
        };
        buffer.create_source_mark(None, category, &iter);

        // Apply underline tag to the range
        let tag_name = match diag.severity {
            DiagnosticSeverity::Error => "diagnostic-error",
            DiagnosticSeverity::Warning => "diagnostic-warning",
            _ => "diagnostic-info",
        };

        if let Some(start_iter) = buffer.iter_at_line_offset(line, diag.character as i32) {
            let end_line = diag.end_line as i32;
            let end_iter = if end_line < buffer.line_count() {
                buffer
                    .iter_at_line_offset(end_line, diag.end_character as i32)
                    .unwrap_or(buffer.end_iter())
            } else {
                buffer.end_iter()
            };
            buffer.apply_tag_by_name(tag_name, &start_iter, &end_iter);
        }
    }
}

fn ensure_diagnostic_tags(buffer: &sourceview5::Buffer) {
    let table = buffer.tag_table();

    if table.lookup("diagnostic-error").is_none() {
        let tag = gtk4::TextTag::builder()
            .name("diagnostic-error")
            .underline(gtk4::pango::Underline::Error)
            .build();
        table.add(&tag);
    }

    if table.lookup("diagnostic-warning").is_none() {
        let tag = gtk4::TextTag::builder()
            .name("diagnostic-warning")
            .underline(gtk4::pango::Underline::Error)
            .build();
        table.add(&tag);
    }

    if table.lookup("diagnostic-info").is_none() {
        let tag = gtk4::TextTag::builder()
            .name("diagnostic-info")
            .underline(gtk4::pango::Underline::Single)
            .build();
        table.add(&tag);
    }
}

/// Clear all diagnostic marks and tags from a buffer.
pub fn clear_diagnostics(buffer: &sourceview5::Buffer) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_source_marks(&start, &end, Some("error"));
    buffer.remove_source_marks(&start, &end, Some("warning"));
    buffer.remove_source_marks(&start, &end, Some("info"));

    if let Some(tag) = buffer.tag_table().lookup("diagnostic-error") {
        buffer.remove_tag(&tag, &start, &end);
    }
    if let Some(tag) = buffer.tag_table().lookup("diagnostic-warning") {
        buffer.remove_tag(&tag, &start, &end);
    }
    if let Some(tag) = buffer.tag_table().lookup("diagnostic-info") {
        buffer.remove_tag(&tag, &start, &end);
    }
}
