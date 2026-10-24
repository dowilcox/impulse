//! The command palette and quick open (the Swift `PaletteModel`,
//! `PaletteView`, `PalettePanelController` and `MainWindowController+Palette`):
//! one floating panel, centered near the top of the window, whose query's
//! prefix picks the mode:
//!   (none) files · `>` commands · `:` go to line · `%` text in files ·
//!   `b:` branches · `t:` tabs · `w:` workspaces · `h:` history ·
//!   `task:` branches as tasks · `set:` settings · `@` symbols in the file ·
//!   `#` symbols in the project · `a:` project actions · `?` help.
//!
//! What the window offers the palette (the Swift `PaletteHost`) is taken as
//! a `Host` snapshot when it opens; a chosen row runs against the
//! `Workbench` once the panel has closed.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState, Position};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    actions, div, point, prelude::*, px, AnyElement, App, Context, Entity, Focusable, FontWeight,
    HighlightStyle, KeyBinding, ScrollHandle, StyledText, Subscription, Task, WeakEntity, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::command_palette::{CommandPaletteItem, RecentCommandStore};
use impulse_kit::document_symbols::OutlineSymbol;
use impulse_kit::fuzzy::{self, Match};
use impulse_kit::settings::Settings;

use crate::commands::AppCommand;
use crate::editor_tab::EditorTab;
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;
use crate::workbench_chrome::{abbreviate_home, hsla, icon};
use crate::workspace::{Kind, Surface};

actions!(
    impulse_palette,
    [SelectNext, SelectPrevious, Confirm, Dismiss]
);

/// The panel's key context.
const CONTEXT: &str = "Palette";
const WIDTH: f32 = 640.;
const MAX_LIST_HEIGHT: f32 = 380.;
const ROW_HEIGHT: f32 = 34.;
/// Recent commands kept (the Swift app keeps 20).
const MAX_RECENTS: usize = 20;

/// The field's keys: ↑↓ (and Ctrl-P / Ctrl-N, like most pickers), Return,
/// Escape. As deep as GPUI Kit's own `Input` bindings and registered after
/// them, so they win inside the palette's field.
pub fn bind_keys(cx: &mut App) {
    let c = Some("Palette > Input");
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, c),
        KeyBinding::new("down", SelectNext, c),
        KeyBinding::new("ctrl-p", SelectPrevious, c),
        KeyBinding::new("ctrl-n", SelectNext, c),
        KeyBinding::new("enter", Confirm, c),
        KeyBinding::new("escape", Dismiss, c),
    ]);
}

/// The prefix the palette opens with for command `id`, if it opens it.
pub fn prefix_for(id: &str) -> Option<&'static str> {
    Some(match id {
        "command_palette" => ">",
        "quick_open" => "",
        "go_to_line" => ":",
        "search_text" => "%",
        "switch_branch" => "b:",
        "switch_tab" => "t:",
        "switch_workspace" => "w:",
        "find_setting" => "set:",
        "go_to_symbol" => "@",
        "go_to_project_symbol" => "#",
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Files,
    Commands,
    GoToLine,
    Text,
    Branches,
    Tabs,
    Workspaces,
    History,
    TaskBranches,
    Settings,
    Symbols,
    WorkspaceSymbols,
    Actions,
    Help,
}

impl Mode {
    fn placeholder(self) -> &'static str {
        match self {
            Mode::Files => "Go to file…  (type > for commands, ? for help)",
            Mode::Commands => "Run a command…",
            Mode::GoToLine => "Go to line, e.g. 42 or 42:7",
            Mode::Text => "Search text in project…",
            Mode::Branches => "Switch to branch…",
            Mode::Tabs => "Switch to tab…",
            Mode::Workspaces => "Switch to workspace or open a folder…",
            Mode::History => "Search history…  @here @repo @failed @today",
            Mode::TaskBranches => "Open a branch as a new task…",
            Mode::Settings => "Find a setting…",
            Mode::Symbols => "Go to symbol in this file…",
            Mode::WorkspaceSymbols => "Go to symbol in the project…",
            Mode::Actions => "Run a project action…",
            Mode::Help => "Palette modes",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Mode::Files => "file",
            Mode::Commands => "command",
            Mode::GoToLine => "corner-down-left",
            Mode::Text => "search",
            Mode::Branches => "git-branch",
            Mode::Tabs => "layers",
            Mode::Workspaces => "folder-git-2",
            Mode::History => "history",
            Mode::TaskBranches => "git-branch-plus",
            Mode::Settings => "settings",
            Mode::Symbols | Mode::WorkspaceSymbols => "code",
            Mode::Actions => "play",
            Mode::Help => "info",
        }
    }

    /// What Return does, for the footer.
    fn enter_label(self) -> &'static str {
        match self {
            Mode::Commands => "run",
            Mode::History => "insert",
            Mode::Branches => "switch",
            _ => "open",
        }
    }
}

/// The mode a query picks, and the rest of it.
fn parse(query: &str) -> (Mode, &str) {
    const PREFIXES: [(&str, Mode); 13] = [
        (">", Mode::Commands),
        (":", Mode::GoToLine),
        ("%", Mode::Text),
        ("b:", Mode::Branches),
        ("t:", Mode::Tabs),
        ("w:", Mode::Workspaces),
        ("h:", Mode::History),
        ("task:", Mode::TaskBranches),
        ("set:", Mode::Settings),
        ("@", Mode::Symbols),
        ("#", Mode::WorkspaceSymbols),
        ("a:", Mode::Actions),
        ("?", Mode::Help),
    ];
    for (prefix, mode) in PREFIXES {
        if let Some(rest) = query.strip_prefix(prefix) {
            return (mode, if mode == Mode::Help { "" } else { rest });
        }
    }
    (Mode::Files, query)
}

/// Trim spaces and tabs, as Swift's `.whitespaces`.
fn trim(term: &str) -> &str {
    term.trim_matches([' ', '\t'])
}

type RowAction = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

/// One result row.
struct Row {
    id: String,
    /// A lucide icon name.
    icon: Option<&'static str>,
    title: String,
    /// Byte offsets in `title` to emphasize (fuzzy match positions).
    highlights: Vec<usize>,
    subtitle: Option<String>,
    trailing: Option<String>,
    run: RowAction,
}

