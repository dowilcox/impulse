//! Warp-style terminal context/input bar shown between the tab content and
//! the status bar. Mirrors the macOS `TerminalContextBarView`: context chips
//! (shell, cwd, git branch, last command status), a command input entry, and
//! history/clear action buttons.
//!
//! TODO: macOS also has ghost autosuggestions from command history, Up/Down
//! history cycling, and live-prompt dimming while the input bar has focus
//! (see TerminalContextBarView.swift) — pending parity here.

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::settings::Settings;
use crate::terminal;
use crate::terminal_container;

pub struct ContextBar {
    pub widget: gtk4::Box,
    cwd_chip: gtk4::Label,
    branch_chip: gtk4::Label,
    status_chip: gtk4::Label,
    entry: gtk4::Entry,
    tab_view: adw::TabView,
    /// Mirrors `settings.terminal_context_bar`. Kept in a Cell (updated via
    /// `set_enabled`) instead of borrowing the settings RefCell, because
    /// settings-changed callbacks run while the settings are mutably borrowed.
    enabled: Cell<bool>,
}

/// The terminal in the selected tab page, if the selected tab is a terminal.
fn active_terminal(tab_view: &adw::TabView) -> Option<terminal::Terminal> {
    let page = tab_view.selected_page()?;
    terminal_container::get_active_terminal(&page.child())
}

/// Shorten the home directory prefix to `~` (matches the status bar).
pub(crate) fn abbreviate_home_path(path: &str) -> String {
    match impulse_core::shell::get_home_directory() {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

pub fn build_context_bar(
    tab_view: &adw::TabView,
    settings: &Rc<RefCell<Settings>>,
) -> Rc<ContextBar> {
    let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    widget.add_css_class("context-bar");

    // Context chips: shell name, cwd, git branch, last command status
    let shell_chip = gtk4::Label::new(Some(&impulse_core::shell::get_default_shell_name()));
    shell_chip.add_css_class("context-chip");
    widget.append(&shell_chip);

    let cwd_chip = gtk4::Label::new(None);
    cwd_chip.add_css_class("context-chip");
    cwd_chip.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    cwd_chip.set_max_width_chars(36);
    cwd_chip.set_visible(false);
    widget.append(&cwd_chip);

    let branch_chip = gtk4::Label::new(None);
    branch_chip.add_css_class("context-chip");
    branch_chip.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    branch_chip.set_max_width_chars(24);
    branch_chip.set_visible(false);
    widget.append(&branch_chip);

    let status_chip = gtk4::Label::new(None);
    status_chip.add_css_class("context-chip");
    status_chip.set_visible(false);
    widget.append(&status_chip);

    // Command input
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Run a command…"));
    entry.set_hexpand(true);
    entry.add_css_class("context-input");
    widget.append(&entry);

    // Action buttons: history + clear
    let history_btn = gtk4::Button::from_icon_name("document-open-recent-symbolic");
    history_btn.add_css_class("flat");
    history_btn.set_tooltip_text(Some("Command History"));
    history_btn.set_cursor_from_name(Some("pointer"));
    widget.append(&history_btn);

    let clear_btn = gtk4::Button::from_icon_name("edit-clear-all-symbolic");
    clear_btn.add_css_class("flat");
    clear_btn.set_tooltip_text(Some("Clear Terminal"));
    clear_btn.set_cursor_from_name(Some("pointer"));
    widget.append(&clear_btn);

    // Enter runs the typed command in the active terminal.
    {
        let tab_view = tab_view.clone();
        entry.connect_activate(move |entry| {
            let text = entry.text();
            let command = text.trim();
            if command.is_empty() {
                return;
            }
            if let Some(term) = active_terminal(&tab_view) {
                terminal::write_text(&term, &format!("{command}\n"));
            }
            entry.set_text("");
        });
    }

    {
        let tab_view = tab_view.clone();
        history_btn.connect_clicked(move |_| {
            if let Some(term) = active_terminal(&tab_view) {
                terminal::show_history(&term);
            }
        });
    }

    {
        let tab_view = tab_view.clone();
        clear_btn.connect_clicked(move |_| {
            if let Some(term) = active_terminal(&tab_view) {
                terminal::clear_screen(&term);
                term.grab_focus();
            }
        });
    }

    let bar = Rc::new(ContextBar {
        widget,
        cwd_chip,
        branch_chip,
        status_chip,
        entry,
        tab_view: tab_view.clone(),
        enabled: Cell::new(settings.borrow().terminal_context_bar),
    });
    bar.refresh();
    bar
}

impl ContextBar {
    /// Update the enabled flag from a settings change and re-evaluate. Safe
    /// to call while the settings RefCell is mutably borrowed.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        self.refresh();
    }

    /// Re-evaluate visibility and refresh all chips and the input state from
    /// the active tab. Called on tab switches, terminal CWD changes, command
    /// block start/end, and settings changes.
    pub fn refresh(&self) {
        let Some(term) = active_terminal(&self.tab_view).filter(|_| self.enabled.get()) else {
            self.widget.set_visible(false);
            return;
        };
        self.widget.set_visible(true);

        // CWD + git branch chips
        match terminal::current_directory(&term) {
            Some(path) if !path.is_empty() => {
                self.cwd_chip.set_text(&abbreviate_home_path(&path));
                self.cwd_chip.set_visible(true);
                match impulse_core::filesystem::get_git_branch(&path) {
                    Ok(Some(branch)) if !branch.is_empty() => {
                        self.branch_chip.set_text(&branch);
                        self.branch_chip.set_visible(true);
                    }
                    _ => self.branch_chip.set_visible(false),
                }
            }
            _ => {
                self.cwd_chip.set_visible(false);
                self.branch_chip.set_visible(false);
            }
        }

        // Last command status chip + input running state
        if terminal::is_command_running(&term) {
            self.status_chip.set_visible(false);
            self.entry.set_sensitive(false);
            self.entry
                .set_placeholder_text(Some("Running… (Ctrl+C in terminal to stop)"));
        } else {
            self.entry.set_sensitive(true);
            self.entry.set_placeholder_text(Some("Run a command…"));
            match terminal::last_command_status(&term) {
                Some((exit_code, duration_ms)) => {
                    self.status_chip
                        .set_text(&terminal::command_status_chip_text(exit_code, duration_ms));
                    if exit_code == 0 {
                        self.status_chip.remove_css_class("context-chip-error");
                        self.status_chip.add_css_class("context-chip-ok");
                    } else {
                        self.status_chip.remove_css_class("context-chip-ok");
                        self.status_chip.add_css_class("context-chip-error");
                    }
                    self.status_chip.set_visible(true);
                }
                None => self.status_chip.set_visible(false),
            }
        }
    }
}
