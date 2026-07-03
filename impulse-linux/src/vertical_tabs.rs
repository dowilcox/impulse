//! Warp-style vertical tab list shown at the top of the sidebar when the
//! `tab_bar_position` setting is "sidebar". Mirrors the macOS
//! `SidebarTabListView`: each row shows the tab title plus a dimmed
//! subtitle (git branch or abbreviated working directory) and a
//! hover-revealed close button. The divider under the list is draggable:
//! pulling it down grows the tab section and shrinks the file tree; the
//! chosen height is persisted in settings (0 = auto-size to content).

use gtk4::prelude::*;
use gtk4::{gio, glib};
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
pub fn build_vertical_tabs(
    tab_view: &adw::TabView,
    settings: &Rc<RefCell<Settings>>,
    new_tab: Rc<dyn Fn()>,
) -> gtk4::Box {
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
        let new_tab = new_tab.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let selected = tab_view.selected_page();
            let n = tab_view.n_pages();
            for i in 0..n {
                let page = tab_view.nth_page(i);
                let row = build_tab_row(&tab_view, &page, i, &new_tab);
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

    // Drag-and-drop reorder: each row carries its page index; dropping on a row
    // reorders the dragged page to that row's position. adw clamps the target
    // to respect pinned/unpinned boundaries.
    {
        let drop_target =
            gtk4::DropTarget::new(glib::types::Type::STRING, gtk4::gdk::DragAction::MOVE);
        let tab_view = tab_view.clone();
        let list_for_drop = list.clone();
        drop_target.connect_drop(move |_, value, _x, y| {
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            let Ok(source_idx) = source.parse::<i32>() else {
                return false;
            };
            let n = tab_view.n_pages();
            if source_idx < 0 || source_idx >= n {
                return false;
            }
            let target_idx = list_for_drop
                .row_at_y(y as i32)
                .map(|r| r.index())
                .unwrap_or(n - 1)
                .clamp(0, n - 1);
            let page = tab_view.nth_page(source_idx);
            tab_view.reorder_page(&page, target_idx);
            true
        });
        list.add_controller(drop_target);
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

/// A subtitle segment shown under a tab title: the abbreviated working
/// directory (folder glyph) or the git branch (branch glyph).
enum SubtitleSegment {
    Directory(String),
    Branch(String),
}

/// Build one row: title, dimmed subtitle segments, an optional attention dot,
/// and a pin indicator / hover-revealed close button. Right-click opens a
/// per-row context menu; the row can be dragged to reorder tabs.
fn build_tab_row(
    tab_view: &adw::TabView,
    page: &adw::TabPage,
    index: i32,
    new_tab: &Rc<dyn Fn()>,
) -> gtk4::ListBoxRow {
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

    let segments = tab_subtitle_segments(page, &title);
    if !segments.is_empty() {
        let subtitle_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        subtitle_box.set_halign(gtk4::Align::Start);
        for segment in &segments {
            match segment {
                SubtitleSegment::Directory(dir) => {
                    let seg = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
                    let icon = gtk4::Image::from_icon_name("folder-symbolic");
                    icon.set_pixel_size(12);
                    icon.add_css_class("vertical-tab-subtitle");
                    seg.append(&icon);
                    let label = gtk4::Label::new(Some(dir));
                    label.add_css_class("vertical-tab-subtitle");
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                    seg.append(&label);
                    subtitle_box.append(&seg);
                }
                SubtitleSegment::Branch(branch) => {
                    // Nerd-font branch glyph, matching the status bar.
                    let label = gtk4::Label::new(Some(&format!("\u{e0a0} {}", branch)));
                    label.add_css_class("vertical-tab-subtitle");
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                    subtitle_box.append(&label);
                }
            }
        }
        text_box.append(&subtitle_box);
    }
    row_box.append(&text_box);

    // Attention dot: shown when the page requests attention and isn't selected.
    if page.needs_attention() && tab_view.selected_page().as_ref() != Some(page) {
        let dot = gtk4::Label::new(Some("\u{25cf}"));
        dot.add_css_class("vertical-tab-attention");
        dot.set_valign(gtk4::Align::Center);
        row_box.append(&dot);
    }

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

    // Trailing slot: a pinned tab shows a pin icon when the row isn't hovered,
    // swapping to the close button on hover; unpinned tabs show only the
    // hover-revealed close button.
    let is_pinned = tab_view.page_position(page) < tab_view.n_pinned_pages();
    if is_pinned {
        let pin = gtk4::Image::from_icon_name("view-pin-symbolic");
        pin.set_pixel_size(12);
        pin.add_css_class("vertical-tab-pin");
        pin.set_valign(gtk4::Align::Center);

        let slot = gtk4::Overlay::new();
        slot.set_child(Some(&pin));
        slot.add_overlay(&close_btn);
        row_box.append(&slot);

        // Hide the pin on hover so the close button (revealed by CSS) takes its
        // place.
        let motion = gtk4::EventControllerMotion::new();
        {
            let pin = pin.clone();
            motion.connect_enter(move |_, _, _| pin.set_visible(false));
        }
        {
            let pin = pin.clone();
            motion.connect_leave(move |_| pin.set_visible(true));
        }
        row_box.add_controller(motion);
    } else {
        row_box.append(&close_btn);
    }

    // Per-row right-click context menu: Pin/Unpin, Close, New Tab.
    {
        let action_group = gio::SimpleActionGroup::new();

        let pin_action = gio::SimpleAction::new("pin-toggle", None);
        {
            let tab_view = tab_view.clone();
            let page = page.clone();
            pin_action.connect_activate(move |_, _| {
                let pinned = tab_view.page_position(&page) < tab_view.n_pinned_pages();
                tab_view.set_page_pinned(&page, !pinned);
            });
        }
        action_group.add_action(&pin_action);

        let close_action = gio::SimpleAction::new("close", None);
        {
            let tab_view = tab_view.clone();
            let page = page.clone();
            close_action.connect_activate(move |_, _| {
                tab_view.close_page(&page);
            });
        }
        action_group.add_action(&close_action);

        let new_tab_action = gio::SimpleAction::new("new-tab", None);
        {
            let new_tab = new_tab.clone();
            new_tab_action.connect_activate(move |_, _| new_tab());
        }
        action_group.add_action(&new_tab_action);
        row_box.insert_action_group("tabrow", Some(&action_group));

        let menu = gio::Menu::new();
        menu.append(
            Some(if is_pinned { "Unpin Tab" } else { "Pin Tab" }),
            Some("tabrow.pin-toggle"),
        );
        menu.append(Some("Close Tab"), Some("tabrow.close"));
        menu.append(Some("New Tab"), Some("tabrow.new-tab"));

        let gesture = gtk4::GestureClick::new();
        gesture.set_button(3);
        {
            let row_box = row_box.clone();
            gesture.connect_pressed(move |_, _, x, y| {
                let popover = gtk4::PopoverMenu::from_model(Some(&menu));
                popover.set_has_arrow(false);
                popover.set_parent(&row_box);
                let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                popover.set_pointing_to(Some(&rect));
                popover.connect_closed(|p| p.unparent());
                popover.popup();
            });
        }
        row_box.add_controller(gesture);
    }

    // Drag to reorder: the row carries its page index; the list's DropTarget
    // (set up in build_vertical_tabs) reorders the page on drop.
    {
        let drag = gtk4::DragSource::new();
        drag.set_actions(gtk4::gdk::DragAction::MOVE);
        let index_str = index.to_string();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk4::gdk::ContentProvider::for_value(&index_str.to_value()))
        });
        row_box.add_controller(drag);
    }

    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

/// Subtitle segments for a tab row. Terminal tabs surface both the working
/// directory and the git branch (each with its own glyph); editor tabs show the
/// containing directory only. The directory segment is skipped when the title
/// already contains it (matches the macOS `SidebarTabListView.subtitleSegments`).
fn tab_subtitle_segments(page: &adw::TabPage, title: &str) -> Vec<SubtitleSegment> {
    let child = page.child();
    let mut segments = Vec::new();

    if let Some(term) = terminal_container::get_active_terminal(&child) {
        if let Some(dir) = terminal::current_directory(&term).filter(|d| !d.is_empty()) {
            let display = crate::context_bar::abbreviate_home_path(&dir);
            if !title.contains(&display) {
                segments.push(SubtitleSegment::Directory(display));
            }
            if let Ok(Some(branch)) = impulse_core::filesystem::get_git_branch(&dir) {
                if !branch.is_empty() {
                    segments.push(SubtitleSegment::Branch(branch));
                }
            }
        }
    } else if crate::editor::is_editor(&child) {
        let path = child.widget_name().to_string();
        if let Some(dir) = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|d| !d.is_empty())
        {
            let display = crate::context_bar::abbreviate_home_path(&dir);
            if !title.contains(&display) {
                segments.push(SubtitleSegment::Directory(display));
            }
        }
    }

    segments
}