impl Row {
    fn new(
        id: impl Into<String>,
        icon: Option<&'static str>,
        title: impl Into<String>,
        run: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> Self {
        Row {
            id: id.into(),
            icon,
            title: title.into(),
            highlights: Vec::new(),
            subtitle: None,
            trailing: None,
            run: Rc::new(run),
        }
    }

    fn highlights(mut self, positions: Vec<usize>) -> Self {
        self.highlights = positions;
        self
    }

    fn subtitle(mut self, subtitle: Option<String>) -> Self {
        self.subtitle = subtitle.filter(|s| !s.is_empty());
        self
    }

    fn trailing(mut self, trailing: Option<String>) -> Self {
        self.trailing = trailing;
        self
    }
}

/// A tab as the palette lists it.
struct TabInfo {
    id: u64,
    title: String,
    is_terminal: bool,
    workspace_name: String,
    directory: Option<String>,
    /// Its place in the titlebar's strip, when it's in the active workspace.
    position: Option<usize>,
}

struct WorkspaceInfo {
    id: u64,
    name: String,
    root: String,
    is_scratch: bool,
    tab_count: usize,
    is_active: bool,
}

/// What the window offers the palette, taken when it opens.
#[derive(Default)]
pub(crate) struct Host {
    root: String,
    trusted: bool,
    open_files: Vec<PathBuf>,
    tabs: Vec<TabInfo>,
    workspaces: Vec<WorkspaceInfo>,
    branch: Option<String>,
    editor: Option<Entity<EditorTab>>,
    commands: Vec<AppCommand>,
    settings: Settings,
}

pub struct Palette {
    workbench: WeakEntity<Workbench>,
    colors: ChromePalette,
    input: Entity<InputState>,
    visible: bool,
    host: Host,
    query: String,
    mode: Mode,
    rows: Vec<Row>,
    selected: usize,
    busy: bool,
    /// Text shown when there are no rows.
    empty_message: String,
    scroll: ScrollHandle,
    recents: RecentCommandStore,

