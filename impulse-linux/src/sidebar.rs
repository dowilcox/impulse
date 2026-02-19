use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use crate::file_icons::IconCache;
use crate::project_search;
use crate::settings;
use crate::theme::ThemeColors;

type EventCallback = Rc<RefCell<Option<Box<dyn Fn(&str)>>>>;
use impulse_core::filesystem::FileEntry;

/// A node in the sidebar file tree, representing either a file or directory at a given depth.
#[derive(Clone)]
pub struct TreeNode {
    pub entry: FileEntry,
    pub depth: usize,
    pub expanded: bool,
}

/// Saved tree state for a tab, used to preserve expand/collapse and scroll position.
pub struct TabTreeState {
    pub nodes: Vec<TreeNode>,
    pub current_path: String,
    pub scroll_position: f64,
}

/// Build the sidebar widget containing file tree and search panel.
pub fn build_sidebar(
    settings: &Rc<RefCell<settings::Settings>>,
    theme: &ThemeColors,
) -> (gtk4::Box, SidebarState) {
    let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(250);

    let icon_cache: Rc<RefCell<IconCache>> = Rc::new(RefCell::new(IconCache::new(theme)));

    // Stack for File Tree / Search
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_vexpand(true);

    // Custom toggle button row instead of StackSwitcher
    let switcher_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    switcher_box.add_css_class("sidebar-switcher");
    switcher_box.set_homogeneous(true);

    let files_btn = gtk4::ToggleButton::builder()
        .label("Files")
        .active(true)
        .build();
    files_btn.add_css_class("sidebar-tab");
    files_btn.add_css_class("sidebar-tab-active");
    files_btn.set_cursor_from_name(Some("pointer"));

    let search_btn = gtk4::ToggleButton::builder()
        .label("Search")
        .active(false)
        .build();
    search_btn.add_css_class("sidebar-tab");
    search_btn.set_cursor_from_name(Some("pointer"));

    // Link them as a group
    search_btn.set_group(Some(&files_btn));

    switcher_box.append(&files_btn);
    switcher_box.append(&search_btn);

    // Toolbar row with action buttons
    let toolbar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    toolbar_box.add_css_class("sidebar-toolbar");
    toolbar_box.set_halign(gtk4::Align::End);
    toolbar_box.set_margin_end(4);
    toolbar_box.set_margin_top(2);
    toolbar_box.set_margin_bottom(2);

    let show_hidden = Rc::new(RefCell::new(settings.borrow().sidebar_show_hidden));

    let hidden_btn = gtk4::ToggleButton::new();
    hidden_btn.set_tooltip_text(Some("Toggle Hidden Files"));
    hidden_btn.set_active(settings.borrow().sidebar_show_hidden);
    hidden_btn.set_cursor_from_name(Some("pointer"));
    hidden_btn.add_css_class("flat");
    hidden_btn.add_css_class("sidebar-toolbar-btn");
    {
        let cache = icon_cache.borrow();
        let icon_name = if settings.borrow().sidebar_show_hidden {
            "toolbar-eye-open"
        } else {
            "toolbar-eye-closed"
        };
        if let Some(texture) = cache.get_toolbar_icon(icon_name) {
            hidden_btn.set_child(Some(&gtk4::Image::from_paintable(Some(texture))));
        }
    }

    let refresh_btn = gtk4::Button::new();
    refresh_btn.set_tooltip_text(Some("Refresh File Tree"));
    refresh_btn.set_cursor_from_name(Some("pointer"));
    refresh_btn.add_css_class("flat");
    refresh_btn.add_css_class("sidebar-toolbar-btn");
    if let Some(texture) = icon_cache.borrow().get_toolbar_icon("toolbar-refresh") {
        refresh_btn.set_child(Some(&gtk4::Image::from_paintable(Some(texture))));
    }

    let collapse_btn = gtk4::Button::new();
    collapse_btn.set_tooltip_text(Some("Collapse All"));
    collapse_btn.set_cursor_from_name(Some("pointer"));
    collapse_btn.add_css_class("flat");
    collapse_btn.add_css_class("sidebar-toolbar-btn");
    if let Some(texture) = icon_cache.borrow().get_toolbar_icon("toolbar-collapse") {
        collapse_btn.set_child(Some(&gtk4::Image::from_paintable(Some(texture))));
    }

    toolbar_box.append(&hidden_btn);
    toolbar_box.append(&refresh_btn);
    toolbar_box.append(&collapse_btn);

    // File tree page
    let file_tree_scroll = gtk4::ScrolledWindow::new();
    file_tree_scroll.set_vexpand(true);
    let file_tree_list = gtk4::ListBox::new();
    file_tree_list.set_selection_mode(gtk4::SelectionMode::Single);
    file_tree_list.add_css_class("file-tree");
    file_tree_scroll.set_child(Some(&file_tree_list));
    stack.add_named(&file_tree_scroll, Some("files"));

    // Create shared state early so context menu actions can reference it
    let tree_nodes: Rc<RefCell<Vec<TreeNode>>> = Rc::new(RefCell::new(Vec::new()));
    let current_path: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // --- Right-click context menu for file tree ---
    let clicked_path: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let on_open_terminal: EventCallback = Rc::new(RefCell::new(None));

    // Menu models: one for files, one for directories
    let file_menu = gio::Menu::new();
    file_menu.append(Some("Open in Default App"), Some("filetree.open"));
    file_menu.append(Some("Copy Path"), Some("filetree.copy-path"));
    file_menu.append(Some("New File"), Some("filetree.new-file"));
    file_menu.append(Some("New Folder"), Some("filetree.new-folder"));
    file_menu.append(Some("Rename"), Some("filetree.rename"));
    file_menu.append(Some("Delete"), Some("filetree.delete"));

    let file_menu_git = gio::Menu::new();
    file_menu_git.append(Some("Open in Default App"), Some("filetree.open"));
    file_menu_git.append(Some("Copy Path"), Some("filetree.copy-path"));
    file_menu_git.append(Some("New File"), Some("filetree.new-file"));
    file_menu_git.append(Some("New Folder"), Some("filetree.new-folder"));
    file_menu_git.append(Some("Rename"), Some("filetree.rename"));
    file_menu_git.append(Some("Delete"), Some("filetree.delete"));
    file_menu_git.append(Some("Discard Changes"), Some("filetree.discard-changes"));

    let dir_menu = gio::Menu::new();
    dir_menu.append(Some("Open in Terminal"), Some("filetree.open-terminal"));
    dir_menu.append(Some("Copy Path"), Some("filetree.copy-path"));
    dir_menu.append(Some("New File"), Some("filetree.new-file"));
    dir_menu.append(Some("New Folder"), Some("filetree.new-folder"));
    dir_menu.append(Some("Rename"), Some("filetree.rename"));
    dir_menu.append(Some("Delete"), Some("filetree.delete"));

    // Create popover menu
    let popover = gtk4::PopoverMenu::from_model(Some(&file_menu));
    popover.set_parent(&file_tree_list);
    popover.set_has_arrow(false);

    // Action group for filetree context menu actions
    let action_group = gio::SimpleActionGroup::new();

    // "open" action - opens file in default app
    let open_action = gio::SimpleAction::new("open", None);
    {
        let clicked_path = clicked_path.clone();
        open_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if !path.is_empty() {
                let file = gio::File::for_path(&path);
                let uri = file.uri();
                let _ = gio::AppInfo::launch_default_for_uri(uri.as_str(), None::<&gio::AppLaunchContext>);
            }
        });
    }
    action_group.add_action(&open_action);

    // "copy-path" action - copies path to clipboard
    let copy_action = gio::SimpleAction::new("copy-path", None);
    {
        let clicked_path = clicked_path.clone();
        let list_ref = file_tree_list.clone();
        copy_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if !path.is_empty() {
                list_ref.clipboard().set_text(&path);
            }
        });
    }
    action_group.add_action(&copy_action);

    // "open-terminal" action - opens directory in a new terminal tab
    let open_terminal_action = gio::SimpleAction::new("open-terminal", None);
    {
        let clicked_path = clicked_path.clone();
        let on_open_terminal = on_open_terminal.clone();
        open_terminal_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if !path.is_empty() {
                if let Some(ref callback) = *on_open_terminal.borrow() {
                    callback(&path);
                }
            }
        });
    }
    action_group.add_action(&open_terminal_action);

    // "rename" action - rename file or directory via inline dialog
    let rename_action = gio::SimpleAction::new("rename", None);
    {
        let clicked_path = clicked_path.clone();
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = file_tree_list.clone();
        let icon_cache = icon_cache.clone();
        rename_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if path.is_empty() {
                return;
            }

            let old_name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let parent = std::path::Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let dialog = gtk4::Window::builder()
                .modal(true)
                .decorated(false)
                .default_width(300)
                .default_height(50)
                .build();
            if let Some(root) = file_tree_list.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.set_transient_for(Some(window));
                }
            }
            dialog.add_css_class("quick-open");

            let entry = gtk4::Entry::new();
            entry.set_text(&old_name);
            entry.set_margin_start(12);
            entry.set_margin_end(12);
            entry.set_margin_top(12);
            entry.set_margin_bottom(12);
            dialog.set_child(Some(&entry));

            // Select the name without extension
            if let Some(dot_idx) = old_name.rfind('.') {
                entry.select_region(0, dot_idx as i32);
            } else {
                entry.select_region(0, -1);
            }

            let tree_nodes = tree_nodes.clone();
            let file_tree_list = file_tree_list.clone();
            let icon_cache = icon_cache.clone();
            {
                let dialog = dialog.clone();
                let parent = parent.clone();
                entry.connect_activate(move |entry| {
                    let new_name = entry.text().to_string();
                    if !new_name.is_empty() && new_name != old_name {
                        // Validate filename is a single valid component
                        let path_check = Path::new(&new_name);
                        if path_check.file_name() != Some(path_check.as_os_str())
                            || new_name.contains('\0')
                        {
                            log::error!("Invalid filename: must be a single filename component");
                            dialog.close();
                            return;
                        }
                        let new_path = std::path::Path::new(&parent).join(&new_name);
                        if let Err(e) = std::fs::rename(&path, &new_path) {
                            log::error!("Failed to rename: {}", e);
                        } else {
                            let mut nodes = tree_nodes.borrow_mut();
                            if let Some(node) = nodes.iter_mut().find(|n| n.entry.path == path) {
                                node.entry.path = new_path.to_string_lossy().to_string();
                                node.entry.name = new_name;
                            }
                            let snapshot: Vec<_> = nodes.clone();
                            drop(nodes);
                            render_tree(&file_tree_list, &snapshot, &icon_cache.borrow());
                        }
                    }
                    dialog.close();
                });
            }

            // Escape to cancel
            let key_ctrl = gtk4::EventControllerKey::new();
            {
                let dialog = dialog.clone();
                key_ctrl.connect_key_pressed(move |_, key, _, _| {
                    if key == gtk4::gdk::Key::Escape {
                        dialog.close();
                        return gtk4::glib::Propagation::Stop;
                    }
                    gtk4::glib::Propagation::Proceed
                });
            }
            entry.add_controller(key_ctrl);

            dialog.present();
            entry.grab_focus();
        });
    }
    action_group.add_action(&rename_action);

    // "delete" action - delete file or directory with confirmation
    let delete_action = gio::SimpleAction::new("delete", None);
    {
        let clicked_path = clicked_path.clone();
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = file_tree_list.clone();
        let icon_cache = icon_cache.clone();
        delete_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if path.is_empty() {
                return;
            }

            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item")
                .to_string();
            let is_dir = std::path::Path::new(&path).is_dir();

            let dialog = adw::AlertDialog::builder()
                .heading("Delete File")
                .body(format!(
                    "Are you sure you want to delete \"{}\"?{}",
                    filename,
                    if is_dir {
                        " This will delete the directory and all its contents."
                    } else {
                        ""
                    }
                ))
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("delete", "Delete");
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let tree_nodes = tree_nodes.clone();
            let file_tree_list_for_response = file_tree_list.clone();
            let icon_cache = icon_cache.clone();
            dialog.connect_response(None, move |_dialog, response| {
                if response != "delete" {
                    return;
                }

                let result = if is_dir {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };

                match result {
                    Ok(()) => {
                        let mut nodes = tree_nodes.borrow_mut();
                        // Remove the node and any descendants (for directories)
                        if let Some(idx) = nodes.iter().position(|n| n.entry.path == path) {
                            let depth = nodes[idx].depth;
                            let mut end = idx + 1;
                            while end < nodes.len() && nodes[end].depth > depth {
                                end += 1;
                            }
                            nodes.drain(idx..end);
                        }
                        let snapshot: Vec<_> = nodes.clone();
                        drop(nodes);
                        render_tree(
                            &file_tree_list_for_response,
                            &snapshot,
                            &icon_cache.borrow(),
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to delete {}: {}", path, e);
                    }
                }
            });

            // Present on the nearest window
            if let Some(root) = file_tree_list.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.present(Some(window));
                }
            }
        });
    }
    action_group.add_action(&delete_action);

    // "discard-changes" action - revert file to HEAD version
    let discard_action = gio::SimpleAction::new("discard-changes", None);
    {
        let clicked_path = clicked_path.clone();
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = file_tree_list.clone();
        let icon_cache = icon_cache.clone();
        let current_path = current_path.clone();
        let file_tree_scroll = file_tree_scroll.clone();
        let show_hidden = show_hidden.clone();
        discard_action.connect_activate(move |_, _| {
            let path = clicked_path.borrow().clone();
            if path.is_empty() {
                return;
            }

            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();

            let dialog = adw::AlertDialog::builder()
                .heading("Discard Changes")
                .body(format!(
                    "Are you sure you want to discard all changes to \"{}\"? This cannot be undone.",
                    filename
                ))
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("discard", "Discard");
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let tree_nodes = tree_nodes.clone();
            let file_tree_list_for_dialog = file_tree_list.clone();
            let file_tree_scroll = file_tree_scroll.clone();
            let current_path = current_path.clone();
            let icon_cache = icon_cache.clone();
            let show_hidden = show_hidden.clone();
            dialog.connect_response(None, move |_dialog, response| {
                if response != "discard" {
                    return;
                }

                match impulse_core::git::discard_file_changes(&path, &current_path.borrow()) {
                    Ok(()) => {
                        // Reload the file in the editor if it's open
                        if let Some(handle) = crate::editor::get_handle(&path) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                let lang = handle.language.borrow().clone();
                                handle.suppress_next_modify.set(true);
                                handle.open_file(&path, &content, &lang);
                                // Refresh diff decorations (should now be empty)
                                crate::window::send_diff_decorations(&handle, &path);
                            }
                        }
                        refresh_tree(
                            &tree_nodes,
                            &file_tree_list_for_dialog,
                            &file_tree_scroll,
                            &current_path,
                            *show_hidden.borrow(),
                            icon_cache.clone(),
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to discard changes for {}: {}", path, e);
                    }
                }
            });

            if let Some(root) = file_tree_list.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.present(Some(window));
                }
            }
        });
    }
    action_group.add_action(&discard_action);

    // "new-file" action - create a new file in a directory
    let new_file_action = gio::SimpleAction::new("new-file", None);
    {
        let clicked_path = clicked_path.clone();
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = file_tree_list.clone();
        let current_path = current_path.clone();
        let icon_cache = icon_cache.clone();
        new_file_action.connect_activate(move |_, _| {
            let clicked = clicked_path.borrow().clone();
            if clicked.is_empty() {
                return;
            }
            // Resolve to parent directory if the clicked path is a file
            let dir_path = if std::path::Path::new(&clicked).is_dir() {
                clicked
            } else {
                std::path::Path::new(&clicked)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            };

            let dialog = gtk4::Window::builder()
                .modal(true)
                .decorated(false)
                .default_width(300)
                .default_height(50)
                .build();
            if let Some(root) = file_tree_list.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.set_transient_for(Some(window));
                }
            }
            dialog.add_css_class("quick-open");

            let entry = gtk4::Entry::new();
            entry.set_placeholder_text(Some("New file name..."));
            entry.set_margin_start(12);
            entry.set_margin_end(12);
            entry.set_margin_top(12);
            entry.set_margin_bottom(12);
            dialog.set_child(Some(&entry));

            let tree_nodes = tree_nodes.clone();
            let file_tree_list = file_tree_list.clone();
            let current_path = current_path.clone();
            let icon_cache = icon_cache.clone();
            {
                let dialog = dialog.clone();
                let dir_path = dir_path.clone();
                entry.connect_activate(move |entry| {
                    let name = entry.text().to_string();
                    if !name.is_empty() {
                        // Validate filename is a single valid component
                        let path_check = Path::new(&name);
                        if path_check.file_name() != Some(path_check.as_os_str())
                            || name.contains('\0')
                        {
                            log::error!("Invalid filename: must be a single filename component");
                            dialog.close();
                            return;
                        }
                        let new_path = std::path::Path::new(&dir_path).join(&name);
                        if let Err(e) = std::fs::write(&new_path, "") {
                            log::error!("Failed to create file: {}", e);
                        } else {
                            insert_new_entry_into_tree(
                                &tree_nodes,
                                &file_tree_list,
                                &current_path,
                                &dir_path,
                                &name,
                                &new_path.to_string_lossy(),
                                false,
                                &icon_cache.borrow(),
                            );
                        }
                    }
                    dialog.close();
                });
            }

            let key_ctrl = gtk4::EventControllerKey::new();
            {
                let dialog = dialog.clone();
                key_ctrl.connect_key_pressed(move |_, key, _, _| {
                    if key == gtk4::gdk::Key::Escape {
                        dialog.close();
                        return gtk4::glib::Propagation::Stop;
                    }
                    gtk4::glib::Propagation::Proceed
                });
            }
            entry.add_controller(key_ctrl);

            dialog.present();
            entry.grab_focus();
        });
    }
    action_group.add_action(&new_file_action);

    // "new-folder" action - create a new folder in a directory
    let new_folder_action = gio::SimpleAction::new("new-folder", None);
    {
        let clicked_path = clicked_path.clone();
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = file_tree_list.clone();
        let current_path = current_path.clone();
        let icon_cache = icon_cache.clone();
        new_folder_action.connect_activate(move |_, _| {
            let clicked = clicked_path.borrow().clone();
            if clicked.is_empty() {
                return;
            }
            // Resolve to parent directory if the clicked path is a file
            let dir_path = if std::path::Path::new(&clicked).is_dir() {
                clicked
            } else {
                std::path::Path::new(&clicked)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            };

            let dialog = gtk4::Window::builder()
                .modal(true)
                .decorated(false)
                .default_width(300)
                .default_height(50)
                .build();
            if let Some(root) = file_tree_list.root() {
                if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                    dialog.set_transient_for(Some(window));
                }
            }
            dialog.add_css_class("quick-open");

            let entry = gtk4::Entry::new();
            entry.set_placeholder_text(Some("New folder name..."));
            entry.set_margin_start(12);
            entry.set_margin_end(12);
            entry.set_margin_top(12);
            entry.set_margin_bottom(12);
            dialog.set_child(Some(&entry));

            let tree_nodes = tree_nodes.clone();
            let file_tree_list = file_tree_list.clone();
            let current_path = current_path.clone();
            let icon_cache = icon_cache.clone();
            {
                let dialog = dialog.clone();
                let dir_path = dir_path.clone();
                entry.connect_activate(move |entry| {
                    let name = entry.text().to_string();
                    if !name.is_empty() {
                        // Validate filename is a single valid component
                        let path_check = Path::new(&name);
                        if path_check.file_name() != Some(path_check.as_os_str())
                            || name.contains('\0')
                        {
                            log::error!("Invalid filename: must be a single filename component");
                            dialog.close();
                            return;
                        }
                        let new_path = std::path::Path::new(&dir_path).join(&name);
                        if let Err(e) = std::fs::create_dir(&new_path) {
                            log::error!("Failed to create folder: {}", e);
                        } else {
                            insert_new_entry_into_tree(
                                &tree_nodes,
                                &file_tree_list,
                                &current_path,
                                &dir_path,
                                &name,
                                &new_path.to_string_lossy(),
                                true,
                                &icon_cache.borrow(),
                            );
                        }
                    }
                    dialog.close();
                });
            }

            let key_ctrl = gtk4::EventControllerKey::new();
            {
                let dialog = dialog.clone();
                key_ctrl.connect_key_pressed(move |_, key, _, _| {
                    if key == gtk4::gdk::Key::Escape {
                        dialog.close();
                        return gtk4::glib::Propagation::Stop;
                    }
                    gtk4::glib::Propagation::Proceed
                });
            }
            entry.add_controller(key_ctrl);

            dialog.present();
            entry.grab_focus();
        });
    }
    action_group.add_action(&new_folder_action);

    file_tree_list.insert_action_group("filetree", Some(&action_group));

    // Right-click gesture
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3); // right click
    {
        let popover = popover.clone();
        let file_tree_list_ref = file_tree_list.clone();
        let clicked_path = clicked_path.clone();
        let file_menu = file_menu.clone();
        let file_menu_git = file_menu_git.clone();
        let dir_menu = dir_menu.clone();
        let current_path = current_path.clone();
        let tree_nodes_for_menu = tree_nodes.clone();
        gesture.connect_pressed(move |_gesture, _n_press, x, y| {
            if let Some(row) = file_tree_list_ref.row_at_y(y as i32) {
                if let Some(child) = row.child() {
                    let path = child.widget_name().to_string();
                    let is_dir = std::path::Path::new(&path).is_dir();
                    *clicked_path.borrow_mut() = path.clone();

                    if is_dir {
                        popover.set_menu_model(Some(&dir_menu));
                    } else {
                        // Check if file has git changes
                        let has_git_status = tree_nodes_for_menu
                            .borrow()
                            .iter()
                            .any(|n| n.entry.path == path && n.entry.git_status.is_some());
                        if has_git_status {
                            popover.set_menu_model(Some(&file_menu_git));
                        } else {
                            popover.set_menu_model(Some(&file_menu));
                        }
                    }

                    let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                    popover.set_pointing_to(Some(&rect));
                    popover.popup();
                    return;
                }
            }
            // Right-clicked on empty space — show dir menu for current directory
            let cur = current_path.borrow().clone();
            if !cur.is_empty() {
                *clicked_path.borrow_mut() = cur;
                popover.set_menu_model(Some(&dir_menu));
                let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                popover.set_pointing_to(Some(&rect));
                popover.popup();
            }
        });
    }
    file_tree_list.add_controller(gesture);

    // Enable dragging file paths from the tree
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gtk4::gdk::DragAction::COPY | gtk4::gdk::DragAction::MOVE);
    {
        let file_tree_list = file_tree_list.clone();
        drag_source.connect_prepare(move |_source, _x, y| {
            if let Some(row) = file_tree_list.row_at_y(y as i32) {
                if let Some(child) = row.child() {
                    let path = child.widget_name().to_string();
                    if !path.is_empty() {
                        let content = gtk4::gdk::ContentProvider::for_value(&path.to_value());
                        return Some(content);
                    }
                }
            }
            None
        });
    }
    file_tree_list.add_controller(drag_source);

    // Accept internal moves: string paths from our DragSource
    let drop_target_internal =
        gtk4::DropTarget::new(glib::types::Type::STRING, gtk4::gdk::DragAction::MOVE);
    {
        let file_tree_list = file_tree_list.clone();
        drop_target_internal.connect_motion(move |_target, _x, y| {
            remove_drop_highlights(&file_tree_list);
            if let Some(row) = file_tree_list.row_at_y(y as i32) {
                row.add_css_class("drop-target");
            }
            gtk4::gdk::DragAction::MOVE
        });
    }
    {
        let file_tree_list = file_tree_list.clone();
        drop_target_internal.connect_leave(move |_target| {
            remove_drop_highlights(&file_tree_list);
        });
    }
    {
        let file_tree_list = file_tree_list.clone();
        let tree_nodes = tree_nodes.clone();
        let current_path = current_path.clone();
        let file_tree_scroll = file_tree_scroll.clone();
        let show_hidden = show_hidden.clone();
        let icon_cache = icon_cache.clone();
        drop_target_internal.connect_drop(move |_target, value, _x, y| {
            remove_drop_highlights(&file_tree_list);
            if let Ok(source_path) = value.get::<String>() {
                let cur = current_path.borrow().clone();
                if let Some(target_dir) = resolve_drop_target_dir(&file_tree_list, y, &cur) {
                    match perform_internal_move(&source_path, &target_dir) {
                        Ok(()) => {
                            refresh_tree(
                                &tree_nodes,
                                &file_tree_list,
                                &file_tree_scroll,
                                &current_path,
                                *show_hidden.borrow(),
                                icon_cache.clone(),
                            );
                            return true;
                        }
                        Err(e) => {
                            log::warn!("Internal move failed: {}", e);
                        }
                    }
                }
            }
            false
        });
    }
    file_tree_list.add_controller(drop_target_internal);

    // Accept external file drops from file managers
    let drop_target_external = gtk4::DropTarget::new(
        gtk4::gdk::FileList::static_type(),
        gtk4::gdk::DragAction::COPY,
    );
    {
        let file_tree_list = file_tree_list.clone();
        drop_target_external.connect_motion(move |_target, _x, y| {
            remove_drop_highlights(&file_tree_list);
            if let Some(row) = file_tree_list.row_at_y(y as i32) {
                row.add_css_class("drop-target");
            }
            gtk4::gdk::DragAction::COPY
        });
    }
    {
        let file_tree_list = file_tree_list.clone();
        drop_target_external.connect_leave(move |_target| {
            remove_drop_highlights(&file_tree_list);
        });
    }
    {
        let file_tree_list = file_tree_list.clone();
        let tree_nodes = tree_nodes.clone();
        let current_path = current_path.clone();
        let file_tree_scroll = file_tree_scroll.clone();
        let show_hidden = show_hidden.clone();
        let icon_cache = icon_cache.clone();
        drop_target_external.connect_drop(move |_target, value, _x, y| {
            remove_drop_highlights(&file_tree_list);
            if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                let cur = current_path.borrow().clone();
                if let Some(target_dir) = resolve_drop_target_dir(&file_tree_list, y, &cur) {
                    let mut any_success = false;
                    for file in file_list.files() {
                        if let Some(path) = file.path() {
                            let source = path.to_string_lossy().to_string();
                            match perform_external_copy(&source, &target_dir) {
                                Ok(()) => any_success = true,
                                Err(e) => {
                                    log::warn!("External copy failed for {}: {}", source, e);
                                }
                            }
                        }
                    }
                    if any_success {
                        refresh_tree(
                            &tree_nodes,
                            &file_tree_list,
                            &file_tree_scroll,
                            &current_path,
                            *show_hidden.borrow(),
                            icon_cache.clone(),
                        );
                        return true;
                    }
                }
            }
            false
        });
    }
    file_tree_list.add_controller(drop_target_external);

    // Search page: project-wide find and replace
    let project_search_state = project_search::build_project_search_panel();
    stack.add_named(&project_search_state.widget, Some("search"));

    // Wire up toggle buttons to switch stack pages
    {
        let stack = stack.clone();
        let search_btn_ref = search_btn.clone();
        let toolbar_box = toolbar_box.clone();
        files_btn.connect_toggled(move |btn: &gtk4::ToggleButton| {
            if btn.is_active() {
                stack.set_visible_child_name("files");
                btn.add_css_class("sidebar-tab-active");
                search_btn_ref.remove_css_class("sidebar-tab-active");
                toolbar_box.set_visible(true);
            }
        });
    }
    {
        let stack = stack.clone();
        let files_btn_ref = files_btn.clone();
        let toolbar_box = toolbar_box.clone();
        search_btn.connect_toggled(move |btn: &gtk4::ToggleButton| {
            if btn.is_active() {
                stack.set_visible_child_name("search");
                btn.add_css_class("sidebar-tab-active");
                files_btn_ref.remove_css_class("sidebar-tab-active");
                toolbar_box.set_visible(false);
            }
        });
    }

    sidebar.append(&switcher_box);
    sidebar.append(&toolbar_box);
    sidebar.append(&stack);

    let on_file_activated: EventCallback = Rc::new(RefCell::new(None));

    let state = SidebarState {
        file_tree_list,
        file_tree_scroll: file_tree_scroll.clone(),
        search_btn: search_btn.clone(),
        project_search: project_search_state,
        current_path: current_path.clone(),
        on_file_activated: on_file_activated.clone(),
        on_open_terminal: on_open_terminal.clone(),
        tree_nodes: tree_nodes.clone(),
        tab_tree_states: Rc::new(RefCell::new(HashMap::new())),
        active_tab: Rc::new(RefCell::new(None)),
        show_hidden: show_hidden.clone(),
        icon_cache: icon_cache.clone(),
        #[allow(clippy::arc_with_non_send_sync)]
        _watcher: Rc::new(RefCell::new(None)),
        _watcher_timer: Rc::new(RefCell::new(None)),
        #[allow(clippy::arc_with_non_send_sync)]
        _git_index_watcher: Rc::new(RefCell::new(None)),
        _git_index_timer: Rc::new(RefCell::new(None)),
        _git_status_timer: Rc::new(RefCell::new(None)),
        _last_git_status_hash: Rc::new(Cell::new(0)),
    };

    // Wire up toolbar buttons
    {
        let state_tree_nodes = state.tree_nodes.clone();
        let state_current_path = state.current_path.clone();
        let state_file_tree_list = state.file_tree_list.clone();
        let state_file_tree_scroll = state.file_tree_scroll.clone();
        let state_show_hidden = state.show_hidden.clone();
        let icon_cache = icon_cache.clone();
        refresh_btn.connect_clicked(move |_| {
            refresh_tree(
                &state_tree_nodes,
                &state_file_tree_list,
                &state_file_tree_scroll,
                &state_current_path,
                *state_show_hidden.borrow(),
                icon_cache.clone(),
            );
        });
    }
    {
        let state_tree_nodes = state.tree_nodes.clone();
        let state_file_tree_list = state.file_tree_list.clone();
        let icon_cache = icon_cache.clone();
        collapse_btn.connect_clicked(move |_| {
            collapse_all(
                &state_tree_nodes,
                &state_file_tree_list,
                &icon_cache.borrow(),
            );
        });
    }
    {
        let state_tree_nodes = state.tree_nodes.clone();
        let state_current_path = state.current_path.clone();
        let state_file_tree_list = state.file_tree_list.clone();
        let state_file_tree_scroll = state.file_tree_scroll.clone();
        let show_hidden = show_hidden.clone();
        let settings = settings.clone();
        let icon_cache = icon_cache.clone();
        hidden_btn.connect_toggled(move |btn| {
            let active = btn.is_active();
            *show_hidden.borrow_mut() = active;
            settings.borrow_mut().sidebar_show_hidden = active;
            let icon_name = if active {
                "toolbar-eye-open"
            } else {
                "toolbar-eye-closed"
            };
            if let Some(texture) = icon_cache.borrow().get_toolbar_icon(icon_name) {
                btn.set_child(Some(&gtk4::Image::from_paintable(Some(texture))));
            }
            refresh_tree(
                &state_tree_nodes,
                &state_file_tree_list,
                &state_file_tree_scroll,
                &state_current_path,
                active,
                icon_cache.clone(),
            );
        });
    }

    // Wire up file tree row activation for tree expand/collapse and file opening
    {
        let tree_nodes = tree_nodes.clone();
        let file_tree_list = state.file_tree_list.clone();
        let on_file_activated = on_file_activated.clone();
        let show_hidden = state.show_hidden.clone();
        let icon_cache = icon_cache.clone();
        let watcher_rc = state._watcher.clone();
        state
            .file_tree_list
            .connect_row_activated(move |_list, row| {
                let index = row.index() as usize;
                let tree_nodes_ref = tree_nodes.clone();
                let list = file_tree_list.clone();
                let on_file_activated = on_file_activated.clone();
                let show_hidden = show_hidden.clone();
                let icon_cache = icon_cache.clone();

                let node = {
                    let nodes = tree_nodes_ref.borrow();
                    if index >= nodes.len() {
                        return;
                    }
                    nodes[index].clone()
                };

                if node.entry.is_dir {
                    let cache = icon_cache.borrow();
                    if node.expanded {
                        // Collapse: remove descendant nodes and rows incrementally
                        // Stop watching this directory and any collapsed subdirectories
                        {
                            use notify::Watcher;
                            if let Some(ref mut w) = *watcher_rc.borrow_mut() {
                                let _ = w.unwatch(Path::new(&node.entry.path));
                            }
                        }
                        let mut nodes = tree_nodes_ref.borrow_mut();
                        nodes[index].expanded = false;
                        let depth = node.depth;
                        let mut remove_count = 0;
                        for i in (index + 1)..nodes.len() {
                            if nodes[i].depth > depth {
                                // Also unwatch any expanded subdirectories being collapsed
                                if nodes[i].entry.is_dir && nodes[i].expanded {
                                    use notify::Watcher;
                                    if let Some(ref mut w) = *watcher_rc.borrow_mut() {
                                        let _ = w.unwatch(Path::new(&nodes[i].entry.path));
                                    }
                                }
                                remove_count += 1;
                            } else {
                                break;
                            }
                        }
                        if remove_count > 0 {
                            nodes.drain((index + 1)..(index + 1 + remove_count));
                        }
                        // Update the directory row arrow and icon
                        update_dir_row_expanded(&list, index, nodes[index].expanded, &cache);
                        // Remove child rows from the ListBox
                        if remove_count > 0 {
                            remove_rows_at(&list, index + 1, remove_count);
                        }
                    } else {
                        // Expand: mark as expanded, update arrow, then load children async
                        {
                            let mut nodes = tree_nodes_ref.borrow_mut();
                            nodes[index].expanded = true;
                            update_dir_row_expanded(&list, index, nodes[index].expanded, &cache);
                        }

                        // Start watching this subdirectory for changes
                        {
                            use notify::{RecursiveMode, Watcher};
                            if let Some(ref mut w) = *watcher_rc.borrow_mut() {
                                let _ = w.watch(
                                    Path::new(&node.entry.path),
                                    RecursiveMode::NonRecursive,
                                );
                            }
                        }

                        let child_depth = node.depth + 1;
                        let path = node.entry.path.clone();
                        let tree_nodes_ref2 = tree_nodes_ref.clone();
                        let list2 = list.clone();
                        let show_hidden_val = *show_hidden.borrow();
                        let icon_cache2 = icon_cache.clone();
                        glib::spawn_future_local(async move {
                            let path_clone = path.clone();
                            let result = gio::spawn_blocking(move || {
                                impulse_core::filesystem::read_directory_with_git_status(
                                    &path_clone,
                                    show_hidden_val,
                                )
                            })
                            .await;

                            if let Ok(Ok(entries)) = result {
                                let mut nodes = tree_nodes_ref2.borrow_mut();
                                let insert_idx = nodes
                                    .iter()
                                    .position(|n| n.entry.path == path)
                                    .map(|i| i + 1);
                                if let Some(insert_idx) = insert_idx {
                                    let new_nodes: Vec<TreeNode> = entries
                                        .into_iter()
                                        .map(|e| TreeNode {
                                            entry: e,
                                            depth: child_depth,
                                            expanded: false,
                                        })
                                        .collect();
                                    // Insert into data model
                                    let nodes_to_insert: Vec<TreeNode> = new_nodes.clone();
                                    for (i, child_node) in new_nodes.into_iter().enumerate() {
                                        nodes.insert(insert_idx + i, child_node);
                                    }
                                    drop(nodes);
                                    // Insert rows into the ListBox at the right position
                                    insert_rows_at(
                                        &list2,
                                        insert_idx,
                                        &nodes_to_insert,
                                        &icon_cache2.borrow(),
                                    );
                                }
                            }
                        });
                    }
                } else {
                    // File activated -- invoke callback
                    if let Some(cb) = on_file_activated.borrow().as_ref() {
                        cb(&node.entry.path);
                    }
                }
            });
    }

    (sidebar, state)
}

