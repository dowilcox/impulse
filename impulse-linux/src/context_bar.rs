//! Warp-style terminal context/input bar shown between the tab content and
//! the status bar. Mirrors the macOS `TerminalContextBarView`: context chips
//! (shell, cwd, git branch with switcher popover, last command status), a
//! command input entry with history ghost autosuggestions, ↑/↓ history
//! cycling, a Tab-triggered path completion dropdown, and history/clear
//! action buttons.

use gtk4::prelude::*;
use libadwaita as adw;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use impulse_core::completion::{CompletionCandidate, CompletionResult};

use crate::settings::Settings;
use crate::terminal;
use crate::terminal_container;

/// Fetches dropdown candidates for the current input text.
type FetchCandidatesFn = Rc<dyn Fn(&str) -> Option<CompletionResult>>;
/// Applies a fetch result to the dropdown; returns whether it is open.
type ApplyCandidatesFn = Rc<dyn Fn(Option<CompletionResult>, &str) -> bool>;

/// Shared mutable state for the input entry's suggestion/history/completion
/// machinery (mirrors the @State fields of the macOS view).
#[derive(Default)]
struct InputState {
    /// Full completed line for the current text, rendered as ghost text.
    suggestion: Option<String>,
    /// Index into the recent-history list while cycling with ↑/↓; None = live draft.
    history_index: Option<usize>,
    /// The draft text saved when history cycling starts.
    saved_draft: String,
    /// Candidates for the active token. Non-empty == dropdown open.
    candidates: Vec<CompletionCandidate>,
    /// Byte range in the input that an accepted candidate replaces.
    span: (usize, usize),
    /// Highlighted candidate index.
    selected: usize,
    /// Guard so programmatic set_text (history cycling, accepting a
    /// completion) doesn't feed back through connect_changed.
    setting_text: bool,
}

pub struct ContextBar {
    pub widget: gtk4::Box,
    cwd_chip: gtk4::Label,
    branch_chip: gtk4::Label,
    branch_btn: gtk4::MenuButton,
    current_branch: Rc<RefCell<String>>,
    review_btn: gtk4::Button,
    /// Callback opening the Review Changes tab (wired by the window).
    on_open_review: RefCell<Option<Rc<dyn Fn()>>>,
    /// Last time the review chip's counts were polled (throttled to ~2s).
    review_last_poll: Cell<Option<std::time::Instant>>,
    status_chip: gtk4::Label,
    stop_btn: gtk4::Button,
    /// Whether a command was running at the last refresh, for detecting the
    /// running→idle transition (which reclaims focus for the input).
    last_running: Cell<bool>,
    entry: gtk4::Entry,
    ghost: gtk4::Label,
    completion_popover: gtk4::Popover,
    input_state: Rc<RefCell<InputState>>,
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
    // Two-row layout mirroring macOS: a context-chip row (shell, cwd,
    // branch, review, status + action buttons) above the command input row.
    let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    widget.add_css_class("context-bar");

    let chip_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    widget.append(&chip_row);

    // Context chips: shell name, cwd, git branch, last command status
    let shell_chip = gtk4::Label::new(Some(&impulse_core::shell::get_default_shell_name()));
    shell_chip.add_css_class("context-chip");
    chip_row.append(&shell_chip);

    let cwd_chip = gtk4::Label::new(None);
    cwd_chip.add_css_class("context-chip");
    cwd_chip.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    cwd_chip.set_max_width_chars(36);
    cwd_chip.set_visible(false);
    chip_row.append(&cwd_chip);

    // Branch chip: a button that opens a Warp-style branch switcher popover
    // (mirrors the macOS BranchChip + BranchPickerView).
    let branch_chip = gtk4::Label::new(None);
    branch_chip.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    branch_chip.set_max_width_chars(24);
    let current_branch = Rc::new(RefCell::new(String::new()));
    let branch_btn = gtk4::MenuButton::new();
    branch_btn.set_child(Some(&branch_chip));
    branch_btn.add_css_class("context-chip-button");
    branch_btn.set_always_show_arrow(true);
    branch_btn.set_tooltip_text(Some("Switch branch"));
    branch_btn.set_cursor_from_name(Some("pointer"));
    branch_btn.set_visible(false);
    branch_btn.set_popover(Some(&build_branch_popover(tab_view, &current_branch)));
    chip_row.append(&branch_btn);

