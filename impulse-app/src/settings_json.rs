//! settings.json in an editor tab is checked against its schema, as the
//! Swift editor registers the schema with Monaco: problems as diagnostics,
//! each setting's description on hover, and completions for keys and
//! values, all from `impulse_kit::settings_schema`. No language server is
//! needed (the file is in the data folder, never a trusted one).

use gpui_kit::component::input::{CompletionProvider, HoverProvider, Rope, RopeExt};
use gpui_kit::{App, Task, Window};
use impulse_kit::settings_schema;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Diagnostic, DiagnosticSeverity, Documentation, Hover, HoverContents, MarkupContent, MarkupKind,
    Position, Range, TextEdit,
};

/// The name its diagnostics go under (as a server's would).
pub(crate) const SOURCE: &str = "settings.json";

/// Whether `path` is the settings file Impulse reads.
pub(crate) fn is_settings_file(path: &std::path::Path) -> bool {
    let settings = impulse_kit::settings::settings_path(cfg!(debug_assertions));
    path == settings
        || path.canonicalize().ok() == settings.canonicalize().ok().filter(|_| settings.exists())
}

/// A byte offset as an LSP position (UTF-16 columns).
fn position(text: &Rope, offset: usize) -> Position {
    let chars = text.offset_to_position(offset);
    let line = text.slice_line(chars.line as usize);
    let byte = line.char_to_byte_idx(chars.character as usize);
    Position::new(chars.line, line.byte_to_utf16_idx(byte) as u32)
}

fn range(text: &Rope, bytes: std::ops::Range<usize>) -> Range {
    Range::new(position(text, bytes.start), position(text, bytes.end))
}

/// What's wrong with the file, as diagnostics.
pub(crate) fn diagnostics(text: &Rope) -> Vec<Diagnostic> {
    settings_schema::problems(&text.to_string())
        .into_iter()
        .map(|problem| Diagnostic {
            range: range(text, problem.range),
            severity: Some(if problem.error {
                DiagnosticSeverity::ERROR
            } else {
                DiagnosticSeverity::WARNING
            }),
            source: Some(SOURCE.into()),
            message: problem.message,
            ..Default::default()
        })
        .collect()
}

/// Keys and values from the schema, and descriptions on hover.
pub(crate) struct SchemaProvider;

impl CompletionProvider for SchemaProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<gpui_kit::Result<CompletionResponse>> {
        let source = text.to_string();
        let items = settings_schema::completions(&source, offset)
            .into_iter()
            .filter(|c| {
                // What's typed of it narrows the list.
                let typed = source[c.replace.start..offset.max(c.replace.start)]
                    .trim_start_matches('"')
                    .to_lowercase();
                c.label.to_lowercase().contains(&typed)
            })
            .map(|c| CompletionItem {
                label: c.label.clone(),
                kind: Some(if c.documentation.is_some() {
                    CompletionItemKind::PROPERTY
                } else {
                    CompletionItemKind::VALUE
                }),
                detail: c.detail,
                documentation: c.documentation.map(|value| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    })
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: range(text, c.replace),
                    new_text: c.insert,
                })),
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        new_text
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '"' | ':' | ' '))
    }
}

impl HoverProvider for SchemaProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<gpui_kit::Result<Option<Hover>>> {
        let hover = settings_schema::hover(&text.to_string(), offset).map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        });
        Task::ready(Ok(hover))
    }
}
