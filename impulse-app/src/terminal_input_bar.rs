//! The Warp-style input bar under a terminal (the Swift
//! `TerminalContextBarView`, with `TerminalTab`'s input-bar parts and
//! `CompletionPopupView`): a row of context chips (shell, folder, git
//! branch, how the last command ended) above an editor where commands are
//! typed with shell colors. Return runs the command (or sends the line to
//! a command that's running); Up and Down go through history; Tab opens
//! the completion menu (or inserts a lone candidate, or takes the ghost
//! suggestion); Right takes the suggestion, Alt+Right its next word; Escape
//! moves the keyboard to the grid. While the program reads a password the
//! editor masks what's typed.
//!
//! The bar is in the terminal that last had the keyboard; the others keep
//! its space with a quiet stand-in, so moving between split panes never
//! resizes their grids. It hides while a full-screen program, or a running
//! command that asked for raw input, owns the grid (`TerminalView::direct`).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{
    anchored, canvas, deferred, div, point, prelude::*, px, Anchor, AnyElement, Context, Entity,
    FontWeight, Hsla, MouseButton, Pixels, SharedString, Window,
};
use impulse_kit::chrome::Rgba as KitRgba;
use impulse_kit::completion::engine::{self, CompletionContext};
use impulse_kit::completion::{
    input, shell_completions, CompletionCandidate, CompletionResult, TextSpan,
};
use impulse_kit::shell::quoting;

use crate::command_editor::{CommandEditor, CommandEditorEvent, EditorKey};
use crate::command_history::CommandHistory;
use crate::settings_store::SettingsStore;
use crate::terminal_view::{format_block_duration, TerminalView, TerminalViewEvent};
use crate::workbench_chrome::{abbreviate_home, hsla};

/// How far the bar is from its top edge to the editor's first line, plus
/// the space under it: padding, the chip row, the gap and the input row's
/// padding and border.
const BAR_CHROME_HEIGHT: f32 = 8.0 + 22.0 + 6.0 + 14.0 + 8.0;
/// Rows the completion menu shows before scrolling.
const MENU_ROWS: usize = 10;
const MENU_ROW_HEIGHT: f32 = 28.0;
const MENU_WIDTH: f32 = 420.0;

thread_local! {
    /// The terminal that last had the keyboard: the bar lives there.
    static ACTIVE: Cell<Option<gpui_kit::EntityId>> = const { Cell::new(None) };
}

type ShellCache = Arc<Mutex<HashMap<String, (Instant, Vec<CompletionCandidate>)>>>;

/// The bar's state, kept per terminal (its draft is the editor's text).
#[derive(Default)]
pub(crate) struct InputBar {
    /// Index into the recent commands while going through them with Up and
    /// Down; `None` for the live draft.
    history_index: Option<usize>,
    saved_draft: String,
    /// The suggestion is a command from history (taken as it is), not a
    /// completion (already escaped).
    suggestion_from_history: bool,
    suggestion_generation: u64,
    /// Candidates for the token being typed; non-empty while the menu is open.
    completions: Vec<CompletionCandidate>,
    completion_span: Option<TextSpan>,
    /// The typed part of the token's last path segment (shown in bold).
    completion_prefix: String,
    selected: usize,
    completion_generation: u64,
    /// This session's commands and then everything else Impulse has seen.
    history_cache: Option<(u64, u64, Arc<Vec<String>>)>,
    /// Bumped when a command finishes (this session's history changed).
    pub session_revision: u64,
    shell_cache: ShellCache,
    /// The bar's width in the last frame (which chips fit).
    width: Rc<Cell<Pixels>>,
    /// The running command's spinner: its step, whether its timer runs, and
    /// when it was last drawn (the timer stops once it isn't).
    spinner_step: usize,
    spinner_ticking: bool,
    spinner_drawn: Option<Instant>,
}

fn c(hex: &str) -> KitRgba {
    KitRgba::from_hex(hex)
}

impl TerminalView {
    /// The bar is on and no program owns the grid.
    pub(crate) fn bar_shown(&self, cx: &gpui_kit::App) -> bool {
        SettingsStore::get(cx).terminal_context_bar && !self.direct && !self.no_bar
    }

    /// The bar is in this terminal (rather than its stand-in).
    pub(crate) fn has_live_bar(&self) -> bool {
        ACTIVE.with(|a| a.get()) == Some(self.entity_id)
    }

    pub(crate) fn note_active(&self) {
        ACTIVE.with(|a| a.set(Some(self.entity_id)));
    }

    /// The bar's height with one line of text, so stand-ins match it.
    pub(crate) fn bar_height(&self, window: &mut Window, cx: &mut gpui_kit::App) -> Pixels {
        let editor = self.editor.read(cx);
        let line = editor.height(window);
        px(BAR_CHROME_HEIGHT) + line
    }