    // Review Changes chip: changed-file count + aggregate +/- line counts.
    // Rendered only while the working tree has uncommitted changes; opens
    // the Review Changes tab when clicked (mirrors macOS ReviewChip).
    let review_btn = gtk4::Button::new();
    review_btn.add_css_class("context-chip-button");
    review_btn.set_tooltip_text(Some("Review Changes"));
    review_btn.set_cursor_from_name(Some("pointer"));
    review_btn.set_visible(false);
    chip_row.append(&review_btn);

    let status_chip = gtk4::Label::new(None);
    status_chip.add_css_class("context-chip");
    status_chip.set_visible(false);
    chip_row.append(&status_chip);

    // Spacer pushes the history/clear action buttons to the right edge.
    let chip_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    chip_spacer.set_hexpand(true);
    chip_row.append(&chip_spacer);

    // Input row: prompt glyph + command entry (+ Stop button while running).
    let input_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    input_row.add_css_class("context-input-row");
    widget.append(&input_row);

    let prompt_arrow = gtk4::Label::new(Some("\u{276f}"));
    prompt_arrow.add_css_class("context-prompt-arrow");
    input_row.append(&prompt_arrow);

    // Command input with a ghost-suggestion label layered behind the text
    // (typed prefix rendered invisible, completion suffix dimmed).
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("Run a command…"));
    entry.set_hexpand(true);
    entry.add_css_class("context-input");

    let ghost = gtk4::Label::new(None);
    ghost.add_css_class("context-ghost");
    ghost.set_halign(gtk4::Align::Start);
    ghost.set_valign(gtk4::Align::Center);
    ghost.set_can_target(false);
    ghost.set_visible(false);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_hexpand(true);
    entry_overlay.set_child(Some(&entry));
    entry_overlay.add_overlay(&ghost);
    input_row.append(&entry_overlay);

    // Stop button: sends SIGINT to the running command (visible while one runs).
    let stop_btn = gtk4::Button::from_icon_name("media-playback-stop-symbolic");
    stop_btn.add_css_class("flat");
    stop_btn.add_css_class("context-stop");
    stop_btn.set_tooltip_text(Some("Stop (Ctrl+C)"));
    stop_btn.set_cursor_from_name(Some("pointer"));
    stop_btn.set_visible(false);
    input_row.append(&stop_btn);
    {
        let tab_view = tab_view.clone();
        stop_btn.connect_clicked(move |_| {
            if let Some(term) = active_terminal(&tab_view) {
                terminal::write(&term, &[0x03]);
            }
        });
    }

    let input_state: Rc<RefCell<InputState>> = Rc::new(RefCell::new(InputState::default()));

    // Tab-triggered path completion dropdown, anchored above the entry. Not
    // autohiding so it never steals keyboard focus from the entry.
    let completion_list = gtk4::ListBox::new();
    completion_list.set_selection_mode(gtk4::SelectionMode::Single);
    completion_list.add_css_class("completion-list");

    let completion_scroll = gtk4::ScrolledWindow::new();
    completion_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    completion_scroll.set_max_content_height(320);
    completion_scroll.set_propagate_natural_height(true);
    completion_scroll.set_child(Some(&completion_list));

    let completion_popover = gtk4::Popover::new();
    completion_popover.add_css_class("completion-popover");
    completion_popover.set_position(gtk4::PositionType::Top);
    completion_popover.set_autohide(false);
    completion_popover.set_has_arrow(false);
    completion_popover.set_child(Some(&completion_scroll));
    completion_popover.set_parent(&entry);
    {
        let popover = completion_popover.clone();
        entry.connect_destroy(move |_| popover.unparent());
    }

    // Recompute the ghost suggestion for the current entry text.
    let refresh_suggestion: Rc<dyn Fn()> = Rc::new({
        let entry = entry.clone();
        let ghost = ghost.clone();
        let tab_view = tab_view.clone();
        let input_state = input_state.clone();
        move || {
            let text = entry.text().to_string();
            let suggestion = if text.is_empty() {
                None
            } else {
                active_terminal(&tab_view)
                    .filter(|term| !terminal::is_command_running(term))
                    .and_then(|term| {
                        let cwd = terminal::current_directory(&term);
                        let history = terminal::recent_commands(&term, 500);
                        impulse_core::completion::complete(&text, cwd.as_deref(), &history)
                    })
            };
            update_ghost(&ghost, &text, suggestion.as_deref());
            input_state.borrow_mut().suggestion = suggestion;
        }
    });

    // Fetch dropdown candidates for `input` from the active terminal.
    let fetch_candidates: FetchCandidatesFn = Rc::new({
        let tab_view = tab_view.clone();
        move |input: &str| {
            if input.is_empty() {
                return None;
            }
            let term = active_terminal(&tab_view)?;
            if terminal::is_command_running(&term) {
                return None;
            }
            let cwd = terminal::current_directory(&term);
            let history = terminal::recent_commands(&term, 500);
            Some(impulse_core::completion::complete_candidates(
                input,
                cwd.as_deref(),
                &history,
                50,
            ))
        }
    });

    // Apply a fetch result: open/refresh the dropdown when there are two or
    // more candidates, close it otherwise. Returns whether it is open.
    let apply_candidates: ApplyCandidatesFn = Rc::new({
        let input_state = input_state.clone();
        let list = completion_list.clone();
        let popover = completion_popover.clone();
        move |result, input| {
            let Some(result) = result.filter(|r| r.candidates.len() >= 2) else {
                input_state.borrow_mut().candidates.clear();
                popover.popdown();
                return false;
            };
            {
                let mut state = input_state.borrow_mut();
                state.candidates = result.candidates;
                state.span = (result.span.start, result.span.end);
                state.selected = 0;
            }
            populate_completion_list(&list, &input_state.borrow(), input);
            popover.popup();
            true
        }
    });

    let close_dropdown: Rc<dyn Fn()> = Rc::new({
        let input_state = input_state.clone();
        let popover = completion_popover.clone();
        move || {
            input_state.borrow_mut().candidates.clear();
            popover.popdown();
        }
    });

    // Accept the highlighted candidate: splice its (escaped) value over the
    // active token span. Directory candidates keep drilling on Tab/click
    // (`reopen_for_directory`); Enter and file candidates close the dropdown.
    let accept_completion: Rc<dyn Fn(bool)> = Rc::new({
        let entry = entry.clone();
        let input_state = input_state.clone();
        let fetch = fetch_candidates.clone();
        let apply = apply_candidates.clone();
        let close = close_dropdown.clone();
        let refresh_suggestion = refresh_suggestion.clone();
        move |reopen_for_directory: bool| {
            let (candidate, span) = {
                let state = input_state.borrow();
                let Some(candidate) = state.candidates.get(state.selected).cloned() else {
                    return;
                };
                (candidate, state.span)
            };
            let text = entry.text().to_string();
            let start = span.0.min(text.len());
            let end = span.1.clamp(start, text.len());
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                close();
                return;
            }
            // The candidate's value already carries any trailing "/" for
            // directories; files get a trailing space.
            let mut new_text = String::new();
            new_text.push_str(&text[..start]);
            new_text.push_str(&shell_quoted(&candidate.value));
            if !candidate.is_dir {
                new_text.push(' ');
            }
            new_text.push_str(&text[end..]);

            {
                let mut state = input_state.borrow_mut();
                state.history_index = None;
                state.setting_text = true;
            }
            entry.set_text(&new_text);
            entry.set_position(-1);
            input_state.borrow_mut().setting_text = false;
            refresh_suggestion();

            if candidate.is_dir && reopen_for_directory {
                apply(fetch(&new_text), &new_text);
            } else {
                close();
            }
        }
    });

    // Clicking a row accepts it (and keeps drilling into directories).
    {
        let input_state = input_state.clone();
        let accept = accept_completion.clone();
        completion_list.connect_row_activated(move |_, row| {
            input_state.borrow_mut().selected = row.index().max(0) as usize;
            accept(true);
        });
    }

    // Text changes recompute the ghost and narrow/close the open dropdown.
    {
        let input_state = input_state.clone();
        let refresh_suggestion = refresh_suggestion.clone();
        let fetch = fetch_candidates.clone();
        let apply = apply_candidates.clone();
        entry.connect_changed(move |entry| {
            if input_state.borrow().setting_text {
                return;
            }
            // Manual edits leave history-cycling mode.
            input_state.borrow_mut().history_index = None;
            refresh_suggestion();
            if !input_state.borrow().candidates.is_empty() {
                let text = entry.text().to_string();
                apply(fetch(&text), &text);
            }
        });
    }

    // Keyboard: Tab opens/accepts the dropdown or accepts the ghost, ↑/↓
    // move the dropdown highlight or cycle history, → accepts the next ghost
    // word, Enter accepts the highlighted candidate, Esc closes the dropdown
    // then moves focus to the terminal grid.
    {
        let input_state = input_state.clone();
        let tab_view_keys = tab_view.clone();
        let entry_keys = entry.clone();
        let ghost = ghost.clone();
        let list = completion_list.clone();
        let scroll = completion_scroll.clone();
        let accept = accept_completion.clone();
        let apply = apply_candidates.clone();
        let fetch = fetch_candidates.clone();
        let close = close_dropdown.clone();
        let refresh_suggestion = refresh_suggestion.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key_ctrl.connect_key_pressed(move |_, key, _keycode, modifiers| {
            use gtk4::gdk::Key;
            if modifiers.intersects(
                gtk4::gdk::ModifierType::CONTROL_MASK | gtk4::gdk::ModifierType::ALT_MASK,
            ) {
                // Ctrl+C while a command runs sends SIGINT to it (the input
                // bar owns the keyboard in the Warp model).
                if modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
                    && !modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
                    && (key == Key::c || key == Key::C)
                {
                    if let Some(term) = active_terminal(&tab_view_keys) {
                        if terminal::is_command_running(&term) {
                            terminal::write(&term, &[0x03]);
                            return gtk4::glib::Propagation::Stop;
                        }
                    }
                }
                return gtk4::glib::Propagation::Proceed;
            }
            let dropdown_open = !input_state.borrow().candidates.is_empty();
            match key {
                Key::Tab => {
                    if dropdown_open {
                        accept(true);
                        return gtk4::glib::Propagation::Stop;
                    }
                    let text = entry_keys.text().to_string();
                    if !text.is_empty() && !apply(fetch(&text), &text) {
                        // Zero/one candidate: fall back to the ghost.
                        accept_full_suggestion(&entry_keys, &input_state, &refresh_suggestion);
                    }
                    // Always swallow Tab so it never focus-traverses away.
                    gtk4::glib::Propagation::Stop
                }
                Key::Up | Key::KP_Up => {
                    if dropdown_open {
                        move_completion_selection(&input_state, &list, &scroll, -1);
                        gtk4::glib::Propagation::Stop
                    } else if cycle_history(&entry_keys, &ghost, &tab_view_keys, &input_state, 1) {
                        gtk4::glib::Propagation::Stop
                    } else {
                        gtk4::glib::Propagation::Proceed
                    }
                }
                Key::Down | Key::KP_Down => {
                    if dropdown_open {
                        move_completion_selection(&input_state, &list, &scroll, 1);
                        gtk4::glib::Propagation::Stop
                    } else if cycle_history(&entry_keys, &ghost, &tab_view_keys, &input_state, -1) {
                        gtk4::glib::Propagation::Stop
                    } else {
                        gtk4::glib::Propagation::Proceed
                    }
                }
                Key::Return | Key::KP_Enter => {
                    if dropdown_open {
                        // Enter commits without drilling into directories.
                        accept(false);
                        gtk4::glib::Propagation::Stop
                    } else {
                        gtk4::glib::Propagation::Proceed
                    }
                }
                Key::Right | Key::KP_Right => {
                    if accept_suggestion_word(&entry_keys, &input_state, &refresh_suggestion) {
                        gtk4::glib::Propagation::Stop
                    } else {
                        gtk4::glib::Propagation::Proceed
                    }
                }
                Key::Escape => {
                    if dropdown_open {
                        close();
                    } else if let Some(term) = active_terminal(&tab_view_keys) {
                        term.grab_focus();
                    }
                    gtk4::glib::Propagation::Stop
                }
                _ => gtk4::glib::Propagation::Proceed,
            }
        });
        entry.add_controller(key_ctrl);
    }

    // Focus loss dismisses the dropdown so it never lingers detached.
    {
        let close = close_dropdown.clone();
        let focus_ctrl = gtk4::EventControllerFocus::new();
        focus_ctrl.connect_leave(move |_| close());
        entry.add_controller(focus_ctrl);
    }

    // Action buttons: history + clear
    let history_btn = gtk4::Button::from_icon_name("document-open-recent-symbolic");
    history_btn.add_css_class("flat");
    history_btn.set_tooltip_text(Some("Command History"));
    history_btn.set_cursor_from_name(Some("pointer"));
    chip_row.append(&history_btn);

    let clear_btn = gtk4::Button::from_icon_name("edit-clear-all-symbolic");
    clear_btn.add_css_class("flat");
    clear_btn.set_tooltip_text(Some("Clear Terminal"));
    clear_btn.set_cursor_from_name(Some("pointer"));
    chip_row.append(&clear_btn);

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
        branch_btn,
        current_branch,
        review_btn: review_btn.clone(),
        on_open_review: RefCell::new(None),
        review_last_poll: Cell::new(None),
        status_chip,
        stop_btn,
        last_running: Cell::new(false),
        entry,
        ghost,
        completion_popover,
        input_state,
        tab_view: tab_view.clone(),
        enabled: Cell::new(settings.borrow().terminal_context_bar),
    });
    {
        let bar_weak = Rc::downgrade(&bar);
        review_btn.connect_clicked(move |_| {
            let open = bar_weak
                .upgrade()
                .and_then(|bar| bar.on_open_review.borrow().clone());
            if let Some(open) = open {
                open();
            }
        });
    }
    bar.refresh();
    bar
}

