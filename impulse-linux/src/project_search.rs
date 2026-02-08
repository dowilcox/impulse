use gtk4::prelude::*;
use gtk4::{gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use impulse_core::search::SearchResult;

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

/// State for the project-wide search panel, used to wire callbacks from window.rs.
pub struct ProjectSearchState {
    pub widget: gtk4::Box,
    pub search_entry: gtk4::SearchEntry,
    pub replace_entry: gtk4::Entry,
    pub result_list: gtk4::ListBox,
    pub result_count_label: gtk4::Label,
    pub case_sensitive: Rc<RefCell<bool>>,
    pub on_result_activated: Rc<RefCell<Option<Box<dyn Fn(&str, u32)>>>>,
    /// Called after Replace All with the list of file paths that were modified.
    pub on_files_replaced: Rc<RefCell<Option<Box<dyn Fn(&[String])>>>>,
    pub current_results: Rc<RefCell<Vec<SearchResult>>>,
    pub current_root: Rc<RefCell<String>>,
}

/// Build the project search panel widget and return its state.
pub fn build_project_search_panel() -> ProjectSearchState {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.add_css_class("project-search-panel");

    // Search input row
    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    search_row.add_css_class("project-search-row");

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search in project..."));
    search_entry.set_hexpand(true);

    let case_btn = gtk4::ToggleButton::with_label("Aa");
    case_btn.set_tooltip_text(Some("Case Sensitive"));
    case_btn.add_css_class("project-search-toggle");

    search_row.append(&search_entry);
    search_row.append(&case_btn);

    // Replace input row
    let replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    replace_row.add_css_class("project-search-row");

    let replace_entry = gtk4::Entry::new();
    replace_entry.set_placeholder_text(Some("Replace..."));
    replace_entry.set_hexpand(true);

    let replace_all_btn = gtk4::Button::with_label("Replace All");
    replace_all_btn.set_tooltip_text(Some("Replace all matches in project"));
    replace_all_btn.add_css_class("project-search-toggle");

    replace_row.append(&replace_entry);
    replace_row.append(&replace_all_btn);

    // Result count label
    let result_count_label = gtk4::Label::new(None);
    result_count_label.add_css_class("project-search-count");
    result_count_label.set_halign(gtk4::Align::Start);
    result_count_label.set_margin_start(8);
    result_count_label.set_margin_top(2);
    result_count_label.set_margin_bottom(2);

    // Results list in a scrolled window
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let result_list = gtk4::ListBox::new();
    result_list.set_selection_mode(gtk4::SelectionMode::Single);
    result_list.add_css_class("project-search-results");
    scroll.set_child(Some(&result_list));

    panel.append(&search_row);
    panel.append(&replace_row);
    panel.append(&result_count_label);
    panel.append(&scroll);

    let case_sensitive: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let on_result_activated: Rc<RefCell<Option<Box<dyn Fn(&str, u32)>>>> =
        Rc::new(RefCell::new(None));
    let on_files_replaced: Rc<RefCell<Option<Box<dyn Fn(&[String])>>>> =
        Rc::new(RefCell::new(None));
    let current_results: Rc<RefCell<Vec<SearchResult>>> = Rc::new(RefCell::new(Vec::new()));
    let current_root: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // Wire case toggle
    {
        let case_sensitive = case_sensitive.clone();
        case_btn.connect_toggled(move |btn| {
            *case_sensitive.borrow_mut() = btn.is_active();
        });
    }

    // Debounced search on text change
    let pending_search: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    {
        let result_list = result_list.clone();
        let result_count_label = result_count_label.clone();
        let case_sensitive = case_sensitive.clone();
        let current_results = current_results.clone();
        let current_root = current_root.clone();
        search_entry.connect_search_changed(move |entry| {
            run_guarded_ui("project-search-changed", || {
                // Cancel any pending search
                let previous_id = { pending_search.borrow_mut().take() };
                if let Some(id) = previous_id {
                    id.remove();
                }

                let query = entry.text().to_string();
                let root = current_root.borrow().clone();
                if query.is_empty() || root.is_empty() {
                    clear_list(&result_list);
                    result_count_label.set_text("");
                    current_results.borrow_mut().clear();
                    return;
                }

                let result_list = result_list.clone();
                let result_count_label = result_count_label.clone();
                let case_sensitive = *case_sensitive.borrow();
                let current_results = current_results.clone();
                let pending_clear = pending_search.clone();

                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(300),
                    move || {
                        // Clear the pending source ID so it won't be double-removed
                        let _ = pending_clear.borrow_mut().take();
                        let result_list = result_list.clone();
                        let result_count_label = result_count_label.clone();
                        let current_results = current_results.clone();
                        glib::spawn_future_local(async move {
                            let q = query.clone();
                            let r = root.clone();
                            let cs = case_sensitive;
                            let results = gio::spawn_blocking(move || {
                                impulse_core::search::search_contents(&r, &q, 500, cs)
                            })
                            .await;
                            match results {
                                Ok(Ok(results)) => {
                                    let count = results.len();
                                    *current_results.borrow_mut() = results.clone();
                                    populate_project_results(&result_list, &results);
                                    if count == 0 {
                                        result_count_label.set_text("No results");
                                    } else if count == 500 {
                                        result_count_label.set_text(&format!(
                                            "{} results (limit reached)",
                                            count
                                        ));
                                    } else {
                                        result_count_label.set_text(&format!(
                                            "{} result{}",
                                            count,
                                            if count == 1 { "" } else { "s" }
                                        ));
                                    }
                                }
                                _ => {
                                    clear_list(&result_list);
                                    result_count_label.set_text("Search error");
                                    current_results.borrow_mut().clear();
                                }
                            }
                        });
                    },
                );
                *pending_search.borrow_mut() = Some(id);
            });
        });
    }

    // Also re-trigger search when case toggle changes
    {
        let search_entry = search_entry.clone();
        case_btn.connect_toggled(move |_| {
            // Emit search-changed to retrigger
            search_entry.emit_by_name::<()>("search-changed", &[]);
        });
    }

    // Wire result list activation
    {
        let on_result_activated = on_result_activated.clone();
        result_list.connect_row_activated(move |_list, row| {
            if let Some(child) = row.child() {
                let path = child.widget_name().to_string();
                if !path.is_empty() {
                    // Extract line number from tooltip-text (stored as "line:N")
                    let line = child
                        .tooltip_text()
                        .and_then(|t| t.to_string().parse::<u32>().ok())
                        .unwrap_or(1);
                    if let Some(cb) = on_result_activated.borrow().as_ref() {
                        cb(&path, line);
                    }
                }
            }
        });
    }

    // Wire Replace All button
    {
        let search_entry = search_entry.clone();
        let replace_entry = replace_entry.clone();
        let case_sensitive = case_sensitive.clone();
        let current_results = current_results.clone();
        let result_list = result_list.clone();
        let result_count_label = result_count_label.clone();
        let on_files_replaced = on_files_replaced.clone();
        replace_all_btn.connect_clicked(move |_| {
            let search_text = search_entry.text().to_string();
            let replace_text = replace_entry.text().to_string();
            if search_text.is_empty() {
                return;
            }

            // Collect unique file paths from current results
            let results = current_results.borrow();
            let mut paths: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for r in results.iter() {
                if seen.insert(r.path.clone()) {
                    paths.push(r.path.clone());
                }
            }
            drop(results);

            let cs = *case_sensitive.borrow();
            let result_list = result_list.clone();
            let result_count_label = result_count_label.clone();
            let current_results = current_results.clone();
            let search_entry_ref = search_entry.clone();
            let on_files_replaced = on_files_replaced.clone();
            let replaced_paths = paths.clone();

            glib::spawn_future_local(async move {
                let st = search_text.clone();
                let rt = replace_text;
                let p = paths;
                let result = gio::spawn_blocking(move || {
                    impulse_core::search::replace_in_files(&p, &st, &rt, cs)
                })
                .await;

                match result {
                    Ok(Ok(count)) => {
                        result_count_label.set_text(&format!(
                            "Replaced {} occurrence{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        ));
                        // Notify window to refresh open editor buffers
                        if let Some(cb) = on_files_replaced.borrow().as_ref() {
                            cb(&replaced_paths);
                        }
                        // Re-trigger search to refresh results
                        clear_list(&result_list);
                        current_results.borrow_mut().clear();
                        search_entry_ref.emit_by_name::<()>("search-changed", &[]);
                    }
                    Ok(Err(e)) => {
                        result_count_label.set_text(&format!("Replace error: {}", e));
                    }
                    Err(_) => {
                        result_count_label.set_text("Replace failed");
                    }
                }
            });
        });
    }

    ProjectSearchState {
        widget: panel,
        search_entry,
        replace_entry,
        result_list,
        result_count_label,
        case_sensitive,
        on_result_activated,
        on_files_replaced,
        current_results,
        current_root,
    }
}

/// Populate the result list grouped by file.
fn populate_project_results(list: &gtk4::ListBox, results: &[SearchResult]) {
    clear_list(list);

    // Group results by file path
    let mut grouped: Vec<(String, String, Vec<&SearchResult>)> = Vec::new();
    let mut file_map: HashMap<String, usize> = HashMap::new();

    for result in results {
        if let Some(&idx) = file_map.get(&result.path) {
            grouped[idx].2.push(result);
        } else {
            let idx = grouped.len();
            file_map.insert(result.path.clone(), idx);
            grouped.push((result.path.clone(), result.name.clone(), vec![result]));
        }
    }

    for (file_path, file_name, matches) in &grouped {
        // File header row
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        header.add_css_class("project-search-file-header");
        // Don't set widget_name to a path since this is a header, not clickable to open

        let icon = gtk4::Image::from_icon_name("text-x-generic-symbolic");
        icon.set_pixel_size(14);
        header.append(&icon);

        let name_label = gtk4::Label::new(Some(file_name));
        name_label.add_css_class("project-search-filename");
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        header.append(&name_label);

        let count_label = gtk4::Label::new(Some(&format!("{}", matches.len())));
        count_label.add_css_class("project-search-match-count");
        header.append(&count_label);

        let header_row = gtk4::ListBoxRow::new();
        header_row.set_selectable(false);
        header_row.set_activatable(false);
        header_row.set_child(Some(&header));
        list.append(&header_row);

        // Match rows
        for m in matches {
            let match_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            match_box.add_css_class("project-search-match");
            match_box.set_widget_name(file_path);
            match_box.set_cursor_from_name(Some("pointer"));

            // Store line number in tooltip for retrieval on activation
            if let Some(line_num) = m.line_number {
                match_box.set_tooltip_text(Some(&format!("{}", line_num)));
            }

            if let Some(line_num) = m.line_number {
                let line_label = gtk4::Label::new(Some(&format!("{}", line_num)));
                line_label.add_css_class("project-search-line-num");
                line_label.set_width_chars(5);
                line_label.set_xalign(1.0);
                match_box.append(&line_label);
            }

            if let Some(ref content) = m.line_content {
                let content_label = gtk4::Label::new(Some(content.trim()));
                content_label.add_css_class("project-search-line-content");
                content_label.set_halign(gtk4::Align::Start);
                content_label.set_hexpand(true);
                content_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                match_box.append(&content_label);
            }

            list.append(&match_box);
        }
    }
}

fn clear_list(list: &gtk4::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}
