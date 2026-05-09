use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::terminal;

/// A container that wraps an Impulse terminal and supports splitting.
/// The container is a `gtk4::Box` that holds either a single terminal
/// or a `gtk4::Paned` with two child containers for split layouts.
pub struct TerminalContainer {
    pub widget: gtk4::Box,
}

#[derive(Clone, Debug)]
pub struct TerminalSessionSnapshot {
    pub panes: Vec<impulse_core::session_state::SessionTerminalPane>,
    pub active_pane_index: Option<usize>,
    pub pane_layout: impulse_core::session_state::SessionTerminalPaneLayout,
}

impl TerminalContainer {
    pub fn new(term: &terminal::Terminal) -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.append(term);

        TerminalContainer { widget }
    }

    pub fn from_session_tab(
        tab: &impulse_core::session_state::SessionTerminalTab,
        setup_terminal: &dyn Fn(&terminal::Terminal),
        settings: &crate::settings::Settings,
        theme: &crate::theme::ThemeColors,
        copy_on_select_flag: Rc<Cell<bool>>,
        shell_cache: &Rc<terminal::ShellSpawnCache>,
    ) -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        widget.set_hexpand(true);
        widget.set_vexpand(true);

        let panes = session_panes_for_tab(tab);
        let active_pane_index = tab.active_pane_index.unwrap_or(0);
        let layout = tab.pane_layout.clone().unwrap_or(
            impulse_core::session_state::SessionTerminalPaneLayout::Pane(
                impulse_core::session_state::SessionTerminalPaneLeaf { pane_index: 0 },
            ),
        );
        let mut active_terminal = None;

        if let Some(restored) = restore_session_layout(
            &layout,
            &panes,
            active_pane_index,
            &mut active_terminal,
            setup_terminal,
            settings,
            theme,
            copy_on_select_flag.clone(),
            shell_cache,
        ) {
            widget.append(&restored);
        } else {
            let term = terminal::create_terminal(settings, theme, copy_on_select_flag);
            setup_terminal(&term);
            terminal::spawn_shell(
                &term,
                shell_cache,
                Some(default_terminal_directory().as_str()),
            );
            active_terminal = Some(term.clone());
            widget.append(&term);
        }

        if let Some(term) = active_terminal {
            gtk4::glib::idle_add_local_once(move || {
                term.grab_focus();
            });
        }

        TerminalContainer { widget }
    }
}

fn session_panes_for_tab(
    tab: &impulse_core::session_state::SessionTerminalTab,
) -> Vec<impulse_core::session_state::SessionTerminalPane> {
    if !tab.panes.is_empty() {
        return tab.panes.clone();
    }
    vec![impulse_core::session_state::SessionTerminalPane {
        cwd: tab.cwd.clone(),
        title: tab.title.clone(),
        shell: tab.shell.clone(),
    }]
}