    /// Text that belongs in the bar (typed in the grid at a prompt, a hint
    /// inserted): it goes in at the cursor and the bar takes the keyboard.
    pub(crate) fn insert_into_bar(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor
            .update(cx, |editor, cx| editor.insert_text(text, cx));
        let handle = self.editor.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    /// A command picked elsewhere (the palette's history): into the bar,
    /// replacing what's there, or typed at the grid's prompt when the bar
    /// isn't up. Not run.
    pub(crate) fn put_command(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bar_shown(cx) {
            self.set_bar_text(command.to_string(), cx);
            let handle = self.editor.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        } else {
            let bracketed = self
                .backend
                .mode()
                .contains(impulse_terminal::TerminalMode::BRACKETED_PASTE);
            self.backend
                .write(&impulse_kit::key_encoder::paste_bytes(command, bracketed));
            self.focus(window, cx);
        }
    }

    /// Text waiting in the bar for the user (a restored agent's resume
    /// command): typed, not run, and the keyboard stays where it is.
    pub(crate) fn set_draft(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_bar_text(text.to_string(), cx);
    }

    pub(crate) fn editor_text(&self, cx: &gpui_kit::App) -> String {
        self.editor.read(cx).text().to_string()
    }

    /// Replace the bar's text, as typing would have.
    fn set_bar_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
        self.bar_text_changed(cx);
    }

