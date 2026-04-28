/// Extract plain text from LSP hover content (MarkupContent or MarkedString).
/// This handles the various formats that LSP servers can return for hover.
pub fn extract_hover_text(contents: &str) -> String {
    // The content we receive is already extracted by the core module.
    // Just do basic cleanup: strip markdown code fences if present.
    let mut result = contents.to_string();

    // Strip leading/trailing code fences
    if result.starts_with("```") {
        if let Some(first_newline) = result.find('\n') {
            result = result[first_newline + 1..].to_string();
        }
    }
    if result.ends_with("```") {
        result = result[..result.len() - 3].trim_end().to_string();
    }

    result.trim().to_string()
}

/// Convert LSP hover response content to a displayable string.
pub fn hover_content_to_string(hover: &lsp_types::Hover) -> String {
    match &hover.contents {
        lsp_types::HoverContents::Scalar(marked_string) => marked_string_to_text(marked_string),
        lsp_types::HoverContents::Array(strings) => strings
            .iter()
            .map(marked_string_to_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        lsp_types::HoverContents::Markup(markup) => markup.value.clone(),
    }
}

fn marked_string_to_text(ms: &lsp_types::MarkedString) -> String {
    match ms {
        lsp_types::MarkedString::String(s) => s.clone(),
        lsp_types::MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}