    file_index: Arc<Vec<String>>,
    file_index_version: usize,
    file_index_root: String,
    file_index_date: Option<Instant>,
    /// Every file the last file query matched (in index order), so a query
    /// that only adds to it searches those instead of the whole index.
    file_matches: Option<(String, usize, Arc<Vec<String>>)>,
    /// Local branches then remote ones without a local branch: (name, remote).
    branches: Option<Vec<(String, bool)>>,
    /// Branches New Task from Branch can open: (name, remote).
    task_branches: Option<Vec<(String, bool)>>,
    symbols: Option<Vec<OutlineSymbol>>,
    generation: usize,
    /// The newest refresh's generation, readable from the background: a
    /// file ranking still running for an older one stops early.
    latest: Arc<AtomicUsize>,
    /// The current mode's pending work (a debounced search, a ranking);
    /// dropping it cancels it.
    work: Option<Task<()>>,
    index_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(
        workbench: WeakEntity<Workbench>,
        colors: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let changed = cx.subscribe_in(
            &input,
            window,
            |palette, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    // Return writes nothing back here, but a refresh for
                    // the same text would still reset the selection.
                    if query != palette.query {
                        palette.query = query;
                        palette.refresh(window, cx);
                    }
                }
            },
        );
        // Closes when the field loses the keyboard, as the Swift panel does
        // when it resigns key.
        let focus = input.read(cx).focus_handle(cx);
        let blurred = cx.on_blur(&focus, window, |palette, window, cx| {
            palette.dismiss(false, window, cx)
        });
        Self {
            workbench,
            colors,
            input,
            visible: false,
            host: Host::default(),
            query: String::new(),
            mode: Mode::Files,
            rows: Vec::new(),
            selected: 0,
            busy: false,
            empty_message: String::new(),
            scroll: ScrollHandle::new(),
            recents: RecentCommandStore::default(),
            file_index: Arc::default(),
            file_index_version: 0,
            file_index_root: String::new(),
            file_index_date: None,
            file_matches: None,
            branches: None,
            task_branches: None,
            symbols: None,
            generation: 0,
            latest: Arc::default(),
            work: None,
            index_task: None,
            _subscriptions: vec![changed, blurred],
        }
    }

    /// Open with `prefix` as the query (or refresh it if it's open).
    pub(crate) fn show(
        &mut self,
        prefix: &str,
        host: Host,
        colors: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host = host;
        self.colors = colors;
        self.visible = true;
        self.recents = load_recents();
        self.prepare(prefix, window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Reload the file index if the root changed or it's stale, forget
    /// cached branches and symbols, and set the initial query.
    fn prepare(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.branches = None;
        self.task_branches = None;
        self.symbols = None;
        let root = self.host.root.clone();
        let stale = self
            .file_index_date
            .is_none_or(|date| date.elapsed() > Duration::from_secs(20));
        if root != self.file_index_root || stale {
            self.file_index_root = root.clone();
            self.set_file_index(Vec::new());
            self.load_file_index(root, window, cx);
        }
        if self.query == prefix {
            self.refresh(window, cx);
        } else {
            self.set_query(prefix, window, cx);
        }
    }

    fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query = query.to_string();
        let value = self.query.clone();
        self.input
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.refresh(window, cx);
    }

    /// Close; Escape and a chosen row give the window its focus back.
    fn dismiss(&mut self, restore_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.work = None;
        // Don't keep the window's editor alive while closed.
        self.host.editor = None;
        cx.notify();
        if restore_focus {
            let workbench = self.workbench.clone();
            window.defer(cx, move |window, cx| {
                let _ = workbench.update(cx, |workbench, cx| workbench.focus_active(window, cx));
            });
        }
    }

    // MARK: Selection

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let count = self.rows.len() as isize;
        self.selected = ((self.selected as isize + delta + count) % count) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(self.selected, window, cx);
    }

    fn dismiss_action(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(true, window, cx);
    }

    /// Run row `index`: help rows change the query instead; with no rows,
    /// Return in go-to-line mode goes to the typed line.
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            if self.mode == Mode::GoToLine {
                let (_, term) = parse(&self.query);
                if let Some((line, column)) = parse_line(trim(term)) {
                    self.run(
                        Rc::new(move |w, window, cx| w.palette_go_to(line, column, window, cx)),
                        window,
                        cx,
                    );
                }
            }
            return;
        };
        if let Some(prefix) = row.id.strip_prefix("help:") {
            let prefix = prefix.to_string();
            self.set_query(&prefix, window, cx);
            return;
        }
        self.run(row.run.clone(), window, cx);
    }

    /// Close, let focus return to the window, then act.
    fn run(&mut self, action: RowAction, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(true, window, cx);
        let workbench = self.workbench.clone();
        window.defer(cx, move |window, cx| {
            let _ = workbench.update(cx, |workbench, cx| action(workbench, window, cx));
        });
    }

    // MARK: Modes

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.latest.store(self.generation, Ordering::Relaxed);
        self.work = None;
        let (mode, term) = parse(&self.query);
        let term = trim(term).to_string();
        self.input.update(cx, |input, cx| {
            input.set_placeholder(mode.placeholder(), window, cx)
        });
        self.mode = mode;
        self.selected = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
        match mode {
            Mode::Files => self.refresh_files(term, window, cx),
            Mode::Commands => self.refresh_commands(&term),
            Mode::GoToLine => self.refresh_go_to_line(&term),
            Mode::Text => self.refresh_text(term, window, cx),
            Mode::Branches => self.refresh_branches(&term, window, cx),
            Mode::Tabs => self.refresh_tabs(&term),
            Mode::Workspaces => self.refresh_workspaces(&term),
            Mode::History => self.not_yet("Command history"),
            Mode::TaskBranches => self.refresh_task_branches(&term, window, cx),
            Mode::Settings => self.refresh_settings(&term),
            Mode::Symbols => self.refresh_symbols(&term, window, cx),
            Mode::WorkspaceSymbols => self.refresh_workspace_symbols(term, window, cx),
            Mode::Actions => self.not_yet("Project actions"),
            Mode::Help => self.refresh_help(),
        }
        cx.notify();
    }

    /// A mode whose feature the Rust app doesn't have yet.
    fn not_yet(&mut self, feature: &str) {
        self.rows = Vec::new();
        self.empty_message = format!("{feature} isn't in the Rust app yet.");
    }

    // MARK: Files

    fn set_file_index(&mut self, files: Vec<String>) {
        self.file_index = Arc::new(files);
        self.file_index_version += 1;
    }

    fn load_file_index(&mut self, root: String, window: &mut Window, cx: &mut Context<Self>) {
        self.index_task = None;
        if root.is_empty() {
            return;
        }
        self.busy = true;
        let trusted = self.host.trusted;
        let listing = {
            let root = root.clone();
            cx.background_executor().spawn(async move {
                impulse_git::file_index::files(
                    &root,
                    impulse_git::file_index::DEFAULT_LIMIT,
                    trusted,
                )
            })
        };
        self.index_task = Some(cx.spawn_in(window, async move |this, cx| {
            let files = listing.await;
            let _ = this.update_in(cx, |palette, window, cx| {
                if palette.file_index_root != root {
                    return;
                }
                palette.set_file_index(files);
                palette.file_index_date = Some(Instant::now());
                palette.busy = false;
                if palette.mode == Mode::Files {
                    palette.refresh(window, cx);
                }
            });
        }));
    }

    fn refresh_files(&mut self, term: String, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.file_index_root.clone();
        if term.is_empty() {
            // Open files first, most useful when no query has been typed.
            self.rows = self
                .host
                .open_files
                .iter()
                .take(12)
                .map(|path| file_row(path, &root, &[]))
                .collect();
            self.empty_message = if self.busy {
                "Indexing files…".into()
            } else {
                format!("Type to search files in {}", display_root(&root))
            };
            return;
        }
        // A query that only adds to the last one can only match among its
        // matches (same case rules or stricter).
        let version = self.file_index_version;
        let candidates = self
            .file_matches
            .as_ref()
            .filter(|(last, v, _)| *v == version && term.starts_with(last.as_str()))
            .map(|(_, _, paths)| paths.clone())
            .unwrap_or_else(|| self.file_index.clone());
        let (generation, latest) = (self.generation, self.latest.clone());
        // Each keystroke asks for a ranking; older ones give up once a newer
        // one is asked for.
        let ranking = {
            let term = term.clone();
            cx.background_executor().spawn(async move {
                let matched = fuzzy::matches(
                    &candidates,
                    &term,
                    true,
                    || latest.load(Ordering::Relaxed) != generation,
                    |path| path.as_str(),
                )?;
                let paths: Vec<String> = matched.iter().map(|(p, _)| (*p).clone()).collect();
                let ranked: Vec<(String, Match)> = fuzzy::best(matched, 60)
                    .into_iter()
                    .map(|(p, m)| (p.clone(), m))
                    .collect();
                Some((paths, ranked))
            })
        };
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let Some((paths, ranked)) = ranking.await else {
                return;
            };
            let _ = this.update(cx, |palette, cx| {
                if palette.generation != generation {
                    return;
                }
                if palette.file_index_version == version {
                    palette.file_matches = Some((term, version, Arc::new(paths)));
                }
                palette.rows = ranked
                    .iter()
                    .map(|(path, m)| file_row(&Path::new(&root).join(path), &root, &m.positions))
                    .collect();
                palette.empty_message = if palette.busy {
                    "Indexing files…".into()
                } else {
                    "No matching files".into()
                };
                cx.notify();
            });
        }));
    }

    // MARK: Commands

    fn refresh_commands(&mut self, term: &str) {
        let recent = |id: &str| self.recents.items.iter().position(|r| r.id == id);
        let ranked: Vec<(&AppCommand, Vec<usize>)> = if term.is_empty() {
            let mut by_recent: Vec<&AppCommand> = self.host.commands.iter().collect();
            by_recent.sort_by(|a, b| {
                let (ra, rb) = (
                    recent(&a.id).unwrap_or(usize::MAX),
                    recent(&b.id).unwrap_or(usize::MAX),
                );
                ra.cmp(&rb).then_with(|| a.title.cmp(&b.title))
            });
            by_recent.into_iter().map(|c| (c, Vec::new())).collect()
        } else {
            // Match titles; fall back to "category keywords" so e.g. "git"
            // finds Review Changes. Recent commands get a small boost.
            let mut scored: Vec<(&AppCommand, Vec<usize>, i64)> = Vec::new();
            for command in &self.host.commands {
                let boost = recent(&command.id).map_or(0, |i| (20 - i as i64 * 2).max(0));
                if let Some(m) = fuzzy::fuzzy_match(term, &command.title, false) {
                    scored.push((command, m.positions, m.score + boost + 10));
                } else {
                    let text = std::iter::once(command.category.as_str())
                        .chain(command.keywords.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" ");
                    if let Some(m) = fuzzy::fuzzy_match(term, &text, false) {
                        scored.push((command, Vec::new(), m.score / 2));
                    }
                }
            }
            scored.sort_by_key(|s| std::cmp::Reverse(s.2));
            scored.into_iter().map(|(c, p, _)| (c, p)).collect()
        };
        self.rows = ranked
            .into_iter()
            .map(|(command, positions)| {
                let (category, shortcut) = (command.category.clone(), command.shortcut.clone());
                let command = command.clone();
                Row::new(
                    format!("cmd:{}", command.id),
                    command.icon,
                    command.title.clone(),
                    move |w, window, cx| {
                        record_recent(&command);
                        w.run_command(&command.id, window, cx);
                    },
                )
                .highlights(positions)
                .subtitle(Some(category))
                .trailing(shortcut)
            })
            .collect();
        self.empty_message = "No matching commands".into();
    }

    // MARK: Go to line

    fn refresh_go_to_line(&mut self, term: &str) {
        self.rows = Vec::new();
        if self.host.editor.is_none() {
            self.empty_message = "Open a file in the editor to go to a line".into();
            return;
        }
        match parse_line(term) {
            Some((line, column)) => {
                let label = match column {
                    Some(column) => format!("Go to line {line}, column {column}"),
                    None => format!("Go to line {line}"),
                };
                self.rows = vec![Row::new(
                    "line",
                    Some("corner-down-left"),
                    label,
                    move |w, window, cx| w.palette_go_to(line, column, window, cx),
                )];
            }
            None => self.empty_message = "Type a line number, optionally :column".into(),
        }
    }

    // MARK: Text search

    fn refresh_text(&mut self, term: String, window: &mut Window, cx: &mut Context<Self>) {
        self.rows = Vec::new();
        if term.chars().count() < 2 {
            self.empty_message = "Type at least 2 characters".into();
            self.busy = false;
            return;
        }
        let root = self.file_index_root.clone();
        if root.is_empty() {
            self.empty_message = "No project folder".into();
            return;
        }
        self.busy = true;
        self.empty_message = "Searching…".into();
        let generation = self.generation;
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let results = {
                let (root, term) = (root.clone(), term.clone());
                cx.background_executor()
                    .spawn(async move {
                        let case_sensitive = term.chars().any(char::is_uppercase);
                        impulse_git::search::search_contents(&root, &term, 200, case_sensitive)
                    })
                    .await
            };
            let _ = this.update(cx, |palette, cx| {
                if palette.generation != generation {
                    return;
                }
                palette.busy = false;
                palette.rows = results
                    .into_iter()
                    .map(|result| {
                        let relative = relative_to(&result.path, &root);
                        let line = result.line_number.unwrap_or(0);
                        let column = result.column_start;
                        let snippet = result.line_content.as_deref().unwrap_or("").trim();
                        let title = if snippet.is_empty() {
                            file_name(&relative)
                        } else {
                            snippet.to_string()
                        };
                        let path = PathBuf::from(&result.path);
                        Row::new(
                            format!("text:{}:{line}:{}", result.path, column.unwrap_or(0)),
                            Some("file-code"),
                            title,
                            move |w, window, cx| {
                                w.palette_open_file(
                                    &path,
                                    result.line_number,
                                    column.map(|c| c + 1),
                                    window,
                                    cx,
                                )
                            },
                        )
                        .subtitle(Some(format!("{relative}:{line}")))
                    })
                    .collect();
                palette.empty_message = format!("No matches for “{term}”");
                cx.notify();
            });
        }));
    }

    // MARK: Branches

    fn refresh_branches(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branches) = self.branches.as_ref() else {
            let root = self.file_index_root.clone();
            self.busy = true;
            self.rows = Vec::new();
            self.empty_message = "Loading branches…".into();
            let current = self.host.branch.clone();
            let listing = cx.background_executor().spawn(async move {
                let (local, remote) = impulse_git::operations::branches(&root);
                // Local branches (most recent first, current on top), then
                // remotes that don't already have a local branch.
                let mut all: Vec<(String, bool)> = local
                    .iter()
                    .filter(|b| Some(*b) == current.as_ref())
                    .chain(local.iter().filter(|b| Some(*b) != current.as_ref()))
                    .map(|b| (b.clone(), false))
                    .collect();
                all.extend(
                    remote
                        .into_iter()
                        .filter(|r| !local.iter().any(|l| l == local_name(r)))
                        .map(|r| (r, true)),
                );
                all
            });
            self.work = Some(cx.spawn_in(window, async move |this, cx| {
                let branches = listing.await;
                let _ = this.update_in(cx, |palette, window, cx| {
                    palette.busy = false;
                    palette.branches = Some(branches);
                    if palette.mode == Mode::Branches {
                        palette.refresh(window, cx);
                    }
                });
            }));
            return;
        };
        let current = self.host.branch.clone();
        let mut rows: Vec<Row> = fuzzy::rank(branches, term, usize::MAX, false, |b| &b.0)
            .into_iter()
            .map(|((name, remote), m)| {
                let (name, remote) = (name.clone(), *remote);
                let trailing = if Some(&name) == current.as_ref() {
                    Some("current".to_string())
                } else {
                    remote.then(|| "remote".to_string())
                };
                let is_current = trailing.as_deref() == Some("current");
                Row::new(
                    format!("branch:{name}"),
                    Some(if remote { "globe" } else { "git-branch" }),
                    name.clone(),
                    move |w, window, cx| {
                        if !is_current {
                            let target = if remote { local_name(&name) } else { &name };
                            w.palette_switch_branch(target.to_string(), false, window, cx);
                        }
                    },
                )
                .highlights(m.positions)
                .trailing(trailing)
            })
            .collect();
        if !term.is_empty()
            && !branches.iter().any(|(name, _)| name == term)
            && impulse_kit::git::refs::is_valid_ref_name(term)
        {
            let name = term.to_string();
            rows.push(
                Row::new(
                    format!("branch-create:{name}"),
                    Some("git-branch-plus"),
                    format!("Create branch “{name}”"),
                    move |w, window, cx| w.palette_switch_branch(name.clone(), true, window, cx),
                )
                .subtitle(current.map(|c| format!("from {c}"))),
            );
        }
        self.empty_message = if branches.is_empty() {
            "Not a git repository".into()
        } else {
            "No matching branches".into()
        };
        self.rows = rows;
    }

    /// `task:`: branches no worktree has checked out, local then remote
    /// (without a local branch of the same name), each opened as a task.
    fn refresh_task_branches(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branches) = self.task_branches.as_ref() else {
            let root = self.file_index_root.clone();
            self.busy = true;
            self.rows = Vec::new();
            self.empty_message = "Loading branches…".into();
            let listing = cx.background_executor().spawn(async move {
                let (local, remote) = impulse_git::operations::branches(&root);
                let checked_out: Vec<String> = impulse_git::operations::worktrees(&root)
                    .into_iter()
                    .filter_map(|w| w.branch)
                    .collect();
                let mut all: Vec<(String, bool)> = local
                    .iter()
                    .filter(|b| !checked_out.contains(b))
                    .map(|b| (b.clone(), false))
                    .collect();
                all.extend(
                    remote
                        .into_iter()
                        .filter(|r| {
                            !r.ends_with("/HEAD") && !local.iter().any(|l| l == local_name(r))
                        })
                        .map(|r| (r, true)),
                );
                all
            });
            self.work = Some(cx.spawn_in(window, async move |this, cx| {
                let branches = listing.await;
                let _ = this.update_in(cx, |palette, window, cx| {
                    palette.busy = false;
                    palette.task_branches = Some(branches);
                    if palette.mode == Mode::TaskBranches {
                        palette.refresh(window, cx);
                    }
                });
            }));
            return;
        };
        self.rows = fuzzy::rank(branches, term, usize::MAX, false, |b| &b.0)
            .into_iter()
            .map(|((name, remote), m)| {
                let (name, remote) = (name.clone(), *remote);
                Row::new(
                    format!("task-branch:{name}"),
                    Some(if remote { "globe" } else { "git-branch" }),
                    name.clone(),
                    move |w, window, cx| w.open_branch_as_task(name.clone(), remote, window, cx),
                )
                .highlights(m.positions)
                .trailing(remote.then(|| "remote".to_string()))
            })
            .collect();
        self.empty_message = if branches.is_empty() {
            "No branches to open: each one is checked out already".into()
        } else {
            "No matching branches".into()
        };
    }

    // MARK: Tabs

    fn refresh_tabs(&mut self, term: &str) {
        let show_workspace = self.host.workspaces.len() > 1;
        let secondary = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl+"
        };
        self.rows = fuzzy::rank(&self.host.tabs, term, usize::MAX, false, |t| &t.title)
            .into_iter()
            .map(|(tab, m)| {
                let subtitle = [
                    show_workspace.then(|| tab.workspace_name.clone()),
                    tab.directory.clone(),
                ]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let id = tab.id;
                Row::new(
                    format!("tab:{id}"),
                    Some(if tab.is_terminal {
                        "terminal"
                    } else {
                        "file-code"
                    }),
                    tab.title.clone(),
                    move |w, window, cx| w.palette_select_tab(id, window, cx),
                )
                .highlights(m.positions)
                .subtitle(Some(subtitle))
                .trailing(
                    tab.position
                        .filter(|p| *p < 9)
                        .map(|p| format!("{secondary}{}", p + 1)),
                )
            })
            .collect();
        self.empty_message = "No matching tabs".into();
    }

    // MARK: Workspaces

    fn refresh_workspaces(&mut self, term: &str) {
        let mut rows: Vec<Row> =
            fuzzy::rank(&self.host.workspaces, term, usize::MAX, false, |w| &w.name)
                .into_iter()
                .map(|(workspace, m)| {
                    let detail = format!(
                        "{} · {} tab{}",
                        if workspace.is_scratch {
                            "Follows the active tab".to_string()
                        } else {
                            workspace.root.clone()
                        },
                        workspace.tab_count,
                        if workspace.tab_count == 1 { "" } else { "s" }
                    );
                    let id = workspace.id;
                    Row::new(
                        format!("workspace:{id}"),
                        Some(if workspace.is_scratch {
                            "square-terminal"
                        } else {
                            "folder-git-2"
                        }),
                        workspace.name.clone(),
                        move |w, window, cx| w.palette_select_workspace(id, window, cx),
                    )
                    .highlights(m.positions)
                    .subtitle(Some(detail))
                    .trailing(workspace.is_active.then(|| "current".to_string()))
                })
                .collect();
        // Recent folders come with the Rust app's recent workspaces list.
        rows.push(Row::new(
            "workspace:open",
            Some("folder-open"),
            "Open Folder as Workspace…",
            |w, window, cx| w.run_command("open_workspace", window, cx),
        ));
        self.rows = rows;
        self.empty_message = "No matching workspaces".into();
    }

    // MARK: Settings

    fn refresh_settings(&mut self, term: &str) {
        let items: Vec<(impulse_kit::settings::SettingItem, String)> =
            impulse_kit::settings::catalog()
                .into_iter()
                .map(|item| {
                    let text = format!("{} {}", item.title, item.key);
                    (item, text)
                })
                .collect();
        let settings = &self.host.settings;
        self.rows = fuzzy::rank(&items, term, usize::MAX, false, |(_, text)| text)
            .into_iter()
            .map(|((item, _), _)| {
                let key = item.key;
                Row::new(
                    format!("setting:{key}"),
                    Some(item.category.icon()),
                    item.title,
                    move |w, window, cx| w.palette_open_setting(key, window, cx),
                )
                .subtitle(Some(format!(
                    "{} › {}",
                    item.category.title(),
                    item.section
                )))
                .trailing(item.is_modified(settings).then(|| "changed".to_string()))
            })
            .collect();
        self.empty_message = "No matching settings".into();
    }

    // MARK: Symbols

    fn refresh_symbols(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(list) = self.symbols.as_ref() {
            let entries: Vec<(&OutlineSymbol, Vec<usize>)> = if term.is_empty() {
                list.iter().map(|s| (s, Vec::new())).collect()
            } else {
                fuzzy::rank(list, term, usize::MAX, false, |s| &s.name)
                    .into_iter()
                    .map(|(s, m)| (s, m.positions))
                    .collect()
            };
            self.rows = entries
                .into_iter()
                .map(|(symbol, positions)| {
                    let indent = if term.is_empty() {
                        "  ".repeat(symbol.depth.min(6))
                    } else {
                        String::new()
                    };
                    let subtitle = [
                        symbol.container.join(" › "),
                        symbol.detail.clone().unwrap_or_default(),
                    ]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ");
                    let (line, column) = (symbol.line as u32, symbol.column as u32);
                    Row::new(
                        format!("symbol:{line}:{column}:{}", symbol.name),
                        Some(symbol_icon(symbol.kind)),
                        format!("{indent}{}", symbol.name),
                        move |w, window, cx| w.palette_go_to(line, Some(column), window, cx),
                    )
                    .highlights(positions.iter().map(|p| p + indent.len()).collect())
                    .subtitle(Some(subtitle))
                    .trailing(Some(format!(
                        "{} · {}",
                        symbol.kind_name(),
                        symbol.line
                    )))
                })
                .collect();
            self.empty_message = if list.is_empty() {
                "No symbols in this file".into()
            } else {
                "No matching symbols".into()
            };
            return;
        }
        self.rows = Vec::new();
        let Some(editor) = self.host.editor.clone() else {
            self.empty_message = "Open a file to see its symbols".into();
            return;
        };
        let Some(request) = editor.read(cx).document_symbols(cx) else {
            self.empty_message = "No language server for this file".into();
            return;
        };
        self.busy = true;
        self.empty_message = "Asking the language server…".into();
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let list = request.await.unwrap_or_default();
            let _ = this.update_in(cx, |palette, window, cx| {
                palette.busy = false;
                palette.symbols = Some(list);
                if palette.mode == Mode::Symbols {
                    palette.refresh(window, cx);
                }
            });
        }));
    }

    fn refresh_workspace_symbols(
        &mut self,
        term: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows = Vec::new();
        let Some(editor) = self.host.editor.clone() else {
            self.empty_message = "Open a file so its language server can search the project".into();
            return;
        };
        if term.is_empty() {
            self.empty_message = "Type a symbol name".into();
            return;
        }
        self.busy = true;
        self.empty_message = "Searching…".into();
        let generation = self.generation;
        let root = self.file_index_root.clone();
        // Debounced like text search: servers search the whole project.
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let Ok(request) = this.update(cx, |_, cx| {
                editor.read(cx).workspace_symbols(term.clone(), cx)
            }) else {
                return;
            };
            let results = match request {
                Some(request) => Some(request.await.unwrap_or_default()),
                None => None,
            };
            let _ = this.update(cx, |palette, cx| {
                if palette.generation != generation {
                    return;
                }
                palette.busy = false;
                let Some(results) = results else {
                    palette.empty_message = "No language server for this file".into();
                    cx.notify();
                    return;
                };
                palette.rows = fuzzy::rank(&results, &term, 200, false, |(s, _)| &s.name)
                    .into_iter()
                    .map(|((symbol, path), m)| {
                        let relative = relative_to(path, &root);
                        let subtitle = [
                            symbol.container.join(" › "),
                            format!("{relative}:{}", symbol.line),
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("  ");
                        let (line, column) = (symbol.line as u32, symbol.column as u32);
                        let path = PathBuf::from(path);
                        Row::new(
                            format!("wsymbol:{}:{line}:{}", path.display(), symbol.name),
                            Some(symbol_icon(symbol.kind)),
                            symbol.name.clone(),
                            move |w, window, cx| {
                                w.palette_open_file(&path, Some(line), Some(column), window, cx)
                            },
                        )
                        .highlights(m.positions)
                        .subtitle(Some(subtitle))
                        .trailing(Some(symbol.kind_name().to_string()))
                    })
                    .collect();
                palette.empty_message = format!("No symbols match “{term}”");
                cx.notify();
            });
        }));
    }

    // MARK: Help

    fn refresh_help(&mut self) {
        const MODES: [(&str, &str, &str); 13] = [
            ("", "Go to file", "file"),
            (">", "Run a command", "command"),
            (":", "Go to line in the current file", "corner-down-left"),
            ("%", "Search text in the project", "search"),
            ("b:", "Switch branch", "git-branch"),
            ("t:", "Switch tab", "layers"),
            ("w:", "Switch workspace", "folder-git-2"),
            ("h:", "Search command history", "history"),
            ("task:", "Open a branch as a new task", "git-branch-plus"),
            ("set:", "Find a setting", "settings"),
            ("@", "Go to a symbol in this file", "code"),
            ("#", "Go to a symbol in the project", "code"),
            ("a:", "Run a project action", "play"),
        ];
        self.rows = MODES
            .iter()
            .map(|(prefix, title, icon)| {
                Row::new(format!("help:{prefix}"), Some(*icon), *title, |_, _, _| {}).trailing(
                    Some(if prefix.is_empty() {
                        "type a name".to_string()
                    } else {
                        prefix.to_string()
                    }),
                )
            })
            .collect();
        self.empty_message = String::new();
    }

    // MARK: Drawing

    fn field(&self) -> impl IntoElement {
        let p = &self.colors;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .h(px(46.))
            .child(icon(self.mode.icon(), 15., p.text_tertiary))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(15.))
                    .text_color(hsla(p.text))
                    .child(
                        Input::new(&self.input)
                            .appearance(false)
                            .px_0()
                            .text_size(px(15.))
                            .aria_label("Palette query"),
                    ),
            )
            .when(self.busy, |d| {
                d.child(
                    Spinner::new()
                        .color(hsla(p.text_tertiary))
                        .with_size(px(12.)),
                )
            })
    }

    fn results(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.colors;
        if self.rows.is_empty() {
            return div()
                .flex()
                .items_center()
                .px(px(16.))
                .h(px(44.))
                .text_size(px(12.))
                .text_color(hsla(p.text_tertiary))
                .child(self.empty_message.clone())
                .into_any_element();
        }
        let height = (self.rows.len() as f32 * ROW_HEIGHT + 10.).min(MAX_LIST_HEIGHT);
        div()
            .id("palette-results")
            .h(px(height))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .py(px(5.))
            .children(
                self.rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| self.row(index, row, cx)),
            )
            .into_any_element()
    }

    fn row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.colors;
        let selected = index == self.selected;
        div()
            .id(("palette-row", index))
            .group("palette-row")
            .flex_none()
            .h(px(ROW_HEIGHT))
            .px(px(6.))
            .cursor_pointer()
            .on_click(
                cx.listener(move |palette, _, window, cx| palette.activate(index, window, cx)),
            )
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(6.))
                    .rounded(px(metrics::RADIUS))
                    .when(selected, |d| d.bg(hsla(p.selection_strong)))
                    .when(!selected, |d| {
                        d.group_hover("palette-row", |s| s.bg(hsla(p.hover)))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(18.))
                            .children(row.icon.map(|name| icon(name, 15., p.text_secondary))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_middle()
                            .text_size(px(13.))
                            .text_color(hsla(p.text))
                            .child(highlighted(&row.title, &row.highlights, p)),
                    )
                    .children(row.subtitle.clone().map(|subtitle| {
                        let mut subtitle_box = div();
                        // The folder gives way before the name does.
                        subtitle_box.style().flex_shrink = Some(1000.);
                        subtitle_box
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_start()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(subtitle)
                    }))
                    .child(div().flex_1().min_w(px(8.)))
                    .children(row.trailing.clone().map(|t| key_hint(t, p))),
            )
    }

    fn footer(&self) -> impl IntoElement {
        let p = &self.colors;
        let hint = |key: &'static str, label: &'static str| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .child(key_hint(key.to_string(), p))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child(label),
                )
        };
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .h(px(28.))
            .child(hint("↑↓", "navigate"))
            .child(hint("⏎", self.mode.enter_label()))
            .child(hint("esc", "close"))
            .child(div().flex_1().min_w(px(8.)))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(10.5))
                    .text_color(hsla(p.text_tertiary))
                    .child(
                        "> commands   : line   % text   b: branches   w: workspaces   h: history   ? more",
                    ),
            )
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().absolute().into_any_element();
        }
        let p = self.colors.clone();
        let hairline = || div().flex_none().h(px(1.)).bg(hsla(p.hairline));
        // Horizontally centered, its top edge a little below the titlebar.
        div()
            .absolute()
            .top(px(metrics::TITLEBAR_HEIGHT + 56.))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("palette")
                    .key_context(CONTEXT)
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::select_previous))
                    .on_action(cx.listener(Self::confirm))
                    .on_action(cx.listener(Self::dismiss_action))
                    .on_mouse_down_out(
                        cx.listener(|palette, _, window, cx| palette.dismiss(false, window, cx)),
                    )
                    .occlude()
                    .flex()
                    .flex_col()
                    .w(px(WIDTH))
                    .bg(hsla(p.overlay))
                    .rounded(px(metrics::RADIUS_LARGE + 2.))
                    .border_1()
                    .border_color(hsla(p.hairline_strong))
                    .shadow_lg()
                    .overflow_hidden()
                    .font_family(crate::workbench::UI_FONT)
                    .child(self.field())
                    .child(hairline())
                    .child(self.results(cx))
                    .child(hairline())
                    .child(self.footer()),
            )
            .into_any_element()
    }
}

