use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::terminal;

/// A container that wraps a single Impulse terminal in a `gtk4::Box`.
///
/// Terminals used to support split panes here; splits never fit the Warp
/// input-bar model (one shared bar, many panes), so the feature was removed
/// on both frontends. Old saved sessions that stored split layouts restore
/// as their active pane.
pub struct TerminalContainer {
    pub widget: gtk4::Box,
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
        // Old sessions may carry multiple panes from the split era; restore
        // only the active pane.
        let cwd = active_session_pane(tab)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.trim().is_empty())
            .unwrap_or_else(default_terminal_directory);

        let term = terminal::create_terminal(settings, theme, copy_on_select_flag);
        setup_terminal(&term);
        terminal::spawn_shell(&term, shell_cache, Some(cwd.as_str()));

        let container = TerminalContainer::new(&term);

        gtk4::glib::idle_add_local_once(move || {
            term.grab_focus();
        });

        container
    }
}

fn active_session_pane(
    tab: &impulse_core::session_state::SessionTerminalTab,
) -> Option<impulse_core::session_state::SessionTerminalPane> {
    if tab.panes.is_empty() {
        return Some(impulse_core::session_state::SessionTerminalPane {
            cwd: tab.cwd.clone(),
            title: tab.title.clone(),
            shell: tab.shell.clone(),
        });
    }
    let active_index = tab.active_pane_index.unwrap_or(0);
    tab.panes
        .get(active_index)
        .or_else(|| tab.panes.first())
        .cloned()
}

/// Get the terminal within the widget tree, if any. Kept as a tree search so
/// callers can pass any tab child widget without knowing the wrapper layout.
pub fn get_active_terminal(widget: &gtk4::Widget) -> Option<terminal::Terminal> {
    if let Some(term) = terminal::from_widget(widget) {
        return Some(term);
    }

    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(term) = get_active_terminal(&c) {
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

/// Snapshot the terminal in this container for session persistence.
pub fn session_snapshot(
    widget: &gtk4::Widget,
) -> Option<impulse_core::session_state::SessionTerminalPane> {
    let term = get_active_terminal(widget)?;
    Some(impulse_core::session_state::SessionTerminalPane {
        cwd: terminal::current_directory(&term).unwrap_or_else(default_terminal_directory),
        title: non_empty_string(terminal::title(&term)),
        shell: None,
    })
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
