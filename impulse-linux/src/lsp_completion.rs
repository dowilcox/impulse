use gtk4::prelude::*;
use sourceview5::prelude::*;

fn run_guarded_ui<F: FnOnce()>(label: &str, f: F) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "non-string panic payload"
        };
        log::error!("UI callback panic in '{}': {}", label, msg);
    }
}

pub struct LspCompletionBridge {
    request_tx: Option<tokio::sync::mpsc::UnboundedSender<LspRequest>>,
}

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

pub fn apply_diagnostics(
    buffer: &sourceview5::Buffer,
    _view: &sourceview5::View,
    diagnostics: &[DiagnosticInfo],
) {
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

    ensure_diagnostic_tags(buffer);

    for diag in diagnostics {
        let line = diag.line as i32;
        if line >= buffer.line_count() {
            continue;
        }

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

fn snippet_to_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }

        if ch != '$' {
            out.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('{') => {
                let _ = chars.next();
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    inner.push(c);
                }

                if let Some((_, default)) = inner.split_once(':') {
                    out.push_str(default);
                } else if let Some((_, choices)) = inner.split_once('|') {
                    if let Some(first) = choices.split(',').next() {
                        out.push_str(first);
                    }
                }
            }
            Some(next) if next.is_ascii_digit() => {
                while let Some(peek) = chars.peek() {
                    if peek.is_ascii_digit() {
                        let _ = chars.next();
                    } else {
                        break;
                    }
                }
            }
            _ => out.push('$'),
        }
    }

    out
}

fn position_to_iter(
    buffer: &sourceview5::Buffer,
    line: u32,
    character: u32,
) -> Option<gtk4::TextIter> {
    buffer.iter_at_line_offset(line as i32, character as i32)
}

fn apply_text_edit(buffer: &sourceview5::Buffer, edit: &TextEditInfo, text: &str) {
    let Some(mut start_iter) = position_to_iter(buffer, edit.start_line, edit.start_character)
    else {
        return;
    };
    let Some(mut end_iter) = position_to_iter(buffer, edit.end_line, edit.end_character) else {
        return;
    };

    buffer.delete(&mut start_iter, &mut end_iter);
    buffer.insert(&mut start_iter, text);
}

fn apply_completion_item(buffer: &sourceview5::Buffer, item: &CompletionInfo) {
    let format = item.insert_text_format;
    let mut additional_edits = item.additional_text_edits.clone();
    additional_edits.sort_by(|a, b| {
        (b.start_line, b.start_character, b.end_line, b.end_character).cmp(&(
            a.start_line,
            a.start_character,
            a.end_line,
            a.end_character,
        ))
    });

    if let Some(main) = &item.text_edit {
        let mut all_edits = additional_edits;
        all_edits.push(main.clone());
        all_edits.sort_by(|a, b| {
            (b.start_line, b.start_character, b.end_line, b.end_character).cmp(&(
                a.start_line,
                a.start_character,
                a.end_line,
                a.end_character,
            ))
        });

        for edit in all_edits {
            let mut text = edit.new_text.clone();
            if format == Some(lsp_types::InsertTextFormat::SNIPPET) {
                text = snippet_to_text(&text);
            }
            apply_text_edit(buffer, &edit, &text);
        }
        return;
    }

    for edit in additional_edits {
        apply_text_edit(buffer, &edit, &edit.new_text);
    }

    let raw = item.insert_text.as_deref().unwrap_or(&item.label);
    let text = if format == Some(lsp_types::InsertTextFormat::SNIPPET) {
        snippet_to_text(raw)
    } else {
        raw.to_string()
    };

    let insert_mark = buffer.get_insert();
    let mut end_iter = buffer.iter_at_mark(&insert_mark);
    let mut start_iter = end_iter;

    while start_iter.backward_char() {
        let ch = start_iter.char();
        if !ch.is_alphanumeric() && ch != '_' {
            start_iter.forward_char();
            break;
        }
    }

    buffer.delete(&mut start_iter, &mut end_iter);
    buffer.insert(&mut start_iter, &text);
}

pub fn show_completion_popup(
    view: &sourceview5::View,
    buffer: &sourceview5::Buffer,
    items: &[CompletionInfo],
) {
    if items.is_empty() {
        return;
    }
    if !view.is_visible() || !view.is_mapped() || view.width() <= 0 || view.height() <= 0 {
        return;
    }

    let insert_mark = buffer.get_insert();
    let iter = buffer.iter_at_mark(&insert_mark);
    let rect = view.iter_location(&iter);
    let (wx, wy) = view.buffer_to_window_coords(gtk4::TextWindowType::Widget, rect.x(), rect.y());

    let popover = gtk4::Popover::new();
    popover.set_parent(view);
    popover.set_autohide(true);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
        wx,
        wy + rect.height(),
        1,
        1,
    )));

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

    if let Some(first) = list_box.row_at_index(0) {
        list_box.select_row(Some(&first));
    }

    {
        let buf = buffer.clone();
        let pop = popover.clone();
        let completion_items: Vec<CompletionInfo> = items.iter().take(20).cloned().collect();
        list_box.connect_row_activated(move |_, row| {
            run_guarded_ui("completion-row-activated", || {
                let idx = row.index() as usize;
                if let Some(item) = completion_items.get(idx) {
                    apply_completion_item(&buf, item);
                }
                pop.popdown();
            });
        });
    }

    {
        let lb = list_box.clone();
        let buf = buffer.clone();
        let pop = popover.clone();
        let completion_items: Vec<CompletionInfo> = items.iter().take(20).cloned().collect();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            let mut result = gtk4::glib::Propagation::Proceed;
            run_guarded_ui("completion-key-pressed", || {
                use gtk4::gdk::Key;
                result = match key {
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
                                apply_completion_item(&buf, item);
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
                };
            });
            result
        });
        popover.add_controller(key_controller);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_max_content_height(250);
    scroll.set_propagate_natural_height(true);
    scroll.set_min_content_width(300);
    scroll.set_child(Some(&list_box));

    popover.set_child(Some(&scroll));

    list_box.set_can_focus(true);
    popover.popup();
    if let Some(first) = list_box.row_at_index(0) {
        first.grab_focus();
    }

    // Let GTK manage popover lifecycle; manual unparent here can race teardown.
}

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