/// The Swift `KeyHint`: a shortcut or label in a hairline box.
fn key_hint(text: String, p: &ChromePalette) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(4.))
        .h(px(16.))
        .rounded(px(3.))
        .border_1()
        .border_color(hsla(p.hairline_strong))
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(hsla(p.text_tertiary))
        .child(text)
}

/// `text` with the fuzzy-matched characters (byte offsets) in the accent
/// color, semibold.
fn highlighted(text: &str, positions: &[usize], p: &ChromePalette) -> StyledText {
    let style = HighlightStyle {
        color: Some(hsla(p.accent)),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &position in positions {
        let Some(c) = text.get(position..).and_then(|rest| rest.chars().next()) else {
            continue;
        };
        let end = position + c.len_utf8();
        match ranges.last_mut() {
            Some(range) if range.end == position => range.end = end,
            _ => ranges.push(position..end),
        }
    }
    StyledText::new(text.to_string()).with_highlights(ranges.into_iter().map(|r| (r, style)))
}

/// A file's row: its name (with the matches that fall in it) and folder.
fn file_row(path: &Path, root: &str, positions: &[usize]) -> Row {
    let absolute = path.to_string_lossy().into_owned();
    let relative = relative_to(&absolute, root);
    let name = file_name(&relative);
    let dir = Path::new(&relative)
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Match positions in the relative path, moved onto the file name.
    let name_start = relative.len() - name.len();
    let highlights = positions
        .iter()
        .filter(|p| **p >= name_start)
        .map(|p| p - name_start)
        .collect();
    let path = path.to_path_buf();
    Row::new(
        format!("file:{absolute}"),
        Some("file"),
        name,
        move |w, window, cx| w.palette_open_file(&path, None, None, window, cx),
    )
    .highlights(highlights)
    .subtitle(Some(dir))
}

fn relative_to(path: &str, root: &str) -> String {
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|_| !root.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn display_root(root: &str) -> String {
    if root.is_empty() {
        "the project".into()
    } else {
        abbreviate_home(root)
    }
}

/// "42" or "42:7".
fn parse_line(term: &str) -> Option<(u32, Option<u32>)> {
    let mut parts = term.split(':');
    let line: u32 = parts.next()?.parse().ok().filter(|l| *l > 0)?;
    let column = parts.next().and_then(|c| c.parse().ok());
    Some((line, column))
}

/// "origin/feature" → "feature" (git switch creates a tracking branch).
fn local_name(name: &str) -> &str {
    name.split_once('/').map_or(name, |(_, rest)| rest)
}

fn symbol_icon(kind: i64) -> &'static str {
    match kind {
        // class, enum, interface, struct, type parameter
        5 | 10 | 11 | 23 | 26 => "layers",
        // method, constructor, function, operator
        6 | 9 | 12 | 25 => "code",
        // module, namespace, package
        2..=4 => "package",
        _ => "tag",
    }
}

// MARK: Recent commands

/// Where the recently run commands are kept (the Swift app keeps them in
/// its defaults; here, a file in the data directory).
fn recents_path() -> PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("palette-recent-commands.json")
}

