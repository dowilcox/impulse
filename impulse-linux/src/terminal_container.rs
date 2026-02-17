use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::terminal;

/// A container that wraps a VTE terminal and supports splitting.
/// The container is a `gtk4::Box` that holds either a single terminal
/// or a `gtk4::Paned` with two child containers for split layouts.
pub struct TerminalContainer {
    pub widget: gtk4::Box,
}

impl TerminalContainer {
    pub fn new(term: &vte4::Terminal) -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.append(term);

        TerminalContainer { widget }
    }
}

/// Split the terminal that currently has focus within the given container widget.
/// `orientation` controls the split direction: `Horizontal` places terminals side
/// by side, `Vertical` stacks them top/bottom.
/// `setup_terminal` is called on the newly created terminal so the caller can
/// wire up signals (CWD change, child-exited, etc.) before the shell is spawned.
/// Returns the newly created terminal, or `None` if the split failed.
pub fn split_terminal(
    container: &gtk4::Widget,
    orientation: gtk4::Orientation,
    setup_terminal: &dyn Fn(&vte4::Terminal),
    settings: &crate::settings::Settings,
    theme: &crate::theme::ThemeColors,
    copy_on_select_flag: Rc<Cell<bool>>,
    shell_cache: &Rc<terminal::ShellSpawnCache>,
) -> Option<vte4::Terminal> {
    // Find the focused terminal; fall back to the first terminal in the tree.
    let focused = find_focused_terminal(container).or_else(|| find_first_terminal(container))?;

    // The focused terminal's immediate parent should be a Box (from
    // TerminalContainer::new or a previous split).
    let parent_widget = focused.parent()?;
    let parent_box = parent_widget.downcast_ref::<gtk4::Box>()?;

    // Create a new terminal and let the caller set up its signals.
    let new_term = terminal::create_terminal(settings, theme, copy_on_select_flag);
    setup_terminal(&new_term);
    terminal::spawn_shell(&new_term, shell_cache);

    // Build a Paned to hold the original and new terminal.
    let paned = gtk4::Paned::new(orientation);
    paned.set_hexpand(true);
    paned.set_vexpand(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Remove the focused terminal from its parent box.
    parent_box.remove(&focused);

    // Wrap each terminal in its own Box so future splits work the same way.
    let box1 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    box1.set_hexpand(true);
    box1.set_vexpand(true);
    box1.append(&focused);

    let box2 = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    box2.set_hexpand(true);
    box2.set_vexpand(true);
    box2.append(&new_term);

    paned.set_start_child(Some(&box1));
    paned.set_end_child(Some(&box2));

    parent_box.append(&paned);

    new_term.grab_focus();
    Some(new_term)
}

/// Find the VTE terminal that currently has keyboard focus within the widget
/// tree rooted at `widget`.  Returns `None` if no focused terminal is found.
pub fn find_focused_terminal(widget: &gtk4::Widget) -> Option<vte4::Terminal> {
    if let Some(term) = widget.downcast_ref::<vte4::Terminal>() {
        if term.has_focus() {
            return Some(term.clone());
        }
    }

    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(term) = find_focused_terminal(&c) {
            return Some(term);
        }
        child = c.next_sibling();
    }

    None
}

/// Get the active terminal within the widget tree. Prefers the focused terminal;
/// falls back to the first terminal found via depth-first search.
pub fn get_active_terminal(widget: &gtk4::Widget) -> Option<vte4::Terminal> {
    find_focused_terminal(widget).or_else(|| find_first_terminal(widget))
}

/// Find the first VTE terminal in the widget tree (depth-first order).
pub fn find_first_terminal(widget: &gtk4::Widget) -> Option<vte4::Terminal> {
    if let Some(term) = widget.downcast_ref::<vte4::Terminal>() {
        return Some(term.clone());
    }

    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(term) = find_first_terminal(&c) {
            return Some(term);
        }
        child = c.next_sibling();
    }

    None
}

