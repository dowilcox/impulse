use gtk4::prelude::*;

/// Show hover information as a tooltip-style popover near the given position in the view.
///
/// This creates a transient GtkPopover anchored to the cursor position in the text view,
/// displaying the hover content from the LSP server.
pub fn show_hover_popover(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    line: u32,
    character: u32,
    content: &str,
) {
    if content.is_empty() {
        return;
    }

    // Get the location in the buffer
    let iter = match buffer.iter_at_line_offset(line as i32, character as i32) {
        Some(iter) => iter,
        None => return,
    };

    // Get the rectangle for this position
    let rect = view.iter_location(&iter);

    // Convert buffer coordinates to widget coordinates
    let (wx, wy) = view.buffer_to_window_coords(gtk4::TextWindowType::Widget, rect.x(), rect.y());

    // Create a popover with the hover content
    let popover = gtk4::Popover::new();
    popover.set_parent(view);
    popover.set_autohide(true);
    popover.set_has_arrow(true);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(wx, wy, 1, rect.height())));

    let label = gtk4::Label::new(Some(content));
    label.set_wrap(true);
    label.set_max_width_chars(80);
    label.set_selectable(true);
    label.set_xalign(0.0);
    // Use monospace font for code content
    label.add_css_class("monospace");
    label.set_margin_start(8);
    label.set_margin_end(8);
    label.set_margin_top(4);
    label.set_margin_bottom(4);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_max_content_height(300);
    scroll.set_max_content_width(600);
    scroll.set_propagate_natural_height(true);
    scroll.set_propagate_natural_width(true);
    scroll.set_child(Some(&label));

    popover.set_child(Some(&scroll));
    popover.popup();

    // Auto-close when the popover loses focus
    popover.connect_closed(move |p| {
        p.unparent();
    });
}

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