pub struct SidebarState {
    pub file_tree_list: gtk4::ListBox,
    pub file_tree_scroll: gtk4::ScrolledWindow,
    pub search_btn: gtk4::ToggleButton,
    pub project_search: project_search::ProjectSearchState,
    pub current_path: Rc<RefCell<String>>,
    pub on_file_activated: EventCallback,
    pub on_open_terminal: EventCallback,
    pub tree_nodes: Rc<RefCell<Vec<TreeNode>>>,
    pub tab_tree_states: Rc<RefCell<HashMap<gtk4::Widget, TabTreeState>>>,
    pub active_tab: Rc<RefCell<Option<gtk4::Widget>>>,
    pub show_hidden: Rc<RefCell<bool>>,
    pub icon_cache: Rc<RefCell<IconCache>>,
    /// Keeps the filesystem watcher alive. Dropping this stops watching.
    _watcher: Rc<RefCell<Option<notify::RecommendedWatcher>>>,
    /// Source ID for the watcher's polling timer, so we can cancel it on re-watch.
    _watcher_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Keeps the .git/index watcher alive.
    _git_index_watcher: Rc<RefCell<Option<notify::RecommendedWatcher>>>,
    /// Source ID for the .git/index watcher's polling timer.
    _git_index_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Source ID for the periodic git status polling timer.
    _git_status_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Hash of the last `git status --porcelain` output, to avoid redundant refreshes.
    _last_git_status_hash: Rc<Cell<u64>>,
}

