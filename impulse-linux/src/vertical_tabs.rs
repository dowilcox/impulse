//! Warp-style vertical tab list shown at the top of the sidebar when the
//! `tab_bar_position` setting is "sidebar". Mirrors the macOS
//! `SidebarTabListView`: each row shows the tab title plus a dimmed
//! subtitle (git branch or abbreviated working directory) and a
//! hover-revealed close button. The divider under the list is draggable:
//! pulling it down grows the tab section and shrinks the file tree; the
//! chosen height is persisted in settings (0 = auto-size to content).

use gtk4::prelude::*;
use libadwaita as adw;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::settings::Settings;
use crate::terminal;
use crate::terminal_container;

/// Auto mode: maximum height of the scrollable tab list, in pixels. The list
/// grows with its content up to this cap so the file tree keeps most of the
/// sidebar.
const LIST_MAX_HEIGHT: i32 = 320;

/// Smallest the tab section may shrink to (about one row).
const MIN_TAB_HEIGHT: i32 = 48;

/// Always leave at least this much sidebar height for the file tree.
const MIN_TREE_HEIGHT: i32 = 140;

/// Build the vertical tab list widget.
///
/// The returned box contains the scrollable list and a draggable divider,
/// so callers only need to `prepend()` it into the sidebar and toggle its
/// visibility as one unit.
pub fn build_vertical_tabs(tab_view: &adw::TabView, settings: &Rc<RefCell<Settings>>) -> gtk4::Box {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.add_css_class("vertical-tabs");

    // Tab list inside a height-capped scrolled window
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.add_css_class("vertical-tabs-list");

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&list));
    container.append(&scrolled);

    // Apply a fixed (user-dragged) or auto height to the list.
    let apply_height = {
        let scrolled = scrolled.clone();
        Rc::new(move |height: i32| {
            if height > 0 {
                let height = height.max(MIN_TAB_HEIGHT);
                scrolled.set_propagate_natural_height(false);
                scrolled.set_min_content_height(height);
                scrolled.set_max_content_height(height);
            } else {
                scrolled.set_min_content_height(-1);
                scrolled.set_propagate_natural_height(true);
                scrolled.set_max_content_height(LIST_MAX_HEIGHT);
            }
        })
    };
    apply_height(settings.borrow().sidebar_tab_section_height);

    // Draggable divider between the tab list and the file tree.
    let handle = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    handle.add_css_class("vertical-tabs-resize-handle");
    handle.set_cursor_from_name(Some("ns-resize"));
    handle.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    container.append(&handle);

    {
        let drag_anchor = Rc::new(Cell::new(0));
        let drag = gtk4::GestureDrag::new();
        {
            let drag_anchor = drag_anchor.clone();
            let scrolled = scrolled.clone();
            drag.connect_drag_begin(move |_, _, _| {
                drag_anchor.set(scrolled.height());
            });
        }
        {
            let drag_anchor = drag_anchor.clone();
            let apply_height = apply_height.clone();
            let container = container.clone();
            drag.connect_drag_update(move |_, _dx, dy| {
                let available = container
                    .parent()
                    .map(|parent| parent.height())
                    .unwrap_or(0);
                let max = (available - MIN_TREE_HEIGHT).max(MIN_TAB_HEIGHT);
                let height = (drag_anchor.get() + dy as i32).clamp(MIN_TAB_HEIGHT, max);
                apply_height(height);
            });
        }
        {
            let settings = settings.clone();
            let scrolled = scrolled.clone();
            drag.connect_drag_end(move |_, _, _| {
                let height = scrolled.height();
                settings.borrow_mut().sidebar_tab_section_height = height;
                crate::settings::save(&settings.borrow());
            });
        }
        handle.add_controller(drag);
    }

    // Rebuild the whole list on any tab change. Tabs are few, so a full
    // rebuild is simpler and more robust than incremental updates.
    let rebuild: Rc<dyn Fn()> = {
        let tab_view = tab_view.clone();
        let list = list.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let selected = tab_view.selected_page();
            let n = tab_view.n_pages();
            for i in 0..n {
                let page = tab_view.nth_page(i);
                let row = build_tab_row(&tab_view, &page);
                list.append(&row);
                if selected.as_ref() == Some(&page) {
                    list.select_row(Some(&row));
                }
            }
        })
    };

    // Activating a row selects the corresponding page. Row index matches
    // page index because the list is fully rebuilt on attach/detach/reorder.
    {
        let tab_view = tab_view.clone();
        list.connect_row_activated(move |_, row| {
            let index = row.index();
            if index >= 0 && index < tab_view.n_pages() {
                let page = tab_view.nth_page(index);
                tab_view.set_selected_page(&page);
                // Match the existing tab switch handler: focus the content.
                let child = page.child();
                if let Some(term) = terminal_container::get_active_terminal(&child) {
                    term.grab_focus();
                } else {
                    child.grab_focus();
                }
            }
        });
    }

    // Keep the list in sync with the tab view.
    {
        let rebuild = rebuild.clone();
        tab_view.connect_page_attached(move |_, page, _| {
            // Rebuild when the page title changes (terminal CWD or file name).
            {
                let rebuild = rebuild.clone();
                page.connect_title_notify(move |_| rebuild());
            }
            rebuild();
        });
    }
    {
        let rebuild = rebuild.clone();
        tab_view.connect_page_detached(move |_, _, _| rebuild());
    }
    {
        let rebuild = rebuild.clone();
        tab_view.connect_page_reordered(move |_, _, _| rebuild());
    }
    {
        let rebuild = rebuild.clone();
        tab_view.connect_selected_page_notify(move |_| rebuild());
    }

    // Pages attached before this widget existed need their title-notify
    // connections too (e.g. tabs restored from the previous session).
    let n = tab_view.n_pages();
    for i in 0..n {
        let page = tab_view.nth_page(i);
        let rebuild = rebuild.clone();
        page.connect_title_notify(move |_| rebuild());
    }

    rebuild();
    container
}