/// Collect all VTE terminals in the widget tree (depth-first order).
pub fn collect_terminals(widget: &gtk4::Widget) -> Vec<vte4::Terminal> {
    let mut terminals = Vec::new();
    collect_terminals_recursive(widget, &mut terminals);
    terminals
}

fn collect_terminals_recursive(widget: &gtk4::Widget, terminals: &mut Vec<vte4::Terminal>) {
    if let Some(term) = widget.downcast_ref::<vte4::Terminal>() {
        terminals.push(term.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        collect_terminals_recursive(&c, terminals);
        child = c.next_sibling();
    }
}

/// Focus the next terminal in the current tab's split layout.
pub fn focus_next_terminal(container: &gtk4::Widget) {
    let terminals = collect_terminals(container);
    if terminals.len() <= 1 {
        return;
    }

    // Find the currently focused terminal
    let focused_idx = terminals.iter().position(|t| t.has_focus());
    let next_idx = match focused_idx {
        Some(idx) => (idx + 1) % terminals.len(),
        None => 0,
    };
    terminals[next_idx].grab_focus();
}

/// Remove a single terminal from a split layout.  If the terminal is inside a
/// `Paned`, the sibling pane is promoted into the `Paned`'s parent, effectively
/// collapsing one level of splitting.  Returns `true` if the pane was removed.
pub fn remove_terminal(container: &gtk4::Widget, terminal: &vte4::Terminal) -> bool {
    // Terminal → wrapper Box → Paned.
    let wrapper = match terminal.parent() {
        Some(p) => p,
        None => return false,
    };
    let paned_widget = match wrapper.parent() {
        Some(p) => p,
        None => return false,
    };
    let paned = match paned_widget.downcast_ref::<gtk4::Paned>() {
        Some(p) => p,
        None => return false, // not in a split
    };

    // Identify the sibling (the other child of the Paned).
    let start = paned.start_child();
    let end = paned.end_child();
    let is_start = start.as_ref() == Some(&wrapper);
    let sibling = if is_start {
        end.clone()
    } else {
        start.clone()
    };
    let sibling = match sibling {
        Some(s) => s,
        None => return false,
    };

    // Detach both children from the Paned.
    paned.set_start_child(gtk4::Widget::NONE);
    paned.set_end_child(gtk4::Widget::NONE);

    // The Paned lives inside a parent Box (from TerminalContainer or a
    // higher-level split wrapper).
    let parent = match paned_widget.parent() {
        Some(p) => p,
        None => return false,
    };
    if let Some(parent_box) = parent.downcast_ref::<gtk4::Box>() {
        parent_box.remove(&paned_widget);
        // Move the sibling's children into parent_box so we don't accumulate
        // extra wrapper Box layers.
        if let Some(sib_box) = sibling.downcast_ref::<gtk4::Box>() {
            while let Some(child) = sib_box.first_child() {
                sib_box.remove(&child);
                parent_box.append(&child);
            }
        } else {
            parent_box.append(&sibling);
        }
    } else if let Some(parent_paned) = parent.downcast_ref::<gtk4::Paned>() {
        // The Paned is itself nested inside another Paned.
        let is_start_of_parent =
            parent_paned.start_child().as_ref() == Some(&paned_widget);
        if is_start_of_parent {
            parent_paned.set_start_child(Some(&sibling));
        } else {
            parent_paned.set_end_child(Some(&sibling));
        }
    }

    // Focus a remaining terminal.
    if let Some(term) = find_first_terminal(container) {
        term.grab_focus();
    }

    true
}

/// Focus the previous terminal in the current tab's split layout.
pub fn focus_prev_terminal(container: &gtk4::Widget) {
    let terminals = collect_terminals(container);
    if terminals.len() <= 1 {
        return;
    }

    let focused_idx = terminals.iter().position(|t| t.has_focus());
    let prev_idx = match focused_idx {
        Some(idx) => {
            if idx == 0 {
                terminals.len() - 1
            } else {
                idx - 1
            }
        }
        None => 0,
    };
    terminals[prev_idx].grab_focus();
}