    fn set_suggestion(&mut self, suggestion: Option<String>, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            if editor.suggestion != suggestion {
                editor.suggestion = suggestion;
                cx.notify();
            }
        });
    }

    fn suggestion(&self, cx: &gpui_kit::App) -> Option<String> {
        self.editor.read(cx).suggestion.clone()
    }

    /// The editor said something.
    pub(crate) fn on_editor_event(
        &mut self,
        _: &Entity<CommandEditor>,
        event: &CommandEditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandEditorEvent::Changed => self.bar_text_changed(cx),
            CommandEditorEvent::Submit { .. } => self.submit(cx),
            CommandEditorEvent::Key(key) => self.editor_key(*key, window, cx),
            CommandEditorEvent::CopyNothing => {
                if let Some(text) = self.backend.selected_text().filter(|t| !t.is_empty()) {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                }
            }
            CommandEditorEvent::Clicked => {
                if !self.blocks.selection.is_empty() {
                    self.end_block_selection(false, window, cx);
                }
            }
        }
    }

    pub(crate) fn bar_text_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.editor_text(cx);
        if self.bar.history_index.is_some() && Some(text.clone()) != self.current_history_entry(cx)
        {
            self.bar.history_index = None;
        }
        self.update_suggestion(text.clone(), cx);
        // Typing never opens the menu, but narrows it while it's open.
        if !self.bar.completions.is_empty() {
            self.schedule_completions(text, cx);
        }
        cx.notify();
    }

    // MARK: Running commands

    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.editor_text(cx);
        if self.password {
            // Verbatim: passwords may carry spaces, and an empty line is an
            // empty password.
            self.send_line(&text);
            self.editor.update(cx, |e, cx| e.set_text("", cx));
            return;
        }
        if !self.bar.completions.is_empty() {
            // Return takes the highlighted candidate (and doesn't open a
            // folder's contents, as Tab does).
            return self.accept_completion(false, cx);
        }
        if self.command_running {
            // The program gets the line as typed; Return on an empty bar
            // answers "Press Enter to continue".
            self.send_line(&text);
            self.agent_key(true, cx);
        } else {
            let Some(command) = command_text(&text) else {
                return;
            };
            self.run_command(&command);
        }
        self.editor.update(cx, |e, cx| e.set_text("", cx));
        self.set_suggestion(None, cx);
        self.bar.history_index = None;
        self.bar.saved_draft.clear();
        self.close_menu(cx);
        cx.notify();
    }

    /// Run a command typed in the bar: the shell echoes it into the block.
    pub(crate) fn run_command(&mut self, command: &str) {
        // The shells leave a command typed with a leading space out of their
        // history; so does Impulse.
        self.hidden_pending = command.starts_with(' ');
        self.backend.scroll_to_bottom();
        if command.contains('\n')
            && self
                .backend
                .mode()
                .contains(impulse_terminal::TerminalMode::BRACKETED_PASTE)
        {
            // A multi-line command reaches the line editor as one paste and
            // runs as a whole.
            let body = quoting::without_control_characters(command);
            self.backend
                .write(format!("\x1b[200~{body}\x1b[201~\r").as_bytes());
        } else {
            self.backend.write(format!("{command}\n").as_bytes());
        }
    }

    /// A line for the running program, as the Return key would end it.
    fn send_line(&mut self, text: &str) {
        self.backend.scroll_to_bottom();
        self.backend.write(typed_line(text).as_bytes());
    }

    // MARK: Keys

    fn editor_key(&mut self, key: EditorKey, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.bar.completions.is_empty();
        match key {
            EditorKey::Up if open => self.move_selection(-1, cx),
            EditorKey::Down if open => self.move_selection(1, cx),
            EditorKey::Up | EditorKey::Down if !self.password => {
                self.cycle_history(if key == EditorKey::Up { 1 } else { -1 }, cx)
            }
            EditorKey::Tab if self.password => {}
            EditorKey::Tab if open => self.accept_completion(true, cx),
            EditorKey::Tab => self.request_completions(cx),
            EditorKey::Right => {
                self.accept_suggestion(cx);
            }
            EditorKey::WordRight => self.accept_suggestion_word(cx),
            EditorKey::Escape if open => self.close_menu(cx),
            EditorKey::Escape => window.focus(&self.focus, cx),
            EditorKey::ControlC => self.backend.write(&[0x03]),
            EditorKey::ControlR => cx.emit(TerminalViewEvent::RunCommand("command_history".into())),
            EditorKey::SelectBlock => {
                self.begin_block_selection(window, cx);
            }
            _ => {}
        }
    }

    fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.editor_text(cx);
        match self.suggestion(cx) {
            Some(s) if !text.is_empty() && s.starts_with(&text) && s != text => {
                self.set_bar_text(s, cx);
                true
            }
            _ => false,
        }
    }

    /// The suggestion's next word, up to and including the next space or `/`.
    fn accept_suggestion_word(&mut self, cx: &mut Context<Self>) {
        let text = self.editor_text(cx);
        let Some(suggestion) = self.suggestion(cx) else {
            return;
        };
        if text.is_empty() || !suggestion.starts_with(&text) || suggestion == text {
            return;
        }
        let rest: Vec<char> = suggestion[text.len()..].chars().collect();
        let boundary = |c: char| c == ' ' || c == '/';
        let mut end = 0;
        if boundary(rest[0]) {
            end = 1;
        } else {
            while end < rest.len() && !boundary(rest[end]) {
                end += 1;
            }
            if end < rest.len() {
                end += 1;
            }
        }
        let word: String = rest[..end].iter().collect();
        self.set_bar_text(format!("{text}{word}"), cx);
    }

    // MARK: History

    /// Recent commands, newest first: this session's, then Impulse's.
    pub(crate) fn recent_commands(&mut self, limit: usize, cx: &mut Context<Self>) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let session = self.session_commands(limit * 4, cx);
        session
            .into_iter()
            .chain(CommandHistory::recent(cx))
            .filter(|c| seen.insert(c.clone()))
            .take(limit)
            .collect()
    }

    /// This terminal's own commands, newest first and each once, without
    /// hidden ones or anything from before Clear Command History.
    fn session_commands(&mut self, records: usize, cx: &mut Context<Self>) -> Vec<String> {
        let cleared = CommandHistory::cleared_at_ms(cx);
        let mut seen = std::collections::HashSet::new();
        let history = self.backend.command_history();
        history
            .iter()
            .rev()
            .take(records)
            .filter(|r| r.ended_at_ms > cleared && !self.hidden_blocks.contains(&r.block_id))
            .map(|r| r.command.clone())
            .filter(|c| seen.insert(c.clone()))
            .collect()
    }

    /// History for suggestions, rebuilt only when either source changed.
    fn suggestion_history(&mut self, cx: &mut Context<Self>) -> Arc<Vec<String>> {
        let global = CommandHistory::revision(cx);
        if let Some((session, g, commands)) = &self.bar.history_cache {
            if *session == self.bar.session_revision && *g == global {
                return commands.clone();
            }
        }
        let session = self.session_commands(1000, cx);
        let in_session: std::collections::HashSet<String> = session.iter().cloned().collect();
        let commands: Vec<String> = session
            .into_iter()
            .chain(
                CommandHistory::recent(cx)
                    .into_iter()
                    .filter(|c| !in_session.contains(c)),
            )
            .collect();
        let commands = Arc::new(commands);
        self.bar.history_cache = Some((self.bar.session_revision, global, commands.clone()));
        commands
    }

    fn current_history_entry(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let index = self.bar.history_index?;
        self.recent_commands(50, cx).get(index).cloned()
    }

    /// `direction` +1 is older (Up), -1 newer (Down).
    fn cycle_history(&mut self, direction: i32, cx: &mut Context<Self>) {
        let recents = self.recent_commands(50, cx);
        if recents.is_empty() {
            return;
        }
        let next = match (self.bar.history_index, direction) {
            (None, 1) => {
                self.bar.saved_draft = self.editor_text(cx);
                Some(0)
            }
            (Some(i), 1) => Some((i + 1).min(recents.len() - 1)),
            (Some(i), _) => i.checked_sub(1),
            (None, _) => return,
        };
        self.bar.history_index = next;
        let text = match next {
            Some(i) => recents[i].clone(),
            None => self.bar.saved_draft.clone(),
        };
        self.set_bar_text(text, cx);
    }

    // MARK: Suggestions

    /// The ghost suggestion: a history continuation right away, otherwise
    /// word and path completion off the main thread. Meanwhile a suggestion
    /// the typing still follows stays up.
    fn update_suggestion(&mut self, text: String, cx: &mut Context<Self>) {
        self.bar.suggestion_generation += 1;
        if text.is_empty() || self.command_running || self.password {
            return self.set_suggestion(None, cx);
        }
        let history = self.suggestion_history(cx);
        if let Some(found) = input::history_continuation(&text, &history) {
            self.bar.suggestion_from_history = true;
            return self.set_suggestion(Some(found.to_string()), cx);
        }
        if let Some(current) = self.suggestion(cx) {
            if !(current.starts_with(&text) && current != text) {
                self.set_suggestion(None, cx);
            }
        }
        let generation = self.bar.suggestion_generation;
        let cwd = self.cwd_option();
        let shell = self.shell;
        let input = text.clone();
        let task =
            cx.background_spawn(
                async move { input::complete(&input, cwd.as_deref(), &history, shell) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.bar.suggestion_generation == generation && view.editor_text(cx) == text {
                    view.bar.suggestion_from_history = false;
                    view.set_suggestion(result, cx);
                }
            });
        })
        .detach();
    }

    fn cwd_option(&self) -> Option<String> {
        (!self.cwd.is_empty()).then(|| self.cwd.clone())
    }

    // MARK: Completion menu

    /// What completion runs with, read now; the result runs off the main
    /// thread.
    fn completion_resolver(
        &self,
        cx: &gpui_kit::App,
    ) -> impl FnOnce(&str) -> CompletionResult + Send {
        let cwd = self.cwd_option();
        let shell_path = impulse_kit::shell::login::default_shell_path();
        let fish = SettingsStore::get(cx).terminal_shell_completions
            && shell_completions::supports(&shell_path);
        let cache = self.bar.shell_cache.clone();
        move |text: &str| completion_candidates(text, cwd, fish.then_some(shell_path), cache)
    }

    /// Tab with the menu closed: two or more candidates open it, a lone one
    /// goes in as a shell's Tab would, and with none the suggestion is taken.
    pub(crate) fn request_completions(&mut self, cx: &mut Context<Self>) {
        let text = self.editor_text(cx);
        if text.is_empty() || self.command_running {
            self.accept_suggestion(cx);
            return;
        }
        self.bar.completion_generation += 1;
        let generation = self.bar.completion_generation;
        let resolve = self.completion_resolver(cx);
        let input = text.clone();
        let task = cx.background_spawn(async move { resolve(&input) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.bar.completion_generation == generation && view.editor_text(cx) == text {
                    view.apply_tab_completions(result, cx);
                }
            });
        })
        .detach();
    }

    fn apply_tab_completions(&mut self, result: CompletionResult, cx: &mut Context<Self>) {
        let text = self.editor_text(cx);
        if result.candidates.is_empty() {
            self.accept_suggestion(cx);
            return;
        }
        if result.candidates.len() == 1 {
            // A history suggestion on screen keeps Tab; a completion one
            // says the same as the candidate, which goes in escaped.
            if let Some(s) = self.suggestion(cx) {
                if self.bar.suggestion_from_history && s.starts_with(&text) && s != text {
                    self.accept_suggestion(cx);
                    return;
                }
            }
            self.bar.completions = result.candidates;
            self.bar.completion_span = Some(result.span);
            self.bar.selected = 0;
            return self.accept_completion(true, cx);
        }
        self.open_menu(&text, result, cx);
    }

    fn open_menu(&mut self, text: &str, result: CompletionResult, cx: &mut Context<Self>) {
        self.bar.completion_prefix = matched_prefix(text, result.span);
        self.bar.completions = result.candidates;
        self.bar.completion_span = Some(result.span);
        self.bar.selected = 0;
        cx.notify();
    }

    /// Fetch candidates again shortly (the menu follows typing).
    fn schedule_completions(&mut self, text: String, cx: &mut Context<Self>) {
        self.bar.completion_generation += 1;
        let generation = self.bar.completion_generation;
        if text.is_empty() || self.command_running {
            return self.close_menu(cx);
        }
        let resolve = self.completion_resolver(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(40))
                .await;
            let current = this
                .update(cx, |view, _| view.bar.completion_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let input = text.clone();
            let result = cx
                .background_executor()
                .spawn(async move { resolve(&input) })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.bar.completion_generation != generation {
                    return;
                }
                if result.candidates.len() >= 2 {
                    view.open_menu(&text, result, cx);
                } else {
                    view.close_menu(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.bar.completions.is_empty() && self.bar.completion_span.is_none() {
            return;
        }
        self.bar.completions.clear();
        self.bar.completion_span = None;
        self.bar.completion_prefix.clear();
        self.bar.selected = 0;
        self.bar.completion_generation += 1;
        cx.notify();
    }

    fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let count = self.bar.completions.len() as i32;
        if count == 0 {
            return;
        }
        self.bar.selected = ((self.bar.selected as i32 + delta).rem_euclid(count)) as usize;
        cx.notify();
    }

    /// Put the highlighted candidate in place of the token. A folder (which
    /// ends in `/`) opens the menu again for what's inside when `reopen`;
    /// anything else gets a space after it and closes the menu.
    fn accept_completion(&mut self, reopen: bool, cx: &mut Context<Self>) {
        let (Some(span), Some(candidate)) = (
            self.bar.completion_span,
            self.bar.completions.get(self.bar.selected).cloned(),
        ) else {
            return;
        };
        let text = self.editor_text(cx);
        if span.start > span.end
            || span.end > text.len()
            || !text.is_char_boundary(span.start)
            || !text.is_char_boundary(span.end)
        {
            return self.close_menu(cx);
        }
        // Escaped (a name like `x$(touch y)` stays a name), after a home
        // `~/` it was found under, which still expands.
        let replacement = candidate.insertion(&text[span.start..span.end], self.shell);
        let (prefix, suffix) = (&text[..span.start], &text[span.end..]);
        self.bar.history_index = None;
        if candidate.is_dir {
            let new = format!("{prefix}{replacement}{suffix}");
            if !reopen {
                self.close_menu(cx);
            }
            self.set_bar_text(new.clone(), cx);
            self.set_suggestion(None, cx);
            if reopen {
                self.schedule_completions(new, cx);
            }
        } else {
            self.close_menu(cx);
            self.set_bar_text(format!("{prefix}{replacement} {suffix}"), cx);
            self.set_suggestion(None, cx);
        }
    }

    // MARK: Drawing

    /// The live bar.
    pub(crate) fn input_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme.clone();
        let focused = self.editor.read(cx).focus_handle().is_focused(window);
        if !focused && !self.bar.completions.is_empty() {
            // The menu never lingers detached from the editor.
            self.bar.completions.clear();
            self.bar.completion_span = None;
        }
        // Keep the editor in step with the terminal.
        let running = self.command_running;
        let password = self.password;
        if running && !password {
            self.tick_spinner(cx);
        }
        let settings = SettingsStore::get(cx).clone();
        let font = gpui_kit::font(SharedString::from(settings.terminal_font_family.clone()));
        let font_size = px((settings.terminal_font_size - 1).max(6) as f32);
        let placeholder: SharedString = if password {
            "Password (input hidden)…".into()
        } else if running {
            "Send input to the running command…".into()
        } else {
            "Run a command…".into()
        };
        let cwd = self.cwd_option();
        let lookup = self.lookup.clone();
        self.editor.update(cx, |editor, cx| {
            let mut changed = false;
            if editor.placeholder != placeholder {
                editor.placeholder = placeholder;
                changed = true;
            }
            if editor.masked != password {
                editor.masked = password;
                changed = true;
            }
            if editor.font != font || editor.font_size != font_size {
                editor.font = font;
                editor.font_size = font_size;
                changed = true;
            }
            if running && editor.suggestion.is_some() {
                editor.suggestion = None;
                changed = true;
            }
            if editor.lookup.as_ref().map(|(_, c)| c) != Some(&cwd) {
                editor.lookup = Some((lookup, cwd));
            }
            if changed {
                cx.notify();
            }
        });

        let text_empty = self.editor.read(cx).text().is_empty();
        let accent = c(&theme.accent);
        let width = self.bar.width.clone();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .px(px(12.))
            .py(px(8.))
            .bg(hsla(c(&theme.bg_dark)))
            .border_t_1()
            .border_color(hsla(c(&theme.border)))
            .child(
                canvas(move |b, _, _| width.set(b.size.width), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(self.chip_row(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(hsla(c(&theme.bg)))
                    .border_1()
                    .border_color(hsla(if focused {
                        accent.opacity(0.6)
                    } else {
                        c(&theme.border)
                    }))
                    .child(self.leading_glyph())
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(self.editor.clone())
                            .when_some(self.completion_menu(cx), |d, menu| d.child(menu)),
                    )
                    .when(running, |d| {
                        d.child(
                            div()
                                .id("stop")
                                .flex_none()
                                .cursor_pointer()
                                // SF Symbols' stop.fill at 11 points.
                                .child(div().size(px(9.)).rounded(px(1.5)).bg(hsla(c(&theme.red))))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|view, _, _, cx| {
                                        cx.stop_propagation();
                                        view.backend.write(&[0x03]);
                                    }),
                                ),
                        )
                    })
                    .when(!running && !text_empty, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(hsla(c(&theme.fg_comment)))
                                .child("⏎ run"),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Step the spinner eight times a second, as the macOS one turns, while
    /// the command runs and the bar is drawn: a notify only per step rather
    /// than an animation's every frame, since a dev server can run for hours.
    fn tick_spinner(&mut self, cx: &mut Context<Self>) {
        self.bar.spinner_drawn = Some(Instant::now());
        if self.bar.spinner_ticking {
            return;
        }
        self.bar.spinner_ticking = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(125))
                .await;
            let go_on = this.update(cx, |view, cx| {
                let drawn = view
                    .bar
                    .spinner_drawn
                    .is_some_and(|at| at.elapsed() < Duration::from_millis(500));
                if !view.command_running || view.password || !drawn {
                    view.bar.spinner_ticking = false;
                    return false;
                }
                view.bar.spinner_step = view.bar.spinner_step.wrapping_add(1);
                cx.notify();
                true
            });
            if !matches!(go_on, Ok(true)) {
                break;
            }
        })
        .detach();
    }

    fn leading_glyph(&self) -> AnyElement {
        let theme = &self.theme;
        let glyph = if self.password {
            crate::workbench_chrome::icon("eye-off", 12., c(&theme.yellow)).into_any_element()
        } else if self.command_running {
            crate::workbench_chrome::spinner(c(&theme.fg), 13., self.bar.spinner_step)
                .into_any_element()
        } else {
            crate::workbench_chrome::icon("chevron-right", 12., c(&theme.accent)).into_any_element()
        };
        div()
            .w(px(14.))
            .flex_none()
            .flex()
            .justify_center()
            .child(glyph)
            .into_any_element()
    }

    /// Shell, folder, branch and the last command's status, as many as fit,
    /// with the history button at the end.
    fn chip_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = &self.theme;
        let mut chips: Vec<(Option<&'static str>, String, bool, Option<bool>)> = Vec::new();
        chips.push((
            Some("square-terminal"),
            self.shell_name.clone(),
            false,
            None,
        ));
        if !self.cwd.is_empty() {
            chips.push((Some("folder"), abbreviate_home(&self.cwd), false, None));
        }
        if let Some(branch) = self.branch.as_ref().filter(|b| !b.is_empty()) {
            chips.push((Some("split"), branch.clone(), true, None));
        }
        // The repository's changes (Swift's ReviewChip), opening Review.
        let review = (!self.cwd.is_empty())
            .then(|| {
                crate::repository_state::RepositoryStore::summary(
                    std::path::Path::new(&self.cwd),
                    cx,
                )
            })
            .flatten()
            .filter(|r| r.changed > 0);
        let review_at = review.as_ref().map(|r| {
            chips.push((
                Some("file"),
                format!("{} \u{2022} +{} -{}", r.changed, r.added, r.removed),
                false,
                None,
            ));
            chips.len() - 1
        });
        if let (false, Some(code)) = (self.command_running, self.last_exit) {
            let mut parts = Vec::new();
            if code != 0 {
                parts.push(format!("exit {code}"));
            }
            if let Some(ms) = self.last_duration_ms {
                parts.push(format_block_duration(ms));
            }
            let text = if parts.is_empty() {
                "ok".to_string()
            } else {
                parts.join(" · ")
            };
            chips.push((None, text, false, Some(code != 0)));
        }
        // Chips that don't fit drop off the end whole.
        let available = f32::from(self.bar.width.get()) - 24. - 22. - 6.;
        let char_width = 11. * 0.6;
        let mut used = 0.;
        let mut shown = 0;
        for (icon, text, chevron, status) in &chips {
            let width =
                (16. + if icon.is_some() || status.is_some() {
                    15.
                } else {
                    0.
                } + text.chars().count() as f32 * char_width
                    + if *chevron { 11. } else { 0. })
                .min(280.);
            let gap = if shown > 0 { 6. } else { 0. };
            if available > 0. && used + gap + width > available {
                break;
            }
            used += gap + width;
            shown += 1;
        }
        let mono = SettingsStore::get(cx).terminal_font_family.clone();
        let chip_bg = hsla(c(&theme.fg).opacity(0.07));
        let row = div()
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .gap(px(6.))
            .children(chips.into_iter().take(shown).enumerate().map(
                |(index, (icon, text, chevron, status))| {
                    if let (true, Some(r)) = (Some(index) == review_at, review.as_ref()) {
                        let mono = SharedString::from(mono.clone());
                        let part = |text: String, color: &str| {
                            div().text_color(hsla(c(color))).child(text)
                        };
                        return div()
                            .id("review-chip")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .h(px(20.))
                            .rounded(px(10.))
                            .bg(chip_bg)
                            .cursor_pointer()
                            .font_family(mono)
                            .text_size(px(11.))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::commands::RunCommand::new("review_changes")),
                                    cx,
                                )
                            })
                            .child(crate::workbench_chrome::icon(
                                "file",
                                11.,
                                c(&theme.fg_comment),
                            ))
                            .child(part(r.changed.to_string(), &theme.fg_muted))
                            .child(part("\u{2022}".into(), &theme.fg_comment))
                            .child(part(format!("+{}", r.added), &theme.green))
                            .child(part(format!("-{}", r.removed), &theme.red))
                            .into_any_element();
                    }
                    let leading = match status {
                        // A filled circle with the mark cut out (SF Symbols'
                        // checkmark.circle.fill and xmark.circle.fill).
                        Some(failed) => Some(
                            div()
                                .size(px(10.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(hsla(c(if failed { &theme.red } else { &theme.green })))
                                .child(crate::workbench_chrome::icon(
                                    if failed { "x" } else { "check" },
                                    7.,
                                    c(&theme.bg),
                                ))
                                .into_any_element(),
                        ),
                        None => icon.map(|i| {
                            crate::workbench_chrome::icon(i, 11., c(&theme.fg_comment))
                                .into_any_element()
                        }),
                    };
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .px(px(8.))
                        .h(px(20.))
                        .max_w(px(280.))
                        .rounded(px(10.))
                        .bg(chip_bg)
                        .when_some(leading, |d, icon| d.child(icon))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(SharedString::from(mono.clone()))
                                .text_size(px(11.))
                                .text_color(hsla(c(&theme.fg_muted)))
                                .child(text),
                        )
                        .when(chevron, |d| {
                            d.child(crate::workbench_chrome::icon(
                                "chevrons-up-down",
                                7.,
                                c(&theme.fg_comment),
                            ))
                        })
                        .into_any_element()
                },
            ));
        div()
            .flex()
            .items_center()
            .h(px(22.))
            .gap(px(6.))
            .child(row)
            .child(
                div()
                    .id("command-history")
                    .size(px(22.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(crate::workbench_chrome::icon(
                        "history",
                        11.,
                        c(&theme.fg_muted),
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(TerminalViewEvent::RunCommand("command_history".into()));
                        }),
                    ),
            )
            .into_any_element()
    }

    /// The menu above the editor, while it's open.
    fn completion_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.bar.completions.is_empty() {
            return None;
        }
        let theme = &self.theme;
        let count = self.bar.completions.len();
        let first = self
            .bar
            .selected
            .saturating_sub(MENU_ROWS / 2)
            .min(count.saturating_sub(MENU_ROWS));
        let prefix = self.bar.completion_prefix.clone();
        let mono = SharedString::from(SettingsStore::get(cx).terminal_font_family.clone());
        let rows = self
            .bar
            .completions
            .iter()
            .enumerate()
            .skip(first)
            .take(MENU_ROWS)
            .map(|(index, candidate)| {
                let selected = index == self.bar.selected;
                let (icon, color) = candidate_icon(candidate, theme);
                let (matched, rest) = match candidate.display.strip_prefix(prefix.as_str()) {
                    Some(rest) if !prefix.is_empty() => (prefix.clone(), rest.to_string()),
                    _ => (String::new(), candidate.display.clone()),
                };
                let (label, label_color) = trailing_label(candidate, theme);
                div()
                    .id(("completion", index))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .h(px(MENU_ROW_HEIGHT))
                    .rounded(px(5.))
                    .when(selected, |d| d.bg(hsla(c(&theme.accent).opacity(0.18))))
                    .cursor_pointer()
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(crate::workbench_chrome::icon(icon, 12., color)),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .text_size(px(12.5))
                            .when(!matched.is_empty(), |d| {
                                d.child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(hsla(c(&theme.accent)))
                                        .child(matched),
                                )
                            })
                            .child(div().text_color(hsla(c(&theme.fg))).child(rest)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(10.5))
                            .when(candidate.kind == "path", |d| d.font_family(mono.clone()))
                            .text_color(label_color)
                            .child(label),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.bar.selected = index;
                            view.accept_completion(true, cx);
                        }),
                    )
            });
        let menu = div()
            .id("completion-menu")
            .occlude()
            .w(px(MENU_WIDTH))
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded(px(10.))
            .border_1()
            .border_color(gpui_kit::white().opacity(0.08))
            .bg(hsla(c(&theme.bg_dark)))
            .shadow_lg()
            .children(rows);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(
                    deferred(
                        anchored()
                            .anchor(Anchor::BottomLeft)
                            .offset(point(px(0.), px(-13.)))
                            .snap_to_window()
                            .child(menu),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }

    /// Where the bar sits in a terminal that doesn't have it: the bar's
    /// background and top border, with a faint prompt mark.
    pub(crate) fn bar_placeholder(&self, height: Pixels) -> AnyElement {
        let theme = &self.theme;
        let border = c(&theme.border);
        let mark = border.mix(KitRgba::new(0.5, 0.5, 0.5, 1.0), 0.35);
        div()
            .relative()
            .flex_none()
            .h(height)
            .bg(hsla(c(&theme.bg_dark)))
            .border_t_1()
            .border_color(hsla(border))
            .child(
                div()
                    .absolute()
                    .left(px(26.))
                    .bottom(px(25. - 9.))
                    .h(px(18.))
                    .text_size(px(13.))
                    .text_color(hsla(mark))
                    .child("›"),
            )
            .into_any_element()
    }
}