impl SidebarState {
    /// Load directory contents into the file tree as root-level (depth 0) nodes.
    pub fn load_directory(&self, path: &str) {
        *self.current_path.borrow_mut() = path.to_string();
        let list = self.file_tree_list.clone();
        let path = path.to_string();
        let tree_nodes = self.tree_nodes.clone();
        let show_hidden = *self.show_hidden.borrow();
        let icon_cache = self.icon_cache.clone();

        // Set up filesystem watcher for this directory
        self.setup_watcher(&path);

        glib::spawn_future_local(async move {
            let path_clone = path.clone();
            let result = gio::spawn_blocking(move || {
                impulse_core::filesystem::read_directory_with_git_status(&path_clone, show_hidden)
            })
            .await;

            match result {
                Ok(Ok(entries)) => {
                    let nodes: Vec<TreeNode> = entries
                        .into_iter()
                        .map(|e| TreeNode {
                            entry: e,
                            depth: 0,
                            expanded: false,
                        })
                        .collect();
                    *tree_nodes.borrow_mut() = nodes.clone();
                    render_tree(&list, &nodes, &icon_cache.borrow());
                }
                _ => {
                    tree_nodes.borrow_mut().clear();
                    clear_list(&list);
                }
            }
        });
    }

    /// Set up a filesystem watcher for the given directory.
    /// Events are debounced and forwarded to the GTK main loop to trigger tree refresh.
    /// Also starts a .git/index watcher and periodic git status polling.
    fn setup_watcher(&self, path: &str) {
        use notify::{RecursiveMode, Watcher};

        // Cancel previous watcher timer to avoid leaked polling loops
        if let Some(id) = self._watcher_timer.borrow_mut().take() {
            id.remove();
        }

        let (tx, rx) = std_mpsc::channel::<()>();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(_) => {
                            let _ = tx.send(());
                        }
                        _ => {}
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("Failed to create filesystem watcher: {}", e);
                    return;
                }
            };

        // Only watch the top-level directory (not recursively). Recursive mode
        // sets up inotify watches on every subdirectory, which blocks the main
        // thread for seconds on large trees like $HOME.
        if let Err(e) = watcher.watch(Path::new(path), RecursiveMode::NonRecursive) {
            log::warn!("Failed to watch directory {}: {}", path, e);
            return;
        }

        // Poll for filesystem events every 500ms (debounced)
        let tree_nodes = self.tree_nodes.clone();
        let file_tree_list = self.file_tree_list.clone();
        let file_tree_scroll = self.file_tree_scroll.clone();
        let current_path = self.current_path.clone();
        let show_hidden = self.show_hidden.clone();
        let icon_cache = self.icon_cache.clone();

        let timer_id = glib::timeout_add_local(Duration::from_millis(500), move || {
            // Drain all pending events
            let mut has_event = false;
            while rx.try_recv().is_ok() {
                has_event = true;
            }
            if has_event {
                refresh_tree(
                    &tree_nodes,
                    &file_tree_list,
                    &file_tree_scroll,
                    &current_path,
                    *show_hidden.borrow(),
                    icon_cache.clone(),
                );
            }
            glib::ControlFlow::Continue
        });

        // Also watch currently expanded subdirectories so changes in them
        // are detected without needing recursive inotify watches.
        for node in self.tree_nodes.borrow().iter() {
            if node.entry.is_dir && node.expanded {
                let _ = watcher.watch(Path::new(&node.entry.path), RecursiveMode::NonRecursive);
            }
        }

        *self._watcher.borrow_mut() = Some(watcher);
        *self._watcher_timer.borrow_mut() = Some(timer_id);

        // Start .git/index watcher and periodic git status polling.
        self.setup_git_index_watcher(path);
        self.setup_git_status_timer(path);
    }

    /// Watch `.git/index` for staging/commit/reset/checkout changes.
    fn setup_git_index_watcher(&self, path: &str) {
        use notify::{RecursiveMode, Watcher};

        // Stop any previous git index watcher.
        if let Some(id) = self._git_index_timer.borrow_mut().take() {
            id.remove();
        }
        *self._git_index_watcher.borrow_mut() = None;

        // Find the git repo root.
        let git_root = match std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
        {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            _ => return,
        };

        let index_path = format!("{}/.git/index", git_root);
        if !Path::new(&index_path).exists() {
            return;
        }

        let (tx, rx) = std_mpsc::channel::<()>();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(
                        event.kind,
                        notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    ) {
                        let _ = tx.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("Failed to create git index watcher: {}", e);
                    return;
                }
            };

        if let Err(e) = watcher.watch(Path::new(&index_path), RecursiveMode::NonRecursive) {
            log::warn!("Failed to watch .git/index: {}", e);
            return;
        }

        // Debounced poll: when .git/index changes, refresh the tree.
        let tree_nodes = self.tree_nodes.clone();
        let file_tree_list = self.file_tree_list.clone();
        let file_tree_scroll = self.file_tree_scroll.clone();
        let current_path = self.current_path.clone();
        let show_hidden = self.show_hidden.clone();
        let icon_cache = self.icon_cache.clone();

        let timer_id = glib::timeout_add_local(Duration::from_millis(500), move || {
            let mut has_event = false;
            while rx.try_recv().is_ok() {
                has_event = true;
            }
            if has_event {
                refresh_tree(
                    &tree_nodes,
                    &file_tree_list,
                    &file_tree_scroll,
                    &current_path,
                    *show_hidden.borrow(),
                    icon_cache.clone(),
                );
            }
            glib::ControlFlow::Continue
        });

        *self._git_index_watcher.borrow_mut() = Some(watcher);
        *self._git_index_timer.borrow_mut() = Some(timer_id);
    }

    /// Start a periodic timer that polls `git status --porcelain -u` every 2 seconds.
    /// Only triggers a tree refresh when the output hash changes.
    fn setup_git_status_timer(&self, path: &str) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Stop any previous timer.
        if let Some(id) = self._git_status_timer.borrow_mut().take() {
            id.remove();
        }
        self._last_git_status_hash.set(0);

        let root = path.to_string();
        let tree_nodes = self.tree_nodes.clone();
        let file_tree_list = self.file_tree_list.clone();
        let file_tree_scroll = self.file_tree_scroll.clone();
        let current_path = self.current_path.clone();
        let show_hidden = self.show_hidden.clone();
        let icon_cache = self.icon_cache.clone();
        let last_hash = self._last_git_status_hash.clone();

        let timer_id = glib::timeout_add_local(Duration::from_secs(2), move || {
            let root = root.clone();
            let tree_nodes = tree_nodes.clone();
            let file_tree_list = file_tree_list.clone();
            let file_tree_scroll = file_tree_scroll.clone();
            let current_path = current_path.clone();
            let show_hidden = show_hidden.clone();
            let icon_cache = icon_cache.clone();
            let last_hash = last_hash.clone();

            glib::spawn_future_local(async move {
                let root_clone = root.clone();
                let result = gio::spawn_blocking(move || {
                    std::process::Command::new("git")
                        .args(["status", "--porcelain", "-u"])
                        .current_dir(&root_clone)
                        .output()
                })
                .await;

                if let Ok(Ok(output)) = result {
                    let mut hasher = DefaultHasher::new();
                    output.stdout.hash(&mut hasher);
                    let hash = hasher.finish();

                    if hash != last_hash.get() {
                        last_hash.set(hash);
                        refresh_tree(
                            &tree_nodes,
                            &file_tree_list,
                            &file_tree_scroll,
                            &current_path,
                            *show_hidden.borrow(),
                            icon_cache.clone(),
                        );
                    }
                }
            });
            glib::ControlFlow::Continue
        });

        *self._git_status_timer.borrow_mut() = Some(timer_id);
    }

    /// Save the current tree state for the active tab.
    pub fn save_active_tab_state(&self) {
        if let Some(ref tab) = *self.active_tab.borrow() {
            let nodes = self.tree_nodes.borrow().clone();
            let current_path = self.current_path.borrow().clone();
            let scroll_position = self.file_tree_scroll.vadjustment().value();
            self.tab_tree_states.borrow_mut().insert(
                tab.clone(),
                TabTreeState {
                    nodes,
                    current_path,
                    scroll_position,
                },
            );
        }
    }

    /// Switch to a new tab, saving old state and restoring or loading new state.
    pub fn switch_to_tab(&self, tab_child: &gtk4::Widget, path: &str) {
        self.save_active_tab_state();
        *self.active_tab.borrow_mut() = Some(tab_child.clone());

        let saved = self
            .tab_tree_states
            .borrow()
            .get(tab_child)
            .map(|s| (s.nodes.clone(), s.current_path.clone(), s.scroll_position));

        if let Some((nodes, saved_path, scroll_pos)) = saved {
            if nodes.is_empty() {
                // Saved state had no tree data; load from disk instead
                self.load_directory(if saved_path.is_empty() {
                    path
                } else {
                    &saved_path
                });
            } else {
                *self.tree_nodes.borrow_mut() = nodes.clone();
                *self.current_path.borrow_mut() = saved_path.clone();
                render_tree(&self.file_tree_list, &nodes, &self.icon_cache.borrow());

                // Re-establish filesystem watcher for the restored directory
                self.setup_watcher(&saved_path);

                // Restore scroll position after the render
                let scroll = self.file_tree_scroll.clone();
                glib::idle_add_local_once(move || {
                    scroll.vadjustment().set_value(scroll_pos);
                });
            }
        } else {
            self.load_directory(path);
        }
    }

    /// Remove saved state for a closed tab.
    pub fn remove_tab_state(&self, tab_child: &gtk4::Widget) {
        self.tab_tree_states.borrow_mut().remove(tab_child);
    }

    /// Set the active tab without saving/restoring state.
    pub fn set_active_tab(&self, tab_child: &gtk4::Widget) {
        *self.active_tab.borrow_mut() = Some(tab_child.clone());
    }

    /// Rebuild the icon cache for a new theme and re-render the tree.
    pub fn update_theme(&self, theme: &ThemeColors) {
        self.icon_cache.borrow_mut().rebuild(theme);
        let nodes = self.tree_nodes.borrow().clone();
        render_tree(&self.file_tree_list, &nodes, &self.icon_cache.borrow());
    }

    /// Refresh the file tree to pick up git status changes (e.g. after saving a file).
    pub fn refresh(&self) {
        refresh_tree(
            &self.tree_nodes,
            &self.file_tree_list,
            &self.file_tree_scroll,
            &self.current_path,
            *self.show_hidden.borrow(),
            self.icon_cache.clone(),
        );
    }
}