/// Render the ghost-suggestion label: the typed prefix invisible (so the
/// dimmed suffix lines up with the entry text), the completion suffix dimmed.
fn update_ghost(ghost: &gtk4::Label, text: &str, suggestion: Option<&str>) {
    match suggestion {
        Some(suggestion)
            if !text.is_empty() && suggestion != text && suggestion.starts_with(text) =>
        {
            let prefix = gtk4::glib::markup_escape_text(text);
            let suffix = gtk4::glib::markup_escape_text(&suggestion[text.len()..]);
            ghost.set_markup(&format!("<span alpha=\"1%\">{prefix}</span>{suffix}"));
            ghost.set_visible(true);
        }
        _ => ghost.set_visible(false),
    }
}

/// The typed basename prefix for the active token (text after the last `/`
/// within the span), used to embolden the matched part of each candidate.
fn candidate_matched_prefix(input: &str, span: (usize, usize)) -> String {
    let start = span.0.min(input.len());
    let end = span.1.clamp(start, input.len());
    if !input.is_char_boundary(start) || !input.is_char_boundary(end) {
        return String::new();
    }
    let token = &input[start..end];
    match token.rfind('/') {
        Some(idx) => token[idx + 1..].to_string(),
        None => token.to_string(),
    }
}

/// Rebuild the dropdown rows from the current candidates.
fn populate_completion_list(list: &gtk4::ListBox, state: &InputState, input: &str) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let prefix = candidate_matched_prefix(input, state.span);
    for candidate in &state.candidates {
        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);
        row_box.set_margin_top(3);
        row_box.set_margin_bottom(3);

        let icon = gtk4::Image::from_icon_name(if candidate.is_dir {
            "folder-symbolic"
        } else {
            "text-x-generic-symbolic"
        });
        row_box.append(&icon);

        let label = gtk4::Label::new(None);
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        label.add_css_class("completion-label");
        if !prefix.is_empty() && candidate.display.starts_with(&prefix) {
            label.set_markup(&format!(
                "<b>{}</b>{}",
                gtk4::glib::markup_escape_text(&prefix),
                gtk4::glib::markup_escape_text(&candidate.display[prefix.len()..]),
            ));
        } else {
            label.set_text(&candidate.display);
        }
        row_box.append(&label);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&row_box));
        list.append(&row);
    }
    if let Some(row) = list.row_at_index(state.selected as i32) {
        list.select_row(Some(&row));
    }
}