/// A command as the bar sends it (`TerminalTab.commandText`): no control
/// characters, trimmed, but with one leading space kept when it was typed
/// with one (the shells leave those out of their history). `None` when
/// there's nothing to run.
pub fn command_text(command: &str) -> Option<String> {
    let clean: String = command
        .chars()
        .filter(|&ch| matches!(ch, '\t' | '\n' | '\r') || !quoting::is_control(ch))
        .collect();
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(if clean.starts_with([' ', '\t']) {
        format!(" {trimmed}")
    } else {
        trimmed.to_string()
    })
}

/// A line as the Return key would end it (`TerminalTab.typedLine`): every
/// line end becomes one CR, plus the final Return.
pub fn typed_line(text: &str) -> String {
    format!("{}\r", text.replace("\r\n", "\n").replace('\n', "\r"))
}

/// The typed part of the token's last path segment.
fn matched_prefix(input: &str, span: TextSpan) -> String {
    let Some(token) = input.get(span.start..span.end) else {
        return String::new();
    };
    match token.rfind('/') {
        Some(slash) => token[slash + 1..].to_string(),
        None => token.to_string(),
    }
}

/// Menu candidates for `text` (`TerminalTab.completionResolver`): commands,
/// subcommands, options and spec'd values, else paths, with fish's own
/// answers when asked for. Runs git and fish: call off the main thread.
fn completion_candidates(
    text: &str,
    cwd: Option<String>,
    fish: Option<String>,
    cache: ShellCache,
) -> CompletionResult {
    if text.is_empty() {
        return CompletionResult::default();
    }
    let dir = cwd.clone();
    let git = move |args: &[&str]| -> Vec<String> {
        let Some(dir) = &dir else { return Vec::new() };
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string)
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let git_branches = git.clone();
    let git_remotes = git.clone();
    let git_tags = git;
    let mut context = CompletionContext::new(cwd.as_deref())
        .git_branches(move || {
            git_branches(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        })
        .git_remotes(move || git_remotes(&["remote"]))
        .git_tags(move || git_tags(&["tag"]));
    if let Some(fish_path) = fish {
        let cwd = cwd.clone();
        context = context.shell_completions(move |line| {
            let key = format!("{}\0{line}", cwd.as_deref().unwrap_or(""));
            if let Some((at, hit)) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
                if at.elapsed() < Duration::from_secs(5) {
                    return hit;
                }
            }
            let found = shell_completions::fish(
                line,
                cwd.as_deref(),
                &fish_path,
                shell_completions::FISH_TIMEOUT,
            );
            if let Ok(mut c) = cache.lock() {
                if c.len() > 64 {
                    c.clear();
                }
                c.insert(key, (Instant::now(), found.clone()));
            }
            found
        });
    }
    engine::candidates(text, &context, engine::DEFAULT_LIMIT)
}

