use gtk4::prelude::*;
use impulse_core::command_palette::{filter_items, RecentCommandStore};
use libadwaita as adw;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::editor;
use crate::sidebar;

use super::{run_guarded_ui, Command};

pub(super) fn show_quick_open(
    window: &adw::ApplicationWindow,
    sidebar_state: &Rc<sidebar::SidebarState>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(500)
        .default_height(400)
        .build();
    dialog.add_css_class("quick-open");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Open file..."));
    vbox.append(&entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    scroll.set_child(Some(&list));
    vbox.append(&scroll);

    dialog.set_child(Some(&vbox));

    // Search on type
    let current_path = sidebar_state.current_path.clone();
    {
        let list = list.clone();
        entry.connect_search_changed(move |entry| {
            run_guarded_ui("quick-open-search-changed", || {
                let query = entry.text().to_string();
                let root = current_path.borrow().clone();
                if query.is_empty() || root.is_empty() {
                    while let Some(row) = list.row_at_index(0) {
                        list.remove(&row);
                    }
                    return;
                }
                let list = list.clone();
                gtk4::glib::spawn_future_local(async move {
                    let results = gtk4::gio::spawn_blocking(move || {
                        impulse_core::search::search_filenames(&root, &query, 30, None)
                    })
                    .await;
                    while let Some(row) = list.row_at_index(0) {
                        list.remove(&row);
                    }
                    if let Ok(Ok(results)) = results {
                        for result in &results {
                            let label = gtk4::Label::new(Some(&result.path));
                            label.set_halign(gtk4::Align::Start);
                            label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
                            // Store the full path in the widget name for retrieval
                            label.set_widget_name(&result.path);
                            list.append(&label);
                        }
                        // Select first row by default
                        if let Some(first_row) = list.row_at_index(0) {
                            list.select_row(Some(&first_row));
                        }
                    }
                });
            });
        });
    }

    // Helper: extract the file path from a selected row
    fn extract_path_from_row(row: &gtk4::ListBoxRow) -> Option<String> {
        let child = row.child()?;
        let name = child.widget_name();
        let path = name.to_string();
        if path.is_empty() || path == "GtkLabel" {
            return None;
        }
        Some(path)
    }

    // Activate file on row click
    {
        let dialog = dialog.clone();
        let on_file_activated = sidebar_state.on_file_activated.clone();
        list.connect_row_activated(move |_list, row| {
            if let Some(path) = extract_path_from_row(row) {
                if let Some(cb) = on_file_activated.borrow().as_ref() {
                    cb(&path);
                }
            }
            dialog.close();
        });
    }

    // Enter key activates selected row; Up/Down navigate between entry and list
    let key_controller = gtk4::EventControllerKey::new();
    {
        let list = list.clone();
        let dialog = dialog.clone();
        let on_file_activated = sidebar_state.on_file_activated.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                if let Some(row) = list.selected_row() {
                    if let Some(path) = extract_path_from_row(&row) {
                        if let Some(cb) = on_file_activated.borrow().as_ref() {
                            cb(&path);
                        }
                    }
                    dialog.close();
                    return gtk4::glib::Propagation::Stop;
                }
            }
            if key == gtk4::gdk::Key::Down {
                // Move selection down in the list
                if let Some(row) = list.selected_row() {
                    let idx = row.index();
                    if let Some(next) = list.row_at_index(idx + 1) {
                        list.select_row(Some(&next));
                    }
                } else if let Some(first) = list.row_at_index(0) {
                    list.select_row(Some(&first));
                }
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Up {
                // Move selection up in the list
                if let Some(row) = list.selected_row() {
                    let idx = row.index();
                    if idx > 0 {
                        if let Some(prev) = list.row_at_index(idx - 1) {
                            list.select_row(Some(&prev));
                        }
                    }
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(key_controller);

    dialog.present();
    entry.grab_focus();
}

pub(super) fn show_command_palette(
    window: &adw::ApplicationWindow,
    commands: &[Command],
    recents: &Rc<RefCell<RecentCommandStore>>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(500)
        .default_height(400)
        .build();
    dialog.add_css_class("quick-open");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Type a command..."));
    vbox.append(&entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    scroll.set_child(Some(&list));
    vbox.append(&scroll);

    dialog.set_child(Some(&vbox));

    // Populate with all commands
    let commands: Vec<Command> = commands.to_vec();
    populate_command_list(&list, &commands, "", recents);

    // Filter on type
    {
        let list = list.clone();
        let commands = commands.clone();
        let recents = recents.clone();
        entry.connect_search_changed(move |entry| {
            run_guarded_ui("command-palette-search-changed", || {
                let query = entry.text().to_string();
                populate_command_list(&list, &commands, &query, &recents);
            });
        });
    }

    // Activate command on row click
    {
        let dialog = dialog.clone();
        let commands = commands.clone();
        let recents = recents.clone();
        list.connect_row_activated(move |_list, row| {
            execute_command_for_row(row, &commands, &recents);
            dialog.close();
        });
    }

    // Enter key activates selected row
    {
        let list = list.clone();
        let dialog = dialog.clone();
        let commands = commands.clone();
        let recents = recents.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                if let Some(row) = list.selected_row() {
                    execute_command_for_row(&row, &commands, &recents);
                    dialog.close();
                    return gtk4::glib::Propagation::Stop;
                }
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_controller);
    }

    dialog.present();
    entry.grab_focus();
}

pub(super) fn show_go_to_line_dialog(
    window: &adw::ApplicationWindow,
    editor_widget: &gtk4::Widget,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(window)
        .modal(true)
        .decorated(false)
        .default_width(300)
        .default_height(60)
        .build();
    dialog.add_css_class("quick-open"); // reuse quick-open styling

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(12);
    hbox.set_margin_bottom(12);

    let label = gtk4::Label::new(Some("Go to line:"));
    let entry = gtk4::Entry::new();
    entry.set_hexpand(true);
    entry.set_input_purpose(gtk4::InputPurpose::Digits);

    hbox.append(&label);
    hbox.append(&entry);
    dialog.set_child(Some(&hbox));

    // Get total line count for placeholder
    if let Some(handle) = editor::get_handle_for_widget(editor_widget) {
        let content = handle.get_content();
        let total = content.lines().count();
        entry.set_placeholder_text(Some(&format!("1-{}", total)));
    }

    // Enter to go to line
    let editor_widget = editor_widget.clone();
    {
        let dialog = dialog.clone();
        entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if let Ok(line_num) = text.trim().parse::<u32>() {
                let line = line_num.max(1); // Monaco uses 1-based lines
                editor::go_to_position(&editor_widget, line, 1);
            }
            dialog.close();
        });
    }

    // Escape to close
    let key_controller = gtk4::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(key_controller);

    dialog.present();
    entry.grab_focus();
}

fn execute_command_for_row(
    row: &gtk4::ListBoxRow,
    commands: &[Command],
    recents: &Rc<RefCell<RecentCommandStore>>,
) {
    let Some(child) = row.child() else {
        return;
    };
    let command_id = child.widget_name().to_string();
    let Some(command) = commands
        .iter()
        .find(|command| command.item.id == command_id)
    else {
        return;
    };
    recents
        .borrow_mut()
        .record(&command.item, current_unix_time_ms(), 20);
    (command.action)();
}

fn current_unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn populate_command_list(
    list: &gtk4::ListBox,
    commands: &[Command],
    filter: &str,
    recents: &Rc<RefCell<RecentCommandStore>>,
) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    let items: Vec<_> = commands
        .iter()
        .map(|command| command.item.clone())
        .collect();
    let filtered_items = filter_items(&items, &recents.borrow(), filter);
    for item in filtered_items {
        let Some(cmd) = commands.iter().find(|command| command.item.id == item.id) else {
            continue;
        };
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.set_widget_name(&cmd.item.id);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(4);
        row.set_margin_bottom(4);

        let name_label = gtk4::Label::new(Some(&cmd.item.title));
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);
        row.append(&name_label);

        if !cmd.shortcut.is_empty() {
            let shortcut_label = gtk4::Label::new(Some(&cmd.shortcut));
            shortcut_label.add_css_class("dim-label");
            row.append(&shortcut_label);
        }

        list.append(&row);
    }

    // Select first row by default
    if let Some(first_row) = list.row_at_index(0) {
        list.select_row(Some(&first_row));
    }
}