/// Move the dropdown highlight by `delta` rows, wrapping at the ends, and
/// keep the selected row scrolled into view.
fn move_completion_selection(
    input_state: &Rc<RefCell<InputState>>,
    list: &gtk4::ListBox,
    scroll: &gtk4::ScrolledWindow,
    delta: i32,
) {
    let selected = {
        let mut state = input_state.borrow_mut();
        let count = state.candidates.len() as i32;
        if count == 0 {
            return;
        }
        state.selected = (state.selected as i32 + delta).rem_euclid(count) as usize;
        state.selected as i32
    };
    if let Some(row) = list.row_at_index(selected) {
        list.select_row(Some(&row));
        // Scroll the row into view without moving keyboard focus off the entry.
        let y = row
            .compute_point(list, &gtk4::graphene::Point::zero())
            .map(|point| point.y() as f64)
            .unwrap_or(0.0);
        let height = row.height() as f64;
        let adj = scroll.vadjustment();
        if y < adj.value() {
            adj.set_value(y);
        } else if y + height > adj.value() + adj.page_size() {
            adj.set_value(y + height - adj.page_size());
        }
    }
}

/// Tab fallback: accept the whole ghost suggestion. No-op when none showing.
fn accept_full_suggestion(
    entry: &gtk4::Entry,
    input_state: &Rc<RefCell<InputState>>,
    refresh_suggestion: &Rc<dyn Fn()>,
) {
    let suggestion = {
        let state = input_state.borrow();
        match &state.suggestion {
            Some(s) if s != entry.text().as_str() && s.starts_with(entry.text().as_str()) => {
                s.clone()
            }
            _ => return,
        }
    };
    {
        let mut state = input_state.borrow_mut();
        state.history_index = None;
        state.setting_text = true;
    }
    entry.set_text(&suggestion);
    entry.set_position(-1);
    input_state.borrow_mut().setting_text = false;
    refresh_suggestion();
}