#[allow(clippy::too_many_arguments)]
fn restore_session_layout(
    layout: &impulse_core::session_state::SessionTerminalPaneLayout,
    panes: &[impulse_core::session_state::SessionTerminalPane],
    active_pane_index: usize,
    active_terminal: &mut Option<terminal::Terminal>,
    setup_terminal: &dyn Fn(&terminal::Terminal),
    settings: &crate::settings::Settings,
    theme: &crate::theme::ThemeColors,
    copy_on_select_flag: Rc<Cell<bool>>,
    shell_cache: &Rc<terminal::ShellSpawnCache>,
) -> Option<gtk4::Widget> {
    match layout {
        impulse_core::session_state::SessionTerminalPaneLayout::Pane(leaf) => {
            let pane = panes.get(leaf.pane_index)?;
            let term = terminal::create_terminal(settings, theme, copy_on_select_flag);
            setup_terminal(&term);
            let cwd = if pane.cwd.trim().is_empty() {
                default_terminal_directory()
            } else {
                pane.cwd.clone()
            };
            terminal::spawn_shell(&term, shell_cache, Some(cwd.as_str()));
            if leaf.pane_index == active_pane_index {
                *active_terminal = Some(term.clone());
            }

            let wrapper = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            wrapper.set_hexpand(true);
            wrapper.set_vexpand(true);
            wrapper.append(&term);
            Some(wrapper.upcast())
        }
        impulse_core::session_state::SessionTerminalPaneLayout::Split(split) => {
            let paned = gtk4::Paned::new(orientation_from_session_axis(split.axis));
            paned.set_hexpand(true);
            paned.set_vexpand(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);

            let first = restore_session_layout(
                &split.first,
                panes,
                active_pane_index,
                active_terminal,
                setup_terminal,
                settings,
                theme,
                copy_on_select_flag.clone(),
                shell_cache,
            );
            let second = restore_session_layout(
                &split.second,
                panes,
                active_pane_index,
                active_terminal,
                setup_terminal,
                settings,
                theme,
                copy_on_select_flag,
                shell_cache,
            );
            match (first, second) {
                (Some(first), Some(second)) => {
                    paned.set_start_child(Some(&first));
                    paned.set_end_child(Some(&second));
                    let ratio = split.ratio;
                    paned.connect_map(move |paned| {
                        let dimension = match paned.orientation() {
                            gtk4::Orientation::Horizontal => paned.width(),
                            gtk4::Orientation::Vertical => paned.height(),
                            _ => 0,
                        };
                        if dimension > 0 {
                            paned.set_position(((dimension as f32) * ratio).round() as i32);
                        }
                    });
                    Some(paned.upcast())
                }
                (Some(layout), None) | (None, Some(layout)) => Some(layout),
                (None, None) => None,
            }
        }
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
    setup_terminal: &dyn Fn(&terminal::Terminal),
    settings: &crate::settings::Settings,
    theme: &crate::theme::ThemeColors,
    copy_on_select_flag: Rc<Cell<bool>>,
    shell_cache: &Rc<terminal::ShellSpawnCache>,
) -> Option<terminal::Terminal> {
    // Find the focused terminal; fall back to the first terminal in the tree.
    let focused = find_focused_terminal(container).or_else(|| find_first_terminal(container))?;

    // The focused terminal's immediate parent should be a Box (from
    // TerminalContainer::new or a previous split).
    let parent_widget = focused.parent()?;
    let parent_box = parent_widget.downcast_ref::<gtk4::Box>()?;

    // Capture the focused terminal's CWD so the new split inherits it.
    let cwd = terminal::current_directory(&focused);

    // Create a new terminal and let the caller set up its signals.
    let new_term = terminal::create_terminal(settings, theme, copy_on_select_flag);
    setup_terminal(&new_term);
    terminal::spawn_shell(&new_term, shell_cache, cwd.as_deref());

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

/// Find the Impulse terminal that currently has keyboard focus within the widget
/// tree rooted at `widget`.  Returns `None` if no focused terminal is found.
pub fn find_focused_terminal(widget: &gtk4::Widget) -> Option<terminal::Terminal> {
    if let Some(term) = terminal::from_widget(widget) {
        if term.has_focus() {
            return Some(term);
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
pub fn get_active_terminal(widget: &gtk4::Widget) -> Option<terminal::Terminal> {
    find_focused_terminal(widget).or_else(|| find_first_terminal(widget))
}

/// Find the first Impulse terminal in the widget tree (depth-first order).
pub fn find_first_terminal(widget: &gtk4::Widget) -> Option<terminal::Terminal> {
    if let Some(term) = terminal::from_widget(widget) {
        return Some(term);
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

/// Collect all Impulse terminals in the widget tree (depth-first order).
pub fn collect_terminals(widget: &gtk4::Widget) -> Vec<terminal::Terminal> {
    let mut terminals = Vec::new();
    collect_terminals_recursive(widget, &mut terminals);
    terminals
}

pub fn session_snapshot(widget: &gtk4::Widget) -> Option<TerminalSessionSnapshot> {
    let mut panes = Vec::new();
    let mut active_pane_index = None;
    let pane_layout = session_layout_recursive(widget, &mut panes, &mut active_pane_index)?;
    if panes.is_empty() {
        return None;
    }
    let active_pane_index = active_pane_index.or(Some(0));
    Some(TerminalSessionSnapshot {
        panes,
        active_pane_index,
        pane_layout,
    })
}

fn session_layout_recursive(
    widget: &gtk4::Widget,
    panes: &mut Vec<impulse_core::session_state::SessionTerminalPane>,
    active_pane_index: &mut Option<usize>,
) -> Option<impulse_core::session_state::SessionTerminalPaneLayout> {
    if let Some(term) = terminal::from_widget(widget) {
        let pane_index = panes.len();
        if term.has_focus() {
            *active_pane_index = Some(pane_index);
        }
        panes.push(impulse_core::session_state::SessionTerminalPane {
            cwd: terminal::current_directory(&term).unwrap_or_else(default_terminal_directory),
            title: non_empty_string(terminal::title(&term)),
            shell: None,
        });
        return Some(
            impulse_core::session_state::SessionTerminalPaneLayout::Pane(
                impulse_core::session_state::SessionTerminalPaneLeaf { pane_index },
            ),
        );
    }

    if let Some(paned) = widget.downcast_ref::<gtk4::Paned>() {
        let first = paned
            .start_child()
            .and_then(|child| session_layout_recursive(&child, panes, active_pane_index));
        let second = paned
            .end_child()
            .and_then(|child| session_layout_recursive(&child, panes, active_pane_index));
        return match (first, second) {
            (Some(first), Some(second)) => Some(
                impulse_core::session_state::SessionTerminalPaneLayout::Split(
                    impulse_core::session_state::SessionTerminalPaneSplit {
                        axis: session_axis(paned.orientation()),
                        ratio: paned_ratio(paned),
                        first: Box::new(first),
                        second: Box::new(second),
                    },
                ),
            ),
            (Some(layout), None) | (None, Some(layout)) => Some(layout),
            (None, None) => None,
        };
    }

    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(layout) = session_layout_recursive(&c, panes, active_pane_index) {
            return Some(layout);
        }
        child = c.next_sibling();
    }
    None
}

fn session_axis(orientation: gtk4::Orientation) -> impulse_core::session_state::SessionSplitAxis {
    match orientation {
        gtk4::Orientation::Horizontal => impulse_core::session_state::SessionSplitAxis::Horizontal,
        gtk4::Orientation::Vertical => impulse_core::session_state::SessionSplitAxis::Vertical,
        _ => impulse_core::session_state::SessionSplitAxis::Horizontal,
    }
}

fn orientation_from_session_axis(
    axis: impulse_core::session_state::SessionSplitAxis,
) -> gtk4::Orientation {
    match axis {
        impulse_core::session_state::SessionSplitAxis::Horizontal => gtk4::Orientation::Horizontal,
        impulse_core::session_state::SessionSplitAxis::Vertical => gtk4::Orientation::Vertical,
    }
}

fn paned_ratio(paned: &gtk4::Paned) -> f32 {
    let dimension = match paned.orientation() {
        gtk4::Orientation::Horizontal => paned.width(),
        gtk4::Orientation::Vertical => paned.height(),
        _ => 0,
    };
    if dimension <= 0 {
        return 0.5;
    }
    ((paned.position().max(0) as f32) / (dimension as f32)).clamp(0.1, 0.9)
}

fn default_terminal_directory() -> String {
    impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string())
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn collect_terminals_recursive(widget: &gtk4::Widget, terminals: &mut Vec<terminal::Terminal>) {
    if let Some(term) = terminal::from_widget(widget) {
        terminals.push(term);
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
pub fn remove_terminal(container: &gtk4::Widget, terminal: &terminal::Terminal) -> bool {
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
    let sibling = if is_start { end.clone() } else { start.clone() };
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
        let is_start_of_parent = parent_paned.start_child().as_ref() == Some(&paned_widget);
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