/// Insert a newly created file or folder into the tree at the correct position.
/// Handles both subdirectory insertion (when parent node is found and expanded)
/// and root-level insertion (when dir_path equals the sidebar's root directory).
#[allow(clippy::too_many_arguments)]
fn insert_new_entry_into_tree(
    tree_nodes: &Rc<RefCell<Vec<TreeNode>>>,
    file_tree_list: &gtk4::ListBox,
    current_path: &Rc<RefCell<String>>,
    dir_path: &str,
    name: &str,
    full_path: &str,
    is_dir: bool,
    icon_cache: &IconCache,
) {
    let new_entry = FileEntry {
        name: name.to_string(),
        path: full_path.to_string(),
        is_dir,
        is_symlink: false,
        size: 0,
        modified: 0,
        git_status: None,
    };

    let mut nodes = tree_nodes.borrow_mut();

    // Try to find the parent directory node in the tree
    if let Some(parent_idx) = nodes.iter().position(|n| n.entry.path == dir_path) {
        let parent_depth = nodes[parent_idx].depth;
        if nodes[parent_idx].expanded {
            // Find the correct sorted insertion point among siblings
            let insert_idx =
                find_sorted_insert_position(&nodes, parent_idx + 1, parent_depth + 1, is_dir, name);
            nodes.insert(
                insert_idx,
                TreeNode {
                    entry: new_entry,
                    depth: parent_depth + 1,
                    expanded: false,
                },
            );
        }
    } else if dir_path == *current_path.borrow() {
        // Root-level insertion: dir_path is the sidebar root, which has no node.
        // Insert at the correct sorted position among depth-0 nodes.
        let insert_idx = find_sorted_insert_position(&nodes, 0, 0, is_dir, name);
        nodes.insert(
            insert_idx,
            TreeNode {
                entry: new_entry,
                depth: 0,
                expanded: false,
            },
        );
    }

    let snapshot: Vec<_> = nodes.clone();
    drop(nodes);
    render_tree(file_tree_list, &snapshot, icon_cache);
}