/// → accepts the next word of the ghost suggestion (up to and including the
/// next space or `/`) when the cursor sits at the end of the text. Returns
/// false to let the key move the cursor normally otherwise.
fn accept_suggestion_word(
    entry: &gtk4::Entry,
    input_state: &Rc<RefCell<InputState>>,
    refresh_suggestion: &Rc<dyn Fn()>,
) -> bool {
    let text = entry.text().to_string();
    if entry.position() != text.chars().count() as i32 {
        return false;
    }
    let suggestion = {
        let state = input_state.borrow();
        match &state.suggestion {
            Some(s) if !text.is_empty() && *s != text && s.starts_with(&text) => s.clone(),
            _ => return false,
        }
    };
    let remainder: Vec<char> = suggestion[text.len()..].chars().collect();
    if remainder.is_empty() {
        return false;
    }
    let is_boundary = |c: char| c == ' ' || c == '/';
    let mut end = 0;
    if is_boundary(remainder[0]) {
        end = 1;
    } else {
        while end < remainder.len() && !is_boundary(remainder[end]) {
            end += 1;
        }
        // Include the trailing boundary so the next press starts a fresh word.
        if end < remainder.len() && is_boundary(remainder[end]) {
            end += 1;
        }
    }
    let mut new_text = text;
    new_text.extend(&remainder[..end]);
    {
        let mut state = input_state.borrow_mut();
        state.history_index = None;
        state.setting_text = true;
    }
    entry.set_text(&new_text);
    entry.set_position(-1);
    input_state.borrow_mut().setting_text = false;
    refresh_suggestion();
    true
}

