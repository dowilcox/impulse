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

/// Show a completion popup near the cursor in the given view.
/// Inserts the selected completion text when an item is activated.
pub fn show_completion_popup(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    items: &[CompletionInfo],
) {
    if items.is_empty() {
        return;
    }

    // Get cursor position
    let insert_mark = buffer.get_insert();
    let iter = buffer.iter_at_mark(&insert_mark);
    let rect = view.iter_location(&iter);
    let (wx, wy) = view.buffer_to_window_coords(gtk4::TextWindowType::Widget, rect.x(), rect.y());

    let popover = gtk4::Popover::new();
    popover.set_parent(view);
    popover.set_autohide(true);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(wx, wy + rect.height(), 1, 1)));

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Browse);
    list_box.add_css_class("completion-list");

    for item in items.iter().take(20) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_margin_start(6);
        row.set_margin_end(6);
        row.set_margin_top(2);
        row.set_margin_bottom(2);

        let kind_label = gtk4::Label::new(Some(&item.kind));
        kind_label.add_css_class("completion-kind");
        kind_label.set_width_chars(3);
        row.append(&kind_label);

        let name_label = gtk4::Label::new(Some(&item.label));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        row.append(&name_label);

        if let Some(ref detail) = item.detail {
            let detail_label = gtk4::Label::new(Some(detail));
            detail_label.add_css_class("completion-detail");
            detail_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            detail_label.set_max_width_chars(40);
            row.append(&detail_label);
        }

        list_box.append(&row);
    }

    // Select first row
    if let Some(first) = list_box.row_at_index(0) {
        list_box.select_row(Some(&first));
    }

    // Handle item activation (Enter/click)
    {
        let buf = buffer.clone();
        let pop = popover.clone();
        let completion_items: Vec<CompletionInfo> = items.iter().take(20).cloned().collect();
        list_box.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            if let Some(item) = completion_items.get(idx) {
                let text = item.insert_text.as_deref().unwrap_or(&item.label);
                // Find the start of the current word to replace
                let insert_mark = buf.get_insert();
                let mut end_iter = buf.iter_at_mark(&insert_mark);
                let mut start_iter = end_iter;
                // Walk backwards to find word start
                while start_iter.backward_char() {
                    let ch = start_iter.char();
                    if !ch.is_alphanumeric() && ch != '_' {
                        start_iter.forward_char();
                        break;
                    }
                }
                buf.delete(&mut start_iter, &mut end_iter);
                buf.insert(&mut start_iter, text);
            }
            pop.popdown();
        });
    }

    // Keyboard navigation: Up/Down to move selection, Enter/Tab to accept, Escape to dismiss
    {
        let lb = list_box.clone();
        let buf = buffer.clone();
        let pop = popover.clone();
        let completion_items: Vec<CompletionInfo> = items.iter().take(20).cloned().collect();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            use gtk4::gdk::Key;
            match key {
                k if k == Key::Up => {
                    if let Some(selected) = lb.selected_row() {
                        let idx = selected.index();
                        if idx > 0 {
                            if let Some(prev) = lb.row_at_index(idx - 1) {
                                lb.select_row(Some(&prev));
                                prev.grab_focus();
                            }
                        }
                    }
                    gtk4::glib::Propagation::Stop
                }
                k if k == Key::Down => {
                    if let Some(selected) = lb.selected_row() {
                        let idx = selected.index();
                        if let Some(next) = lb.row_at_index(idx + 1) {
                            lb.select_row(Some(&next));
                            next.grab_focus();
                        }
                    }
                    gtk4::glib::Propagation::Stop
                }
                k if k == Key::Return || k == Key::Tab => {
                    if let Some(selected) = lb.selected_row() {
                        let idx = selected.index() as usize;
                        if let Some(item) = completion_items.get(idx) {
                            let text = item.insert_text.as_deref().unwrap_or(&item.label);
                            let insert_mark = buf.get_insert();
                            let mut end_iter = buf.iter_at_mark(&insert_mark);
                            let mut start_iter = end_iter;
                            while start_iter.backward_char() {
                                let ch = start_iter.char();
                                if !ch.is_alphanumeric() && ch != '_' {
                                    start_iter.forward_char();
                                    break;
                                }
                            }
                            buf.delete(&mut start_iter, &mut end_iter);
                            buf.insert(&mut start_iter, text);
                        }
                    }
                    pop.popdown();
                    gtk4::glib::Propagation::Stop
                }
                k if k == Key::Escape => {
                    pop.popdown();
                    gtk4::glib::Propagation::Stop
                }
                _ => gtk4::glib::Propagation::Proceed,
            }
        });
        popover.add_controller(key_controller);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_max_content_height(250);
    scroll.set_propagate_natural_height(true);
    scroll.set_min_content_width(300);
    scroll.set_child(Some(&list_box));

    popover.set_child(Some(&scroll));

    // Focus the list so keyboard navigation works immediately
    list_box.set_can_focus(true);
    popover.popup();
    if let Some(first) = list_box.row_at_index(0) {
        first.grab_focus();
    }

    popover.connect_closed(move |p| {
        p.unparent();
    });
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