/// Find the correct sorted insertion position for a new entry among siblings
/// at the given depth, starting from `start_idx`. Directories sort before files,
/// and entries are sorted alphabetically within each group.
fn find_sorted_insert_position(
    nodes: &[TreeNode],
    start_idx: usize,
    target_depth: usize,
    is_dir: bool,
    name: &str,
) -> usize {
    let name_lower = name.to_lowercase();
    let mut idx = start_idx;

    while idx < nodes.len() && nodes[idx].depth >= target_depth {
        if nodes[idx].depth == target_depth {
            // Compare: directories before files, then alphabetical
            let sibling = &nodes[idx];
            let should_insert = match (is_dir, sibling.entry.is_dir) {
                (true, false) => true,  // new dir goes before existing file
                (false, true) => false, // new file goes after existing dir
                _ => name_lower < sibling.entry.name.to_lowercase(),
            };
            if should_insert {
                return idx;
            }
        }
        idx += 1;
    }
    idx
}

/// Refresh the tree while preserving expansion state and scroll position.
fn refresh_tree(
    tree_nodes: &Rc<RefCell<Vec<TreeNode>>>,
    file_tree_list: &gtk4::ListBox,
    file_tree_scroll: &gtk4::ScrolledWindow,
    current_path: &Rc<RefCell<String>>,
    show_hidden: bool,
    icon_cache: Rc<RefCell<IconCache>>,
) {
    let path = current_path.borrow().clone();
    if path.is_empty() {
        return;
    }

    // Collect currently expanded directory paths
    let expanded_paths: Vec<String> = tree_nodes
        .borrow()
        .iter()
        .filter(|n| n.entry.is_dir && n.expanded)
        .map(|n| n.entry.path.clone())
        .collect();

    let scroll_pos = file_tree_scroll.vadjustment().value();
    let tree_nodes = tree_nodes.clone();
    let file_tree_list = file_tree_list.clone();
    let file_tree_scroll = file_tree_scroll.clone();

    glib::spawn_future_local(async move {
        let path_clone = path.clone();
        let result = gio::spawn_blocking(move || {
            impulse_core::filesystem::read_directory_with_git_status(&path_clone, show_hidden)
        })
        .await;

        if let Ok(Ok(entries)) = result {
            let mut nodes: Vec<TreeNode> = entries
                .into_iter()
                .map(|e| TreeNode {
                    entry: e,
                    depth: 0,
                    expanded: false,
                })
                .collect();

            // Re-expand previously expanded directories (breadth-first)
            let mut i = 0;
            while i < nodes.len() {
                if nodes[i].entry.is_dir && expanded_paths.contains(&nodes[i].entry.path) {
                    nodes[i].expanded = true;
                    let child_depth = nodes[i].depth + 1;
                    let dir_path = nodes[i].entry.path.clone();
                    if let Ok(children) = impulse_core::filesystem::read_directory_with_git_status(
                        &dir_path,
                        show_hidden,
                    ) {
                        let child_nodes: Vec<TreeNode> = children
                            .into_iter()
                            .map(|e| TreeNode {
                                entry: e,
                                depth: child_depth,
                                expanded: false,
                            })
                            .collect();
                        for (j, child) in child_nodes.into_iter().enumerate() {
                            nodes.insert(i + 1 + j, child);
                        }
                    }
                }
                i += 1;
            }

            *tree_nodes.borrow_mut() = nodes.clone();
            render_tree(&file_tree_list, &nodes, &icon_cache.borrow());

            // Restore scroll position
            glib::idle_add_local_once(move || {
                file_tree_scroll.vadjustment().set_value(scroll_pos);
            });
        }
    });
}