/// ↑/↓ history cycling. direction: +1 = older (↑), -1 = newer (↓). Returns
/// whether the key was consumed.
fn cycle_history(
    entry: &gtk4::Entry,
    ghost: &gtk4::Label,
    tab_view: &adw::TabView,
    input_state: &Rc<RefCell<InputState>>,
    direction: i32,
) -> bool {
    let recents = active_terminal(tab_view)
        .map(|term| terminal::recent_commands(&term, 50))
        .unwrap_or_default();
    if recents.is_empty() {
        return false;
    }

    let new_text = {
        let mut state = input_state.borrow_mut();
        let next = match (state.history_index, direction) {
            (None, 1) => {
                state.saved_draft = entry.text().to_string();
                Some(0)
            }
            (Some(index), 1) => Some((index + 1).min(recents.len() - 1)),
            (Some(index), -1) => index.checked_sub(1),
            _ => return false,
        };
        state.history_index = next;
        state.suggestion = None;
        state.setting_text = true;
        match next {
            Some(index) => recents[index].clone(),
            None => state.saved_draft.clone(),
        }
    };
    entry.set_text(&new_text);
    entry.set_position(-1);
    input_state.borrow_mut().setting_text = false;
    ghost.set_visible(false);
    true
}

/// Build the branch-switcher popover: a search field over the repo's local
/// branches, current branch pinned first. Selecting a branch runs
/// `git checkout` in the active terminal.
fn build_branch_popover(
    tab_view: &adw::TabView,
    current_branch: &Rc<RefCell<String>>,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    popover.add_css_class("branch-popover");

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_width_request(300);

    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search branches…"));
    content.append(&search);
    content.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("branch-list");

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_max_content_height(400);
    scrolled.set_propagate_natural_height(true);
    scrolled.set_child(Some(&list));
    content.append(&scrolled);

    popover.set_child(Some(&content));

    // Filter rows against the search text (row widget names hold the branch).
    {
        let search = search.clone();
        list.set_filter_func(move |row| {
            let query = search.text().to_lowercase();
            query.is_empty() || row.widget_name().to_lowercase().contains(&query)
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |_| list.invalidate_filter());
    }

    // (Re)populate the branch list every time the popover opens.
    {
        let tab_view = tab_view.clone();
        let current_branch = current_branch.clone();
        let search = search.clone();
        let list = list.clone();
        popover.connect_map(move |_| {
            search.set_text("");
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            let cwd = active_terminal(&tab_view)
                .and_then(|term| terminal::current_directory(&term))
                .unwrap_or_default();
            let branches = impulse_core::git::list_git_branches(&cwd).unwrap_or_default();
            let current = current_branch.borrow().clone();

            if branches.is_empty() {
                let empty = gtk4::Label::new(Some("No branches"));
                empty.add_css_class("dim-label");
                empty.set_margin_top(12);
                empty.set_margin_bottom(12);
                let row = gtk4::ListBoxRow::new();
                row.set_activatable(false);
                row.set_child(Some(&empty));
                list.append(&row);
                return;
            }

            // Current branch first, the rest in alphabetical order.
            let ordered = branches
                .iter()
                .filter(|b| **b == current)
                .chain(branches.iter().filter(|b| **b != current));
            for branch in ordered {
                let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                row_box.set_margin_start(8);
                row_box.set_margin_end(8);
                row_box.set_margin_top(4);
                row_box.set_margin_bottom(4);

                let icon = gtk4::Label::new(Some("\u{e0a0}"));
                icon.add_css_class("dim-label");
                row_box.append(&icon);

                let name = gtk4::Label::new(Some(branch));
                name.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                name.set_hexpand(true);
                name.set_halign(gtk4::Align::Start);
                row_box.append(&name);

                if *branch == current {
                    let check = gtk4::Image::from_icon_name("object-select-symbolic");
                    row_box.append(&check);
                }

                let row = gtk4::ListBoxRow::new();
                row.set_widget_name(branch);
                row.set_child(Some(&row_box));
                list.append(&row);
            }
        });
    }

    // Activating a row checks out that branch in the active terminal.
    {
        let tab_view = tab_view.clone();
        let current_branch = current_branch.clone();
        let popover_weak = popover.downgrade();
        list.connect_row_activated(move |_, row| {
            let branch = row.widget_name().to_string();
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
            if branch.is_empty() || branch == *current_branch.borrow() {
                return;
            }
            if let Some(term) = active_terminal(&tab_view) {
                terminal::write_text(&term, &format!("git checkout {}\n", shell_quoted(&branch)));
            }
        });
    }

    // Enter in the search field activates the first visible row.
    {
        let list = list.clone();
        search.connect_activate(move |search| {
            let query = search.text().to_lowercase();
            let mut child = list.first_child();
            while let Some(c) = child {
                if let Some(row) = c.downcast_ref::<gtk4::ListBoxRow>() {
                    let name = row.widget_name().to_lowercase();
                    if row.is_activatable() && (query.is_empty() || name.contains(&query)) {
                        row.emit_activate();
                        return;
                    }
                }
                child = c.next_sibling();
            }
        });
    }

    popover
}

/// Minimal POSIX single-quote escaping for a branch name (matches macOS).
fn shell_quoted(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl ContextBar {
    /// Update the enabled flag from a settings change and re-evaluate. Safe
    /// to call while the settings RefCell is mutably borrowed.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        self.refresh();
    }

    /// Wire the callback that opens the Review Changes tab.
    pub fn set_on_open_review(&self, open: Rc<dyn Fn()>) {
        *self.on_open_review.borrow_mut() = Some(open);
    }

    /// Focus the command input, appending a printable character forwarded
    /// from the read-only terminal grid so the first keystroke isn't lost.
    pub fn focus_input_with_char(&self, ch: Option<char>) {
        if !self.widget.is_visible() {
            return;
        }
        self.entry.grab_focus_without_selecting();
        if let Some(ch) = ch {
            let mut pos = self.entry.text().chars().count() as i32;
            self.entry.insert_text(&ch.to_string(), &mut pos);
            self.entry.set_position(pos);
        }
    }

    /// Update the Review Changes chip's counts for the terminal cwd `path`.
    /// Polled at most every ~2s (refresh runs on every command-block event)
    /// and computed off the main thread.
    fn update_review_chip(&self, path: &str) {
        let now = std::time::Instant::now();
        if let Some(last) = self.review_last_poll.get() {
            if now.duration_since(last) < std::time::Duration::from_secs(2) {
                return; // keep the current chip state
            }
        }
        self.review_last_poll.set(Some(now));

        let Some(repo_root) = impulse_core::git::get_git_root(path) else {
            self.review_btn.set_visible(false);
            return;
        };
        let review_btn = self.review_btn.clone();
        gtk4::glib::spawn_future_local(async move {
            let result = gtk4::gio::spawn_blocking(move || {
                impulse_core::git::list_changed_files(&repo_root)
            })
            .await;
            match result {
                Ok(Ok(set)) if !set.files.is_empty() => {
                    let files = if set.files.len() == 1 {
                        "1 file".to_string()
                    } else {
                        format!("{} files", set.files.len())
                    };
                    review_btn.set_label(&format!(
                        "{files} +{} \u{2212}{}",
                        set.total_added, set.total_removed
                    ));
                    review_btn.set_visible(true);
                }
                _ => review_btn.set_visible(false),
            }
        });
    }

    /// Hide the ghost suggestion and close the completion dropdown (bar
    /// hidden, tab switched, or a command started running).
    fn dismiss_input_overlays(&self) {
        self.ghost.set_visible(false);
        self.input_state.borrow_mut().candidates.clear();
        self.completion_popover.popdown();
    }

    /// Re-evaluate visibility and refresh all chips and the input state from
    /// the active tab. Called on tab switches, terminal CWD changes, command
    /// block start/end, and settings changes.
    pub fn refresh(&self) {
        let Some(term) = active_terminal(&self.tab_view).filter(|_| self.enabled.get()) else {
            self.widget.set_visible(false);
            self.dismiss_input_overlays();
            return;
        };

        // A full-screen/raw TUI (vim, htop, Claude Code) owns the grid: hide
        // the bar and hand the grid keyboard focus until the TUI exits.
        if terminal::tui_owns_grid(&term) {
            let was_visible = self.widget.is_visible();
            self.widget.set_visible(false);
            self.dismiss_input_overlays();
            if was_visible {
                term.grab_focus();
            }
            return;
        }
        self.widget.set_visible(true);

        // CWD + git branch + review chips
        match terminal::current_directory(&term) {
            Some(path) if !path.is_empty() => {
                self.cwd_chip.set_text(&abbreviate_home_path(&path));
                self.cwd_chip.set_visible(true);
                self.update_review_chip(&path);
                match impulse_core::filesystem::get_git_branch(&path) {
                    Ok(Some(branch)) if !branch.is_empty() => {
                        self.branch_chip.set_text(&branch);
                        *self.current_branch.borrow_mut() = branch;
                        self.branch_btn.set_visible(true);
                    }
                    _ => self.branch_btn.set_visible(false),
                }
            }
            _ => {
                self.cwd_chip.set_visible(false);
                self.branch_btn.set_visible(false);
                self.review_btn.set_visible(false);
            }
        }

        // Last command status chip + input running state. The input stays
        // usable while a command runs so line-based prompts (npm questions,
        // `read`, REPLs) can receive stdin; the branch switcher is inert — a
        // checkout typed into the terminal would land in the running program.
        let running = terminal::is_command_running(&term);
        let was_running = self.last_running.replace(running);
        if running {
            self.branch_btn.set_sensitive(false);
            self.status_chip.set_visible(false);
            self.stop_btn.set_visible(true);
            self.dismiss_input_overlays();
            self.entry
                .set_placeholder_text(Some("Send input to the running command…"));
        } else {
            if was_running {
                // Shell returned to the prompt — reclaim focus for the next
                // command (Warp model: the bar IS the prompt).
                self.entry.grab_focus_without_selecting();
            }
            self.branch_btn.set_sensitive(true);
            self.stop_btn.set_visible(false);
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