/// Build one row: title, dimmed subtitle, and a hover-revealed close button.
fn build_tab_row(tab_view: &adw::TabView, page: &adw::TabPage) -> gtk4::ListBoxRow {
    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row_box.add_css_class("vertical-tab-row");

    let text_box = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
    text_box.set_hexpand(true);
    text_box.set_valign(gtk4::Align::Center);

    let title = page.title().to_string();
    let title_label = gtk4::Label::new(Some(&title));
    title_label.add_css_class("vertical-tab-title");
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    text_box.append(&title_label);

    if let Some(subtitle) = tab_subtitle(page, &title) {
        let subtitle_label = gtk4::Label::new(Some(&subtitle));
        subtitle_label.add_css_class("vertical-tab-subtitle");
        subtitle_label.set_halign(gtk4::Align::Start);
        subtitle_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        text_box.append(&subtitle_label);
    }
    row_box.append(&text_box);

    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.add_css_class("flat");
    close_btn.add_css_class("vertical-tab-close");
    close_btn.set_valign(gtk4::Align::Center);
    close_btn.set_tooltip_text(Some("Close Tab"));
    {
        let tab_view = tab_view.clone();
        let page = page.clone();
        close_btn.connect_clicked(move |_| {
            tab_view.close_page(&page);
        });
    }
    row_box.append(&close_btn);

    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

/// Subtitle for a tab row: the git branch of the tab's directory if it is a
/// git repository, otherwise the abbreviated directory itself. Skipped when
/// the title already contains the directory text (matches the macOS
/// `SidebarTabListView.subtitleContent`).
fn tab_subtitle(page: &adw::TabPage, title: &str) -> Option<String> {
    let child = page.child();
    let dir = if let Some(term) = terminal_container::get_active_terminal(&child) {
        terminal::current_directory(&term)
    } else if crate::editor::is_editor(&child) {
        let path = child.widget_name().to_string();
        std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };
    let dir = dir.filter(|d| !d.is_empty())?;

    if let Ok(Some(branch)) = impulse_core::filesystem::get_git_branch(&dir) {
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    let display = crate::context_bar::abbreviate_home_path(&dir);
    if title.contains(&display) {
        return None;
    }
    Some(display)
}