/// Collapse all expanded directories back to root-level only.
fn collapse_all(
    tree_nodes: &Rc<RefCell<Vec<TreeNode>>>,
    file_tree_list: &gtk4::ListBox,
    icon_cache: &IconCache,
) {
    let mut nodes = tree_nodes.borrow_mut();
    // Keep only depth-0 nodes and mark them as collapsed
    nodes.retain(|n| n.depth == 0);
    for node in nodes.iter_mut() {
        node.expanded = false;
    }
    let snapshot: Vec<_> = nodes.clone();
    drop(nodes);
    render_tree(file_tree_list, &snapshot, icon_cache);
}

/// Map a filename/extension to an appropriate GTK symbolic icon name.
fn file_icon_name(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("");
    match ext.to_lowercase().as_str() {
        // Code
        "rs" | "go" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "cpp" | "h" | "hpp" | "java"
        | "kt" | "swift" | "rb" | "php" | "cs" | "zig" | "hs" | "el" | "lua" | "r" | "jl"
        | "scala" | "clj" | "ex" | "exs" | "erl" | "dart" | "v" | "nim" => "text-x-script-symbolic",
        // Web
        "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" => {
            "text-html-symbolic"
        }
        // Config / data
        "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "cfg" | "conf" | "ron" => {
            "text-x-generic-symbolic"
        }
        // Shell
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => "utilities-terminal-symbolic",
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" | "tiff" => {
            "image-x-generic-symbolic"
        }
        // Documents
        "md" | "txt" | "rst" | "org" | "tex" | "doc" | "docx" | "pdf" => {
            "x-office-document-symbolic"
        }
        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "package-x-generic-symbolic",
        // Audio/Video
        "mp3" | "wav" | "flac" | "ogg" | "mp4" | "mkv" | "avi" | "webm" => {
            "audio-x-generic-symbolic"
        }
        // Lock files, special
        "lock" => "channel-secure-symbolic",
        // Git
        "gitignore" | "gitmodules" | "gitattributes" => "text-x-generic-symbolic",
        // Default
        _ => {
            // Check for special filenames
            match filename.to_lowercase().as_str() {
                "makefile" | "dockerfile" | "rakefile" | "justfile" => "text-x-script-symbolic",
                "license" | "readme" | "changelog" | "authors" | "contributing" => {
                    "x-office-document-symbolic"
                }
                _ => "text-x-generic-symbolic",
            }
        }
    }
}