/// The icon and its color for a candidate (`CompletionPopupView.icon`).
fn candidate_icon(
    candidate: &CompletionCandidate,
    theme: &impulse_kit::theme::ResolvedTheme,
) -> (&'static str, KitRgba) {
    if candidate.kind == "path" {
        return if candidate.is_dir {
            ("folder", c(&theme.accent))
        } else {
            ("file", c(&theme.fg_muted))
        };
    }
    let icon = match candidate.kind.as_str() {
        "command" => "square-terminal",
        "subcommand" => "chevron-right",
        "option" => "flag",
        "branch" => "split",
        "tag" => "tag",
        "remote" | "host" => "globe",
        "script" => "play",
        "target" | "recipe" => "zap",
        _ => "code",
    };
    let color = if candidate.kind == "option" {
        c(&theme.fg_muted)
    } else {
        c(&theme.accent)
    };
    (icon, color)
}

/// The text at a candidate's right: its git status or kind for paths, its
/// description otherwise.
fn trailing_label(
    candidate: &CompletionCandidate,
    theme: &impulse_kit::theme::ResolvedTheme,
) -> (String, Hsla) {
    let comment = hsla(c(&theme.fg_comment));
    if candidate.kind != "path" {
        return (
            candidate.detail.clone().unwrap_or(candidate.kind.clone()),
            comment,
        );
    }
    if let Some(status) = candidate.git_status.as_ref().filter(|s| !s.is_empty()) {
        let color = match status.as_str() {
            "?" | "A" => c(&theme.git_added),
            "M" => c(&theme.git_modified),
            "D" => c(&theme.git_deleted),
            "R" => c(&theme.git_renamed),
            _ => c(&theme.fg_comment),
        };
        return (status.clone(), hsla(color));
    }
    (
        if candidate.is_dir { "dir" } else { "file" }.to_string(),
        comment,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_text_trims_but_keeps_one_leading_space() {
        assert_eq!(command_text("  ls -la  "), Some(" ls -la".into()));
        assert_eq!(command_text("ls\u{15}\n"), Some("ls".into()));
        assert_eq!(command_text("\techo hi"), Some(" echo hi".into()));
        assert_eq!(command_text(" \n "), None);
        assert_eq!(command_text("echo \u{1b}[31m"), Some("echo [31m".into()));
    }

    #[test]
    fn typed_line_ends_every_line_with_return() {
        assert_eq!(typed_line(""), "\r");
        assert_eq!(typed_line("a\r\nb\nc"), "a\rb\rc\r");
    }

    #[test]
    fn matched_prefix_is_the_last_path_segment() {
        let span = |s, e| TextSpan { start: s, end: e };
        assert_eq!(matched_prefix("ls src/ma", span(3, 9)), "ma");
        assert_eq!(matched_prefix("git che", span(4, 7)), "che");
        assert_eq!(matched_prefix("x", span(3, 9)), "");
    }
}