fn load_recents() -> RecentCommandStore {
    std::fs::read(recents_path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn record_recent(command: &AppCommand) {
    let mut store = load_recents();
    let item = CommandPaletteItem {
        id: command.id.clone(),
        title: command.title.clone(),
        category: command.category.clone(),
        keywords: command.keywords.clone(),
        source: "builtin".into(),
        shortcut: None,
        payload: None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    store.record(&item, now, MAX_RECENTS);
    if let Ok(json) = serde_json::to_vec_pretty(&store) {
        let _ = std::fs::write(recents_path(), json);
    }
}

// MARK: The window's side (the Swift `PaletteHost`)

impl Workbench {
    /// Open the palette for command `id` (see `prefix_for`).
    pub(crate) fn show_palette(
        &mut self,
        prefix: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Go to Line is the line mode, for an editor.
        if prefix == ":" && self.active_editor().is_none() {
            return;
        }
        let host = self.palette_host(cx);
        let colors = self.palette.clone();
        let prefix = prefix.to_string();
        self.command_palette.update(cx, |palette, cx| {
            palette.show(&prefix, host, colors, window, cx)
        });
    }

    fn palette_host(&self, cx: &App) -> Host {
        let root = self.tree_root(cx).to_string_lossy().into_owned();
        let mut host = Host {
            trusted: self.trust.is_trusted(&root),
            root,
            branch: Some(self.workspace().branch.clone()).filter(|b| !b.is_empty()),
            editor: self.active_editor().cloned(),
            commands: crate::commands::catalog(SettingsStore::get(cx)),
            settings: SettingsStore::get(cx).clone(),
            ..Default::default()
        };
        for (w, workspace) in self.workspaces.iter().enumerate() {
            host.workspaces.push(WorkspaceInfo {
                id: workspace.id,
                name: workspace.name(),
                root: abbreviate_home(&workspace.root.to_string_lossy()),
                is_scratch: workspace.kind == Kind::Scratch,
                tab_count: workspace.tabs.len(),
                is_active: w == self.active,
            });
            for (t, tab) in workspace.tabs.iter().enumerate() {
                let surface = tab.focused_surface();
                host.tabs.push(TabInfo {
                    id: tab.id,
                    title: surface.title(cx),
                    is_terminal: matches!(surface, Surface::Terminal(_)),
                    workspace_name: workspace.name(),
                    directory: surface
                        .directory(cx)
                        .map(|d| abbreviate_home(&d.to_string_lossy())),
                    position: (w == self.active).then_some(t),
                });
                for (_, surface) in tab.surfaces() {
                    if let Surface::Editor(editor) = surface {
                        host.open_files.push(editor.read(cx).path.clone());
                    }
                }
            }
        }
        host
    }

    /// Open `path`, at a 1-based line and column when given.
    fn palette_open_file(
        &mut self,
        path: &Path,
        line: Option<u32>,
        column: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file(path, window, cx);
        if let Some(line) = line {
            self.palette_go_to(line, column, window, cx);
        }
    }

    /// Put the focused editor's cursor at a 1-based line and column.
    fn palette_go_to(
        &mut self,
        line: u32,
        column: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor().cloned() else {
            return;
        };
        let position = Position::new(
            line.saturating_sub(1),
            column.unwrap_or(1).saturating_sub(1),
        );
        editor.update(cx, |editor, cx| {
            editor.go_to(position, window, cx);
            editor.focus(window, cx);
        });
    }

    fn palette_select_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        for w in 0..self.workspaces.len() {
            if let Some(t) = self.workspaces[w].tabs.iter().position(|t| t.id == id) {
                if w != self.active {
                    self.activate_workspace(w, window, cx);
                }
                self.activate_tab(t, window, cx);
                return;
            }
        }
    }

    fn palette_select_workspace(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.workspaces.iter().position(|w| w.id == id) {
            self.activate_workspace(index, window, cx);
        }
    }

    /// Switch the tree's repository to `name` (or create it and switch),
    /// saying in the status bar when git refuses.
    fn palette_switch_branch(
        &mut self,
        name: String,
        create: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.tree_root(cx).to_string_lossy().into_owned();
        let switching = {
            let name = name.clone();
            cx.background_executor().spawn(async move {
                if create {
                    impulse_git::operations::create_branch(&name, None, true, &root)
                } else {
                    impulse_git::operations::switch_branch(&name, &root)
                }
            })
        };
        cx.spawn(async move |workbench, cx| {
            let result = switching.await;
            let _ = workbench.update(cx, |workbench, cx| {
                for workspace in &mut workbench.workspaces {
                    workspace.refresh_branch();
                }
                match result {
                    Ok(()) if create => {
                        workbench.set_notice(format!("Created and switched to {name}"), cx)
                    }
                    Ok(()) => cx.notify(),
                    Err(error) if create => {
                        workbench.set_notice(format!("Couldn't create {name}: {error}"), cx)
                    }
                    Err(error) => {
                        workbench.set_notice(format!("Couldn't switch to {name}: {error}"), cx)
                    }
                }
            });
        })
        .detach();
    }

    /// The Settings tab isn't in the Rust app yet: open settings.json at
    /// the setting's line when it's there.
    /// A setting picked in `set:`: the Settings tab, searched for it.
    fn palette_open_setting(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(Some(key), window, cx);
    }
}