/// Build a single row widget for a tree node.
fn build_tree_row(node: &TreeNode, icon_cache: &IconCache) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.add_css_class("file-entry");
    row.set_widget_name(&node.entry.path);
    row.set_cursor_from_name(Some("pointer"));

    // Indent based on depth
    if node.depth > 0 {
        row.set_margin_start((node.depth as i32) * 16);
    }

    // Expand/collapse arrow for directories, spacer for files
    if node.entry.is_dir {
        let arrow = if node.expanded {
            gtk4::Image::from_icon_name("pan-down-symbolic")
        } else {
            gtk4::Image::from_icon_name("pan-end-symbolic")
        };
        arrow.set_pixel_size(12);
        row.append(&arrow);
    } else {
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_size_request(12, -1);
        row.append(&spacer);
    }

    // File/folder icon — use themed texture if available, fall back to GTK symbolic icons
    let icon =
        if let Some(texture) = icon_cache.get(&node.entry.name, node.entry.is_dir, node.expanded) {
            gtk4::Image::from_paintable(Some(texture))
        } else {
            let icon_name = if node.entry.is_dir {
                if node.expanded {
                    "folder-open-symbolic"
                } else {
                    "folder-symbolic"
                }
            } else {
                file_icon_name(&node.entry.name)
            };
            gtk4::Image::from_icon_name(icon_name)
        };
    icon.set_pixel_size(16);
    row.append(&icon);

    // Name label
    let label = gtk4::Label::new(Some(&node.entry.name));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    if node.entry.is_dir {
        label.add_css_class("file-entry-dir");
    } else {
        label.add_css_class("file-entry-file");
    }
    // Tint filename by git status
    if let Some(ref status) = node.entry.git_status {
        match status.as_str() {
            "M" => label.add_css_class("file-entry-git-modified"),
            "A" => label.add_css_class("file-entry-git-added"),
            "?" => label.add_css_class("file-entry-git-untracked"),
            "D" => label.add_css_class("file-entry-git-deleted"),
            "R" => label.add_css_class("file-entry-git-renamed"),
            "C" => label.add_css_class("file-entry-git-conflict"),
            _ => {}
        }
    }
    row.append(&label);

    // Git status indicator badge
    if let Some(ref status) = node.entry.git_status {
        let status_label = gtk4::Label::new(Some(status));
        match status.as_str() {
            "M" => status_label.add_css_class("git-modified"),
            "A" => status_label.add_css_class("git-added"),
            "?" => status_label.add_css_class("git-untracked"),
            "D" => status_label.add_css_class("git-deleted"),
            "R" => status_label.add_css_class("git-renamed"),
            "C" => status_label.add_css_class("git-conflict"),
            _ => {}
        }
        row.append(&status_label);
    }

    row
}

/// Update the arrow and icon on an existing directory row in-place (no remove/insert).
fn update_dir_row_expanded(
    list: &gtk4::ListBox,
    index: usize,
    expanded: bool,
    icon_cache: &IconCache,
) {
    let row = match list.row_at_index(index as i32) {
        Some(r) => r,
        None => return,
    };
    let content_box = match row.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
        Some(b) => b,
        None => return,
    };
    // First child is the arrow Image
    if let Some(arrow) = content_box
        .first_child()
        .and_then(|c| c.downcast::<gtk4::Image>().ok())
    {
        arrow.set_icon_name(Some(if expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
        // Second child (sibling) is the folder icon
        if let Some(icon) = arrow
            .next_sibling()
            .and_then(|c| c.downcast::<gtk4::Image>().ok())
        {
            if let Some(texture) = icon_cache.get("", true, expanded) {
                icon.set_paintable(Some(texture));
            } else {
                icon.set_icon_name(Some(if expanded {
                    "folder-open-symbolic"
                } else {
                    "folder-symbolic"
                }));
            }
        }
    }
}

/// Insert rows into the ListBox at the given position without clearing.
fn insert_rows_at(
    list: &gtk4::ListBox,
    position: usize,
    nodes: &[TreeNode],
    icon_cache: &IconCache,
) {
    for (i, node) in nodes.iter().enumerate() {
        let row = build_tree_row(node, icon_cache);
        list.insert(&row, (position + i) as i32);
    }
}

/// Remove `count` rows starting at `position` from the ListBox.
fn remove_rows_at(list: &gtk4::ListBox, position: usize, count: usize) {
    // Remove from bottom to top to keep indices stable
    for _ in 0..count {
        if let Some(row) = list.row_at_index(position as i32) {
            list.remove(&row);
        }
    }
}

/// Render the tree node list into the ListBox.
/// Each row is indented based on depth, directories show expand/collapse arrows.
fn render_tree(list: &gtk4::ListBox, nodes: &[TreeNode], icon_cache: &IconCache) {
    clear_list(list);
    for node in nodes {
        let row = build_tree_row(node, icon_cache);
        list.append(&row);
    }
}

fn clear_list(list: &gtk4::ListBox) {
    // Use row_at_index to only remove actual ListBoxRows,
    // avoiding the popover child which causes infinite loops with remove_all().
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}

/// Remove the `drop-target` CSS class from all rows in the list.
fn remove_drop_highlights(list: &gtk4::ListBox) {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        row.remove_css_class("drop-target");
        i += 1;
    }
}

/// Determine the target directory for a drop at the given y coordinate.
/// If the drop lands on a file row, returns the file's parent directory.
/// If on a directory row, returns that directory.
/// If on empty space, returns the project root.
fn resolve_drop_target_dir(list: &gtk4::ListBox, y: f64, current_path: &str) -> Option<String> {
    if let Some(row) = list.row_at_y(y as i32) {
        if let Some(child) = row.child() {
            let path = child.widget_name().to_string();
            if !path.is_empty() {
                if Path::new(&path).is_dir() {
                    return Some(path);
                } else {
                    return Path::new(&path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string());
                }
            }
        }
    }
    // Empty space — use project root
    if !current_path.is_empty() {
        Some(current_path.to_string())
    } else {
        None
    }
}

/// Move a file or directory from `source` into `target_dir`.
/// Validates against moving into self, same-parent no-ops, and name conflicts.
/// Falls back to copy+delete for cross-device moves.
fn perform_internal_move(source: &str, target_dir: &str) -> Result<(), String> {
    let source_path = Path::new(source);
    let target_dir_path = Path::new(target_dir);

    // Don't move a directory into itself
    if source_path.is_dir() && target_dir_path.starts_with(source_path) {
        return Err("Cannot move a directory into itself".to_string());
    }

    // No-op: already in target directory
    if let Some(parent) = source_path.parent() {
        if parent == target_dir_path {
            return Err("Already in target directory".to_string());
        }
    }

    let file_name = source_path.file_name().ok_or("Invalid source path")?;
    let dest = target_dir_path.join(file_name);

    if dest.exists() {
        return Err(format!(
            "'{}' already exists in target directory",
            file_name.to_string_lossy()
        ));
    }

    // Try rename (fast, same-device)
    match std::fs::rename(source, &dest) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: cross-device move — fall back to copy + delete
            if source_path.is_dir() {
                copy_dir_recursive(source_path, &dest)?;
                std::fs::remove_dir_all(source).map_err(|e| e.to_string())?;
            } else {
                std::fs::copy(source, &dest).map_err(|e| e.to_string())?;
                std::fs::remove_file(source).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Copy a file or directory from `source` into `target_dir`.
fn perform_external_copy(source: &str, target_dir: &str) -> Result<(), String> {
    let source_path = Path::new(source);
    let target_dir_path = Path::new(target_dir);

    let file_name = source_path.file_name().ok_or("Invalid source path")?;
    let dest = target_dir_path.join(file_name);

    if dest.exists() {
        return Err(format!(
            "'{}' already exists in target directory",
            file_name.to_string_lossy()
        ));
    }

    if source_path.is_dir() {
        copy_dir_recursive(source_path, &dest)?;
    } else {
        std::fs::copy(source, &dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_symlink() {
            // Skip symlinks to prevent traversal attacks
            continue;
        } else if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
