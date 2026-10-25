//! The window: its workspaces, their tabs and panes, and every command
//! (`RunCommand`) run against them, as `MainWindowController` and
//! `TabManager` do in the Swift app. Drawing is in `workbench_chrome`
//! (titlebar, sidebar, status bar) and `workbench_panes` (a tab's split
//! panes); trust and language servers in `workbench_trust`.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    actions, div, prelude::*, px, App, Context, Entity, FocusHandle, KeyBinding, Window,
};
use impulse_kit::chrome::ChromePalette;
use impulse_kit::close_risk::CloseRiskAction;
use impulse_kit::layout_tree::{PaneDirection, SplitAxis};
use impulse_kit::theme::ResolvedTheme;
use impulse_kit::trust::WorkspaceTrust;

use crate::commands::RunCommand;
use crate::editor_tab::{EditorTab, EditorTabChanged};
use crate::settings_store::SettingsStore;
use crate::terminal_view::{TerminalView, TerminalViewEvent};
use crate::workspace::{Kind, PaneId, Surface, Tab, Workspace};
use crate::Args;

pub(crate) const UI_FONT: &str = "Inter";
/// Lines of chrome text, in font sizes (Inter's ascent plus descent, which
/// is what SwiftUI lays text out with).
pub(crate) const UI_LINE_HEIGHT: f32 = 1.22;

actions!(
    impulse,
    [FormatDocument, RenameSymbol, FindReferences, UndoClose]
);

/// Monaco's keys for the editor commands that go through the workbench
/// (they reach other files).
pub fn bind_keys(cx: &mut App) {
    let c = Some(crate::editor_keys::CONTEXT);
    cx.bind_keys([
        KeyBinding::new("alt-shift-f", FormatDocument, c),
        KeyBinding::new("f2", RenameSymbol, c),
        KeyBinding::new("shift-f12", FindReferences, c),
    ]);
    // ⌘Z brings back what just closed when nothing focused undoes text
    // (the editor's and input bar's own bindings are deeper), as the Swift
    // window's undo manager does. Linux's Ctrl+Z belongs to the shell.
    if cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new("cmd-z", UndoClose, Some("Workbench"))]);
    }
    crate::changes_panel::bind_keys(cx);
}

/// How long a close can be undone from its toast or with ⌘Z (Swift's).
const UNDO_CLOSE_FOR: std::time::Duration = std::time::Duration::from_secs(10);

/// Something closed that Reopen Closed Tab (or the close's Undo) brings
/// back as it was: a tab, its layout and its terminals' scrollback, back in
/// its workspace; or a whole workspace, in its place.
enum Closed {
    Tab {
        workspace: u64,
        tab: impulse_kit::session::SessionTab,
        /// When a single pane closed: the tab it was in, so it can rejoin it.
        from_tab: Option<u64>,
    },
    Workspace {
        /// Its id when it closed (tabs closed from it before point there).
        id: u64,
        position: usize,
        saved: impulse_kit::session::SessionWorkspace,
    },
}

/// How many closed items are remembered.
const MAX_CLOSED: usize = 20;

/// What the sidebar shows below the workspaces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarPanel {
    Files,
    Changes,
    Search,
}

pub struct Workbench {
    pub(crate) search: Entity<crate::search_panel::SearchPanel>,
    pub(crate) palette: ChromePalette,
    pub(crate) theme: ResolvedTheme,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) active: usize,
    pub(crate) next_id: u64,
    pub(crate) focus: FocusHandle,
    /// A message for the status bar, such as why a file didn't open.
    pub(crate) notice: Option<String>,
    /// The trusted folders (shared by every window).
    pub(crate) trust: Arc<WorkspaceTrust>,
    pub(crate) panel: SidebarPanel,
    /// The file tree's keyboard focus, its selected row (the open file's
    /// until the keys move it) and its scrolling.
    pub(crate) tree_focus: gpui_kit::FocusHandle,
    pub(crate) tree_selection: Option<std::path::PathBuf>,
    /// The lsp.json files' stamp last seen (`start_lsp_config_watch`).
    pub(crate) lsp_config_stamp: Option<String>,
    /// The last close and when, for ⌘Z (`UndoClose`) within
    /// `UNDO_CLOSE_FOR`.
    undo_close: Option<(u64, std::time::Instant)>,
    /// The settings.json warning dismissed (`SettingsStore::warning`'s count).
    pub(crate) dismissed_settings_warning: Option<u64>,
    pub(crate) tree_scroll: gpui_kit::UniformListScrollHandle,
    /// This window's sidebar, shown or not, and its width (Swift keeps them
    /// per window; the settings give new windows theirs, and a change there
    /// applies to every window).
    pub(crate) sidebar_shown: bool,
    pub(crate) sidebar_px: f32,
    sidebar_seen: (bool, i64),
    /// Workspace ids, the most recently shown last: closing one goes back to
    /// the one used before it.
    pub(crate) recent_workspaces: Vec<u64>,
    /// The task states' refresh waiting for changes to settle
    /// (`Workbench::schedule_task_refresh`).
    pub(crate) task_refresh: Option<gpui_kit::Task<()>>,
    /// Closed tabs and workspaces, newest last, each with its id.
    closed: Vec<(u64, Closed)>,
    /// The last close recorded: its id and what to call it (for the Undo
    /// toast of a close the user made).
    last_closed: Option<(u64, String)>,
    /// Language servers' work going on (indexing): `server:token`, newest
    /// last.
    pub(crate) lsp_progress: Vec<(String, crate::workbench_trust::LspProgress)>,
    /// Language-server problems already told (each is said once).
    pub(crate) told_lsp: std::collections::HashSet<String>,
    pub(crate) drag: Option<crate::workbench_panes::DividerDrag>,
    /// The sidebar's edge being dragged (`workbench_chrome`).
    pub(crate) sidebar_drag: Option<crate::workbench_chrome::SidebarDrag>,
    /// The Workspaces section's height once its handle moved (`None`: fit
    /// its rows up to 280), saved with the session; the drag in progress
    /// (height and pointer at its start); the height last laid out.
    pub(crate) workspaces_height: Option<f32>,
    pub(crate) workspaces_drag: Option<(f32, f32)>,
    pub(crate) workspaces_measured: Rc<std::cell::Cell<f32>>,
    pub(crate) split_bounds: crate::workbench_panes::SplitBounds,
    /// `impulse edit` requests waiting for their editor to close.
    pub(crate) pending_edits: Vec<(
        gpui_kit::EntityId,
        async_channel::Sender<impulse_protocol::control::ControlResponse>,
    )>,
    /// `--theme`, which wins over `color_scheme` for this run.
    theme_override: Option<String>,
    theme_id: String,
    /// The Increase Contrast the palette was made with.
    contrast: bool,
    /// The command palette and quick open (`palette.rs`).
    pub(crate) command_palette: Entity<crate::palette::Palette>,
    _settings: gpui_kit::Subscription,
    _contrast: gpui_kit::Subscription,
    /// The window's bounds, for the session (`session::window_frame`).
    pub(crate) bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    _bounds: gpui_kit::Subscription,
    /// Repositories and the Changes panel (`changes_panel`, `git_actions`).
    pub(crate) git: crate::changes_panel::GitPanel,
    /// Every language server's diagnostics (`problems_view`).
    pub(crate) problems: Entity<crate::problems_view::ProblemStore>,
    /// The folders of open files, watched (`editor_watch`).
    pub(crate) editor_watch: crate::editor_watch::EditorWatch,
    /// Closing the window was asked about and confirmed (`close_flow`).
    pub(crate) close_confirmed: bool,
    /// The overlapping workspace pairs seen, by repository (so each new one
    /// is told once).
    pub(crate) known_overlaps: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// The folders the file tree has listed, watched (`tree_watch`).
    pub(crate) tree_watch: crate::tree_watch::TreeWatch,
    /// Folders whose files were said to have language servers off.
    pub(crate) restricted_noted: std::collections::HashSet<String>,
}

impl Workbench {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = args.session.clone();
        let mut workbench = Self::empty(&args, window, cx);
        crate::session::register(cx.entity().downgrade(), window.window_handle(), cx);
        if let Some(state) = restore {
            workbench.restore(state, window, cx);
        }
        if workbench.workspaces.is_empty() {
            workbench.open_default(&args, window, cx);
        }
        let cwd = crate::workspace::normalize(Path::new(&args.cwd));
        for path in &args.open {
            let path = cwd.join(path);
            workbench.open_file(&path, window, cx);
        }
        // Once the window (and its dialog layer) exists: language servers if
        // the folder is trusted, otherwise ask.
        cx.defer_in(window, |workbench, window, cx| {
            workbench.request_trust(workbench.active, window, cx);
            let theme = workbench.theme_id.clone();
            workbench.report_theme_problem(&theme, window, cx);
        });
        workbench.start_task_monitor(window, cx);
        workbench.start_port_scan(window, cx);
        workbench.start_spinner(window, cx);
        workbench.start_editor_watch(window, cx);
        workbench.start_tree_watch(window, cx);
        workbench.start_lsp_config_watch(window, cx);
        // Workspaces list their folders before settings are at hand.
        if SettingsStore::get(cx).sidebar_show_hidden {
            workbench.reload_tree(cx);
        }
        workbench.guard_window_close(window, cx);
        workbench
    }

    /// A window with no workspaces yet.
    fn empty(args: &Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = SettingsStore::get(cx).clone();
        crate::workspace::SCRATCH.with(|s| *s.borrow_mut() = settings.scratch_directory.clone());
        let theme_id = args
            .theme
            .clone()
            .unwrap_or_else(|| settings.color_scheme.clone());
        let theme = impulse_kit::theme::get_theme(&theme_id);
        let contrast = crate::display_options::increase_contrast(cx);
        let palette = ChromePalette::new(&theme, contrast);
        crate::kit_theme::apply(&theme, &palette, &settings, cx);
        let settings_subscription = cx
            .observe_global_in::<SettingsStore>(window, |workbench, window, cx| {
                workbench.apply_settings(window, cx)
            });
        let bounds_subscription = cx.observe_window_bounds(window, |workbench, window, _| {
            workbench.bounds = window.bounds()
        });
        let contrast_subscription = cx
            .observe_global_in::<crate::display_options::IncreaseContrast>(
                window,
                |workbench, window, cx| workbench.apply_settings(window, cx),
            );
        let search =
            cx.new(|cx| crate::search_panel::SearchPanel::new(palette.clone(), window, cx));
        cx.subscribe_in(&search, window, |workbench, panel, event, window, cx| {
            workbench.search_event(panel.clone(), event, window, cx)
        })
        .detach();
        Self {
            search,
            palette: palette.clone(),
            theme,
            workspaces: Vec::new(),
            active: 0,
            next_id: 1,
            focus: cx.focus_handle(),
            notice: None,
            trust: args.trust_store.clone(),
            panel: SidebarPanel::Files,
            tree_focus: cx.focus_handle(),
            tree_selection: None,
            tree_scroll: gpui_kit::UniformListScrollHandle::new(),
            task_refresh: None,
            sidebar_shown: settings.sidebar_visible,
            sidebar_px: settings.sidebar_width as f32,
            sidebar_seen: (settings.sidebar_visible, settings.sidebar_width),
            recent_workspaces: Vec::new(),
            dismissed_settings_warning: None,
            undo_close: None,
            lsp_config_stamp: None,
            closed: Vec::new(),
            last_closed: None,
            lsp_progress: Vec::new(),
            told_lsp: Default::default(),
            drag: None,
            sidebar_drag: None,
            workspaces_height: None,
            workspaces_drag: None,
            workspaces_measured: Default::default(),
            split_bounds: Default::default(),
            pending_edits: Vec::new(),
            theme_override: args.theme.clone(),
            theme_id,
            contrast,
            command_palette: {
                let (workbench, colors) = (cx.weak_entity(), palette.clone());
                cx.new(|cx| crate::palette::Palette::new(workbench, colors, window, cx))
            },
            _settings: settings_subscription,
            _contrast: contrast_subscription,
            bounds: window.bounds(),
            _bounds: bounds_subscription,
            git: crate::changes_panel::GitPanel::new(window, cx),
            problems: cx.new(|_| Default::default()),
            editor_watch: crate::editor_watch::EditorWatch::new(),
            close_confirmed: false,
            restricted_noted: Default::default(),
            tree_watch: crate::tree_watch::TreeWatch::new(),
            known_overlaps: Default::default(),
        }
    }

    /// The folder Impulse started in as a workspace (unless that's home),
    /// and Scratch, with a terminal.
    fn open_default(&mut self, args: &Args, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = crate::workspace::normalize(Path::new(&args.cwd));
        if args.trust {
            self.trust.trust(&cwd.to_string_lossy());
        }
        let home = crate::workspace::home();
        if cwd != home {
            impulse_kit::recent_workspaces::note(
                &crate::workbench_workspaces::recents_file(),
                &cwd.to_string_lossy(),
            );
            let id = self.next_id();
            self.workspaces
                .push(Workspace::new(id, Kind::Folder, cwd.clone()));
        }
        let id = self.next_id();
        self.workspaces
            .push(Workspace::new(id, Kind::Scratch, home));
        let dir = self.workspaces[0].default_directory();
        let terminal = self.new_terminal(dir, args.commands.clone(), window, cx);
        self.add_tab(0, terminal, window, cx);
    }

    /// What a launch asked for, on top of a restored session: `cwd`'s
    /// workspace (added if it's new), `commands` in a new terminal there,
    /// and `open`'s files (relative to `cwd`, else the process's folder).
    pub(crate) fn apply_launch(
        &mut self,
        cwd: Option<String>,
        commands: Vec<String>,
        open: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base = cwd
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .map(|p| crate::workspace::normalize(&p));
        if let Some(folder) = cwd.as_deref() {
            let folder = crate::workspace::normalize(Path::new(folder));
            if folder != crate::workspace::home() {
                self.open_workspace(&folder, window, cx);
            }
        }
        if !commands.is_empty() {
            let dir = self.workspace().default_directory();
            let terminal = self.new_terminal(dir, commands, window, cx);
            let active = self.active;
            self.add_tab(active, terminal, window, cx);
        }
        for path in open {
            let path = match &base {
                Some(base) => base.join(path),
                None => PathBuf::from(path),
            };
            self.open_file(&path, window, cx);
        }
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn workspace(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub(crate) fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    /// The focused surface of the active tab.
    pub(crate) fn focused_surface(&self) -> Option<&Surface> {
        self.workspace().active_tab().map(Tab::focused_surface)
    }

    pub(crate) fn active_editor(&self) -> Option<&Entity<EditorTab>> {
        self.focused_surface().and_then(Surface::editor)
    }

    pub(crate) fn active_terminal(&self) -> Option<&Entity<TerminalView>> {
        self.focused_surface().and_then(Surface::terminal)
    }

    /// A theme file that doesn't load falls back to a built-in theme: say
    /// which file and why rather than switching colors without a word, as
    /// the Swift `reportThemeProblem` does.
    fn report_theme_problem(
        &mut self,
        theme_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, message)) = impulse_kit::theme::load_problem(theme_id) else {
            return;
        };
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let workbench = cx.entity().downgrade();
        let open: crate::tasks::Run = Rc::new(move |window, cx| {
            let _ = workbench.update(cx, |w, cx| w.open_file(&path, window, cx));
        });
        crate::toasts::show(
            window,
            cx,
            crate::toasts::Toast::warning(message)
                .title(format!(
                    "Couldn't load the theme {file}; using {} instead",
                    impulse_kit::theme::get_theme(theme_id).name
                ))
                .action("Open File", open)
                .lifetime(Some(std::time::Duration::from_secs(15))),
        );
    }

    /// Settings changed: the theme, the editor's and terminals' fonts.
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsStore::get(cx).clone();
        let sidebar = (settings.sidebar_visible, settings.sidebar_width);
        if sidebar != self.sidebar_seen {
            self.sidebar_seen = sidebar;
            self.sidebar_shown = sidebar.0;
            self.sidebar_px = sidebar.1 as f32;
        }
        crate::workspace::SCRATCH.with(|s| *s.borrow_mut() = settings.scratch_directory.clone());
        let theme_id = self
            .theme_override
            .clone()
            .unwrap_or_else(|| settings.color_scheme.clone());
        let contrast = crate::display_options::increase_contrast(cx);
        if theme_id != self.theme_id || contrast != self.contrast {
            if theme_id != self.theme_id && window.is_window_active() {
                self.report_theme_problem(&theme_id, window, cx);
            }
            self.theme_id = theme_id.clone();
            self.contrast = contrast;
            self.theme = impulse_kit::theme::get_theme(&theme_id);
            self.palette = ChromePalette::new(&self.theme, contrast);
            let palette = self.palette.clone();
            self.search
                .update(cx, |search, cx| search.set_palette(palette, cx));
        }
        crate::kit_theme::apply(&self.theme, &self.palette, &settings, cx);
        let surfaces: Vec<Surface> = self
            .workspaces
            .iter()
            .flat_map(|w| w.all_surfaces().cloned().collect::<Vec<_>>())
            .collect();
        for surface in surfaces {
            match surface {
                Surface::Editor(editor) => editor.update(cx, |editor, cx| {
                    editor.apply_settings(&settings, window, cx)
                }),
                Surface::Terminal(terminal) => {
                    let (theme, palette) = (self.theme.clone(), self.palette.clone());
                    terminal.update(cx, |t, cx| t.set_theme(theme, palette, window, cx))
                }
                Surface::Review(review) => {
                    let (theme, palette) = (self.theme.clone(), self.palette.clone());
                    review.update(cx, |r, cx| r.set_theme(&theme, &palette, cx))
                }
                Surface::History(history) => {
                    let (theme, palette) = (self.theme.clone(), self.palette.clone());
                    history.update(cx, |h, cx| h.set_theme(&theme, &palette, cx))
                }
                Surface::Image(image) => {
                    let background = crate::workbench_chrome::hsla(
                        impulse_kit::chrome::Rgba::from_hex(&self.theme.bg),
                    );
                    image.update(cx, |i, cx| i.set_background(background, cx))
                }
                Surface::Settings(view) => view.update(cx, |view, cx| {
                    view.palette = self.palette.clone();
                    cx.notify();
                }),
                Surface::Keybindings(view) => view.update(cx, |view, cx| {
                    view.palette = self.palette.clone();
                    cx.notify();
                }),
                Surface::TaskFinish(view) => view.update(cx, |view, cx| {
                    view.palette = self.palette.clone();
                    cx.notify();
                }),
                Surface::Problems(view) => view.update(cx, |view, cx| {
                    view.palette = self.palette.clone();
                    cx.notify();
                }),
                Surface::ProjectSetup(view) => view.update(cx, |view, cx| {
                    view.palette = self.palette.clone();
                    cx.notify();
                }),
            }
        }
        cx.notify();
    }

    // MARK: Surfaces and tabs

    /// A terminal starting in `dir`, running `commands` first.
    pub(crate) fn new_terminal(
        &mut self,
        dir: PathBuf,
        commands: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Surface {
        self.new_terminal_restoring(dir, commands, None, window, cx)
    }

    /// A terminal that first shows a previous session's output.
    pub(crate) fn new_terminal_restoring(
        &mut self,
        dir: PathBuf,
        commands: Vec<String>,
        transcript: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Surface {
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let terminal = cx.new(|cx| {
            TerminalView::restoring(
                theme,
                palette,
                dir.to_string_lossy().into_owned(),
                commands,
                transcript,
                window,
                cx,
            )
        });
        // A terminal on screen redraws the window itself (GPUI marks its
        // ancestors dirty); one in a hidden tab or workspace only when what
        // the chrome shows of it changed.
        let mut shown_key = None;
        cx.observe(&terminal, move |_, terminal, cx| {
            let key = terminal.read(cx).chrome_key();
            if shown_key != Some(key) {
                shown_key = Some(key);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe_in(
            &terminal,
            window,
            |workbench, terminal, event, window, cx| match event {
                TerminalViewEvent::Exited => {
                    // The shell exited: close its pane, as Terminal.app does.
                    let surface = Surface::Terminal(terminal.clone());
                    workbench.close_surface(&surface, window, cx);
                }
                TerminalViewEvent::OpenFile { path, line, column } => {
                    workbench.open_file(path, window, cx);
                    if let (Some(line), Some(editor)) = (line, workbench.active_editor().cloned()) {
                        let position = gpui_kit::component::input::Position::new(
                            line.saturating_sub(1) as u32,
                            column.unwrap_or(1).saturating_sub(1) as u32,
                        );
                        editor.update(cx, |e, cx| e.go_to(position, window, cx));
                    }
                }
                TerminalViewEvent::RunCommand(id) => workbench.run_command(id, window, cx),
                TerminalViewEvent::OpenCommit { sha, cwd } => {
                    workbench.open_commit(sha.clone(), cwd, window, cx)
                }
                TerminalViewEvent::Notice(notice) => workbench.set_notice(notice.clone(), cx),
                // Where the next launch starts (Swift's lastDirectory): the
                // terminal in front, not one cd-ing in the background.
                TerminalViewEvent::DirectoryChanged(dir) => {
                    let in_front = matches!(
                        workbench.focused_surface(),
                        Some(Surface::Terminal(t)) if t == terminal
                    );
                    if in_front && SettingsStore::get(cx).last_directory != *dir {
                        let dir = dir.clone();
                        SettingsStore::update(cx, |s| s.last_directory = dir);
                    }
                }
                _ => {}
            },
        )
        .detach();
        self.watch_agent(&terminal, window, cx);
        Surface::Terminal(terminal)
    }

    /// Put `surface` in a new tab of workspace `index` and show it.
    pub(crate) fn add_tab(
        &mut self,
        index: usize,
        surface: Surface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (tab_id, pane) = (self.next_id(), self.next_id());
        let workspace = &mut self.workspaces[index];
        // After the active tab, as the Swift app inserts them; never among
        // the pinned ones unless it's the last of them.
        let at = if workspace.tabs.is_empty() {
            0
        } else if workspace.tabs[workspace.active].pinned {
            workspace
                .tabs
                .iter()
                .rposition(|t| t.pinned)
                .unwrap_or(workspace.active)
                + 1
        } else {
            workspace.active + 1
        };
        let opener = workspace.tabs.get(workspace.active).map(|t| t.id);
        let mut tab = Tab::new(tab_id, pane, surface);
        tab.opener = opener;
        workspace.tabs.insert(at, tab);
        workspace.active = at;
        self.active = index;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Give the focused pane of the active tab the keyboard.
    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(surface) = self.focused_surface() {
            surface.focus(window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    pub(crate) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace_mut();
        if workspace.tabs.is_empty() {
            return;
        }
        workspace.active = index.min(workspace.tabs.len() - 1);
        self.notice = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Show workspace `index`; one with no tabs gets a terminal.
    pub(crate) fn activate_workspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.workspaces.len() {
            return;
        }
        self.show_workspace(index, window, cx);
        self.request_trust(index, window, cx);
    }

    /// `activate_workspace` without asking about trust: while the window
    /// is being made (a restored session) a dialog can't open yet, and
    /// `Workbench::new` asks once it can.
    pub(crate) fn show_workspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.workspaces.len() {
            return;
        }
        self.active = index;
        let id = self.workspaces[index].id;
        self.recent_workspaces.retain(|w| *w != id);
        self.recent_workspaces.push(id);
        // The branch comes from the repository's state (`sync_branches`),
        // not a git call on the main thread.
        self.sync_branches(cx);
        if self.workspaces[index].tabs.is_empty() {
            let dir = self.workspaces[index].default_directory();
            let terminal = self.new_terminal(dir, Vec::new(), window, cx);
            self.add_tab(index, terminal, window, cx);
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Open `path` in an editor tab, or go to the tab it's already in.
    pub(crate) fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_as(path, false, window, cx);
    }

    /// Open `path` from a single click in the file tree: in the workspace's
    /// preview tab when `editor_preview_tabs` is on.
    pub(crate) fn preview_file(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preview = SettingsStore::get(cx).editor_preview_tabs;
        self.open_file_as(path, preview, window, cx);
    }

    pub(crate) fn open_file_as(
        &mut self,
        path: &Path,
        preview: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((w, t, pane)) = self.find_editor(path, cx) {
            self.active = w;
            self.workspaces[w].active = t;
            let tab = &mut self.workspaces[w].tabs[t];
            tab.focused = pane;
            // Opening it for real keeps a preview.
            if !preview {
                tab.preview = false;
            }
            self.focus_active(window, cx);
            cx.notify();
            return;
        }
        if crate::image_view::is_image_file(path) {
            let surface = self.make_image(path, cx);
            let index = self.active;
            self.add_tab(index, surface, window, cx);
            return;
        }
        match impulse_kit::text_file::open(path)
            .map(|opened| cx.new(|cx| EditorTab::new(path, opened, window, cx)))
        {
            Ok(editor) => {
                let index = self.active;
                self.start_lsp(index, &editor, window, cx);
                self.wire_editor_git(&editor, window, cx);
                cx.subscribe(&editor, |w, editor, _: &EditorTabChanged, cx| {
                    // An edit keeps a preview tab.
                    if editor.read(cx).dirty {
                        w.keep_preview_showing(&editor);
                    }
                    cx.notify()
                })
                .detach();
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                self.notice = None;
                if preview && self.replace_preview(Surface::Editor(editor.clone()), window, cx) {
                    return;
                }
                self.add_tab(index, Surface::Editor(editor), window, cx);
                if preview {
                    let workspace = self.workspace_mut();
                    let active = workspace.active;
                    workspace.tabs[active].preview = true;
                }
            }
            Err(refusal) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                use impulse_kit::text_file::OpenRefusal;
                if matches!(refusal, OpenRefusal::Binary | OpenRefusal::NotUtf8) {
                    // The system's app can show it; one that would run is
                    // shown in the file manager instead, never a click away.
                    let file = path.to_path_buf();
                    let runs = impulse_kit::text_file::opens_as_program(&file);
                    let open: crate::tasks::Run = Rc::new(move |_, cx| {
                        if runs {
                            crate::file_tree_actions::reveal(&file, cx)
                        } else {
                            cx.open_with_system(&file)
                        }
                    });
                    return crate::toasts::show(
                        window,
                        cx,
                        crate::toasts::Toast::info(refusal.message(&name))
                            .action(
                                if runs {
                                    crate::file_tree_actions::REVEAL
                                } else {
                                    "Open in Default App"
                                },
                                open,
                            )
                            .lifetime(Some(std::time::Duration::from_secs(10))),
                    );
                }
                if refusal != OpenRefusal::TooLarge {
                    return crate::tasks::toast(
                        window,
                        cx,
                        crate::tasks::ToastKind::Warning,
                        refusal.message(&name),
                        None,
                        None,
                    );
                }
                self.offer_pager(path, &name, window, cx);
            }
        }
    }

    /// A file too large for the editor: offer to page through it in a
    /// terminal instead (Swift's `showTooLargeToEdit`).
    pub(crate) fn offer_pager(
        &mut self,
        path: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (dir, file) = (
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
            path.to_path_buf(),
        );
        let workbench = cx.entity().downgrade();
        let view: crate::tasks::Run = Rc::new(move |window, cx| {
            let (dir, file) = (dir.clone(), file.clone());
            let _ = workbench.update(cx, |w, cx| {
                let shell = impulse_kit::shell::login::default_shell_type();
                let command = format!(
                    "less -- {}",
                    impulse_kit::shell::quoting::quoted(&file.to_string_lossy(), shell)
                );
                let terminal = w.new_terminal(dir, vec![command], window, cx);
                let active = w.active;
                w.add_tab(active, terminal, window, cx);
            });
        });
        crate::tasks::toast(
            window,
            cx,
            crate::tasks::ToastKind::Info,
            format!("{name} is too large for the editor (over 10 MB)."),
            None,
            Some(("View in Terminal", view)),
        );
    }

    /// Show `surface` in the active workspace's preview tab instead of what
    /// it shows, when there's one to replace: one clean editor, not split.
    fn replace_preview(
        &mut self,
        surface: Surface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self.workspace().tabs.iter().position(|t| t.preview) else {
            return false;
        };
        let tab = &self.workspace().tabs[index];
        let old = tab.focused_surface().clone();
        if tab.is_split() || !matches!(old, Surface::Editor(_)) || old.is_dirty(cx) {
            self.workspace_mut().tabs[index].preview = false;
            return false;
        }
        self.editor_closed(&old);
        let pane = self.next_id();
        let tab = &mut self.workspace_mut().tabs[index];
        tab.layout = impulse_kit::layout_tree::LayoutTree::Leaf(pane);
        tab.panes = std::collections::HashMap::from([(pane, surface)]);
        tab.focused = pane;
        tab.zoomed = None;
        self.workspace_mut().active = index;
        self.focus_active(window, cx);
        cx.notify();
        true
    }

    /// Keep the preview tab showing `editor` (it was edited).
    fn keep_preview_showing(&mut self, editor: &Entity<EditorTab>) {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                if tab.preview
                    && tab
                        .panes
                        .values()
                        .any(|s| matches!(s, Surface::Editor(e) if e == editor))
                {
                    tab.preview = false;
                }
            }
        }
    }

    /// Turn the preview tab at `index` into a normal one (double-click).
    pub(crate) fn keep_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.workspace_mut().tabs.get_mut(index) {
            tab.preview = false;
            cx.notify();
        }
    }

    /// Pin or unpin tab `index` of workspace `ws`.
    pub(crate) fn toggle_pin(&mut self, ws: usize, index: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .workspaces
            .get_mut(ws)
            .and_then(|w| w.tabs.get_mut(index))
        {
            tab.preview = false;
            tab.pinned = !tab.pinned;
            cx.notify();
        }
    }

    /// Move the tab at `from` to `to` in the active workspace, keeping the
    /// same tab active.
    pub(crate) fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let workspace = self.workspace_mut();
        if from == to || from >= workspace.tabs.len() || to >= workspace.tabs.len() {
            return;
        }
        let active = workspace.tabs[workspace.active].id;
        let tab = workspace.tabs.remove(from);
        workspace.tabs.insert(to, tab);
        workspace.active = workspace
            .tabs
            .iter()
            .position(|t| t.id == active)
            .unwrap_or(0);
        cx.notify();
    }

    /// Move every pane of the tab at `index` into the active tab, beside its
    /// focused pane (right, or below).
    pub(crate) fn join_tab(
        &mut self,
        index: usize,
        below: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace();
        if index == workspace.active || index >= workspace.tabs.len() {
            return;
        }
        let target = workspace.tabs[workspace.active].id;
        let source = self.workspace_mut().tabs.remove(index);
        let workspace = self.workspace_mut();
        workspace.active = workspace
            .tabs
            .iter()
            .position(|t| t.id == target)
            .unwrap_or(0);
        let axis = if below {
            impulse_kit::layout_tree::SplitAxis::Vertical
        } else {
            impulse_kit::layout_tree::SplitAxis::Horizontal
        };
        for (_, surface) in source.surfaces() {
            let pane = self.next_id();
            if let Some(tab) = self.workspace_mut().active_tab_mut() {
                tab.split(pane, surface.clone(), axis, false);
            }
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// An image tab's surface.
    pub(crate) fn make_image(&mut self, path: &Path, cx: &mut Context<Self>) -> Surface {
        let background =
            crate::workbench_chrome::hsla(impulse_kit::chrome::Rgba::from_hex(&self.theme.bg));
        Surface::Image(
            cx.new(|cx| crate::image_view::ImageView::new(path.to_path_buf(), background, cx)),
        )
    }

    /// The (workspace, tab, pane) showing `path` in an editor.
    pub(crate) fn find_editor(&self, path: &Path, cx: &App) -> Option<(usize, usize, PaneId)> {
        let wanted = impulse_lsp::workspace_edit::resolved_path(path);
        for (w, workspace) in self.workspaces.iter().enumerate() {
            for (t, tab) in workspace.tabs.iter().enumerate() {
                for (pane, surface) in tab.surfaces() {
                    if let Surface::Editor(editor) = surface {
                        if impulse_lsp::workspace_edit::resolved_path(&editor.read(cx).path)
                            == wanted
                        {
                            return Some((w, t, pane));
                        }
                    }
                }
            }
        }
        None
    }

    /// Close the focused pane (the tab with it when it's the only one),
    /// asking first about unsaved changes and running commands.
    pub(crate) fn close_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace().active_tab() else {
            return;
        };
        if !tab.is_split() {
            let index = self.workspace().active;
            return self.close_tab_at(index, window, cx);
        }
        let target = crate::close_flow::Target::Pane {
            tab: tab.id,
            pane: tab.focused,
        };
        self.confirm_close(
            target,
            CloseRiskAction::CloseTab,
            Default::default(),
            window,
            cx,
            Rc::new(move |w, window, cx| {
                let crate::close_flow::Target::Pane { tab, pane } = target else {
                    return;
                };
                let surface = w
                    .workspaces
                    .iter()
                    .flat_map(|ws| ws.tabs.iter())
                    .find(|t| t.id == tab)
                    .and_then(|t| t.panes.get(&pane).cloned());
                if let Some(surface) = surface {
                    w.close_surface(&surface, window, cx);
                    w.offer_undo_close(window, cx);
                }
            }),
        );
    }

    /// Close the tab at `index` of the active workspace, every pane in it,
    /// asking first about a pin, unsaved changes and running commands.
    pub(crate) fn close_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace().tabs.get(index) else {
            return;
        };
        let id = tab.id;
        self.confirm_close(
            crate::close_flow::Target::Tab(id),
            CloseRiskAction::CloseTab,
            Default::default(),
            window,
            cx,
            Rc::new(move |w, window, cx| {
                w.remove_tab(id, window, cx);
                w.offer_undo_close(window, cx);
            }),
        );
    }

    /// Take tab `id` out of the active workspace now.
    fn remove_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        // Re-find the tab: others may have closed or opened while asking.
        let Some(index) = self.workspace().tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let workspace = self.workspace().id;
        let was_active = self.workspace().active == index;
        let tab = self.workspace_mut().tabs.remove(index);
        let back_to = was_active.then_some(tab.opener).flatten();
        for surface in tab.panes.values() {
            self.editor_closed(surface);
        }
        let saved = crate::session::session_tab(&tab, true, cx);
        let title = tab.focused_surface().name(cx);
        self.remember(
            Closed::Tab {
                workspace,
                tab: saved,
                from_tab: None,
            },
            title,
        );
        self.after_tab_removed(index, back_to, window, cx);
    }

    /// Note something closed, unless nothing in it could come back (tools
    /// aren't saved with the session).
    fn remember(&mut self, item: Closed, title: String) {
        if let Closed::Tab { tab, .. } = &item {
            if tab.panes.iter().all(|p| p.kind == "tool") {
                return;
            }
        }
        let id = self.next_id();
        self.closed.push((id, item));
        if self.closed.len() > MAX_CLOSED {
            self.closed.remove(0);
        }
        self.last_closed = Some((id, title));
    }

    /// "Closed X" with Undo, for 10 seconds, after a close the user made.
    pub(crate) fn offer_undo_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, title)) = self.last_closed.take() else {
            return;
        };
        let workbench = cx.entity().downgrade();
        let undo: crate::tasks::Run = std::rc::Rc::new(move |window, cx| {
            let _ = workbench.update(cx, |w, cx| w.reopen(Some(id), window, cx));
        });
        self.undo_close = Some((id, std::time::Instant::now()));
        crate::toasts::show(
            window,
            cx,
            crate::toasts::Toast::info(format!("Closed {title}"))
                .action(
                    if cfg!(target_os = "macos") {
                        "Undo ⌘Z"
                    } else {
                        "Undo"
                    },
                    undo,
                )
                .lifetime(Some(UNDO_CLOSE_FOR)),
        );
    }

    /// ⌘Z while the last close can still be undone; otherwise the key
    /// goes on.
    fn undo_close_action(&mut self, _: &UndoClose, window: &mut Window, cx: &mut Context<Self>) {
        match self.undo_close.take() {
            Some((id, at)) if at.elapsed() < UNDO_CLOSE_FOR => self.reopen(Some(id), window, cx),
            _ => cx.propagate(),
        }
    }

    /// Bring back closed item `id`, or the newest.
    pub(crate) fn reopen(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let index = match id {
            Some(id) => self.closed.iter().position(|(i, _)| *i == id),
            None => self.closed.len().checked_sub(1),
        };
        let Some(index) = index else {
            return;
        };
        let (_, item) = self.closed.remove(index);
        match item {
            Closed::Tab {
                workspace,
                tab,
                from_tab,
            } => {
                let w = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.id == workspace)
                    .unwrap_or(self.active);
                let Some(mut restored) = self.restore_tab_in(w, &tab, window, cx) else {
                    return;
                };
                let pane = self.next_id();
                let workspace = &mut self.workspaces[w];
                // A single pane rejoins the tab it was in, when that's open.
                let rejoin = from_tab
                    .filter(|_| restored.panes.len() == 1)
                    .and_then(|id| workspace.tabs.iter().position(|t| t.id == id));
                if let Some(t) = rejoin {
                    let surface = restored.panes.drain().next().map(|(_, s)| s);
                    if let Some(surface) = surface {
                        workspace.tabs[t].split(pane, surface, SplitAxis::Horizontal, false);
                    }
                    workspace.active = t;
                } else {
                    let at = (workspace.active + 1).min(workspace.tabs.len());
                    workspace.tabs.insert(at, restored);
                    workspace.active = at;
                }
                self.active = w;
                self.focus_active(window, cx);
                cx.notify();
            }
            Closed::Workspace {
                id: old,
                position,
                saved,
            } => {
                let Some(workspace) = self.restore_workspace(&saved, window, cx) else {
                    return;
                };
                // Tabs closed from it before go back to it.
                for (_, item) in &mut self.closed {
                    if let Closed::Tab { workspace: w, .. } = item {
                        if *w == old {
                            *w = workspace.id;
                        }
                    }
                }
                // Before Scratch, which stays last.
                let at = position.min(self.workspaces.len().saturating_sub(1));
                self.workspaces.insert(at, workspace);
                self.activate_workspace(at, window, cx);
            }
        }
    }

    /// `restore_tab` for workspace `w`, its terminals defaulting to its
    /// folder.
    fn restore_tab_in(
        &mut self,
        w: usize,
        tab: &impulse_kit::session::SessionTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Tab> {
        let dir = self.workspaces[w].default_directory();
        self.restore_tab(&dir, tab, window, cx)
    }

    /// Take a surface out of whichever tab holds it.
    pub(crate) fn close_surface(
        &mut self,
        surface: &Surface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notice = None;
        let id = surface_id(surface);
        for w in 0..self.workspaces.len() {
            for t in 0..self.workspaces[w].tabs.len() {
                let Some(pane) = self.workspaces[w].tabs[t].pane_of(|s| surface_id(s) == id) else {
                    continue;
                };
                self.editor_closed(surface);
                let workspace = self.workspaces[w].id;
                let from_tab = self.workspaces[w].tabs[t].id;
                self.remember(
                    Closed::Tab {
                        workspace,
                        tab: crate::session::session_surface_tab(surface, cx),
                        from_tab: Some(from_tab),
                    },
                    surface.name(cx),
                );
                if !self.workspaces[w].tabs[t].remove(pane) {
                    let was_active = self.workspaces[w].active == t;
                    let removed = self.workspaces[w].tabs.remove(t);
                    let back_to = was_active.then_some(removed.opener).flatten();
                    let previous = self.active;
                    let count = self.workspaces.len();
                    self.active = w;
                    self.after_tab_removed(t, back_to, window, cx);
                    // Its workspace may have gone with its last tab.
                    let gone = self.workspaces.len() < count;
                    if previous != w {
                        let previous = previous - usize::from(gone && previous > w);
                        if previous < self.workspaces.len() {
                            self.active = previous;
                            self.focus_active(window, cx);
                        }
                    }
                } else {
                    self.focus_active(window, cx);
                    cx.notify();
                }
                return;
            }
        }
    }

    /// The active workspace lost tab `index`: show its neighbor, or a new
    /// terminal when it was the last (a workspace always has a tab while
    /// it's shown).
    /// The tab at `index` is gone: the active one becomes the tab that
    /// opened it (`back_to`, when it was active and that tab is still
    /// there), else its right neighbour, else its left (Swift's).
    fn after_tab_removed(
        &mut self,
        index: usize,
        back_to: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A workspace closes with its last tab while others remain.
        if self.workspace().tabs.is_empty() && self.workspaces.len() > 1 {
            let active = self.active;
            return self.remove_workspace(active, false, window, cx);
        }
        let workspace = self.workspace_mut();
        if workspace.tabs.is_empty() {
            let dir = workspace.default_directory();
            let terminal = self.new_terminal(dir, Vec::new(), window, cx);
            let active = self.active;
            self.add_tab(active, terminal, window, cx);
            return;
        }
        let opener = back_to.and_then(|id| workspace.tabs.iter().position(|t| t.id == id));
        workspace.active = match opener {
            Some(at) => at,
            // The closed one was active: what slid into its place.
            None if workspace.active == index => index,
            None if workspace.active > index => workspace.active - 1,
            None => workspace.active,
        }
        .min(workspace.tabs.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// A message in the status bar.
    pub(crate) fn set_notice(&mut self, notice: String, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        crate::debug_snapshot::note_toast("notice", &notice, None, None);
        self.notice = Some(notice);
        cx.notify();
    }

    /// Read every listed folder again (files were created, moved or deleted).
    pub(crate) fn reload_tree(&mut self, cx: &mut Context<Self>) {
        let show_hidden = SettingsStore::get(cx).sidebar_show_hidden;
        for workspace in &mut self.workspaces {
            workspace.reload_tree(show_hidden);
        }
        cx.notify();
    }

    /// The folder new terminals and splits start in: the focused surface's.
    fn current_directory(&self, cx: &App) -> PathBuf {
        self.focused_surface()
            .and_then(|s| s.directory(cx))
            .filter(|d| d.is_dir())
            .unwrap_or_else(|| self.workspace().default_directory())
    }

    // MARK: Commands

    fn run_command_action(
        &mut self,
        action: &RunCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_command(&action.id, window, cx);
    }

    /// Run the command `id` (see `commands::catalog`).
    pub(crate) fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "new_tab" => {
                let dir = self.current_directory(cx);
                let terminal = self.new_terminal(dir, Vec::new(), window, cx);
                let active = self.active;
                self.add_tab(active, terminal, window, cx);
            }
            "close_tab" => self.close_focused(window, cx),
            "reopen_tab" => self.reopen(None, window, cx),
            "next_tab" | "prev_tab" => {
                let count = self.workspace().tabs.len();
                if count > 0 {
                    let current = self.workspace().active;
                    let next = if id == "next_tab" {
                        (current + 1) % count
                    } else {
                        (current + count - 1) % count
                    };
                    self.activate_tab(next, window, cx);
                }
            }
            "split_right" | "split_down" => {
                let dir = self.current_directory(cx);
                let terminal = self.new_terminal(dir, Vec::new(), window, cx);
                let axis = if id == "split_right" {
                    SplitAxis::Horizontal
                } else {
                    SplitAxis::Vertical
                };
                self.split_focused(terminal, axis, window, cx);
            }
            "focus_pane_left" => self.focus_neighbor(PaneDirection::Left, window, cx),
            "focus_pane_right" => self.focus_neighbor(PaneDirection::Right, window, cx),
            "focus_pane_up" => self.focus_neighbor(PaneDirection::Up, window, cx),
            "focus_pane_down" => self.focus_neighbor(PaneDirection::Down, window, cx),
            "resize_pane_left" => self.resize_focused(PaneDirection::Left, cx),
            "resize_pane_right" => self.resize_focused(PaneDirection::Right, cx),
            "resize_pane_up" => self.resize_focused(PaneDirection::Up, cx),
            "resize_pane_down" => self.resize_focused(PaneDirection::Down, cx),
            "next_pane" => self.cycle_pane(1, window, cx),
            "prev_pane" => self.cycle_pane(-1, window, cx),
            "zoom_pane" => self.toggle_zoom(cx),
            "equalize_panes" => self.equalize(cx),
            "move_pane_to_tab" => self.move_pane_to_tab(window, cx),
            "save" => {
                if let Some(editor) = self.active_editor().cloned() {
                    self.save_editor(editor, window, cx);
                }
            }
            "new_file" => self.new_untitled(window, cx),
            "install_lsp" => self.install_language_servers(window, cx),
            "paste" => window.dispatch_action(Box::new(crate::menus::MenuPaste), cx),
            "copy" => window.dispatch_action(Box::new(crate::menus::MenuCopy), cx),
            "import_shell_history" => self.import_shell_history(window, cx),
            "clear_command_history" => self.clear_command_history(window, cx),
            "toggle_sidebar" => {
                self.sidebar_shown = !self.sidebar_shown;
                cx.notify();
            }
            palette if crate::palette::prefix_for(palette).is_some() => {
                let prefix = crate::palette::prefix_for(palette).unwrap_or_default();
                self.show_palette(prefix, window, cx)
            }
            "show_files" | "show_changes" | "project_search" => {
                let panel = match id {
                    "show_files" => SidebarPanel::Files,
                    "show_changes" => SidebarPanel::Changes,
                    _ => SidebarPanel::Search,
                };
                // Pressed again while the panel has the keyboard: back to
                // the tab.
                if self.panel == panel && self.sidebar_shown && self.panel_has_focus(window, cx)
                {
                    self.focus_active(window, cx);
                    cx.notify();
                    return;
                }
                self.panel = panel;
                self.sidebar_shown = true;
                match self.panel {
                    SidebarPanel::Search => self
                        .search
                        .update(cx, |search, cx| search.focus(window, cx)),
                    SidebarPanel::Changes => self.focus_changes_list(window, cx),
                    SidebarPanel::Files => self.focus_tree(window, cx),
                }
                cx.notify();
            }
            "font_increase" | "font_decrease" | "font_reset" => {
                let delta = match id {
                    "font_increase" => 1,
                    "font_decrease" => -1,
                    _ => 0,
                };
                SettingsStore::update(cx, |s| {
                    if delta == 0 {
                        let defaults = impulse_kit::settings::Settings::default();
                        s.font_size = defaults.font_size;
                        s.terminal_font_size = defaults.terminal_font_size;
                    } else {
                        s.font_size += delta;
                        s.terminal_font_size += delta;
                    }
                });
            }
            "fullscreen" => window.toggle_fullscreen(),
            "review_changes" => self.open_review(None, window, cx),
            "git_history" => self.open_history(false, window, cx),
            "toggle_markdown_preview" | "preview_beside" => {
                let Some(editor) = self.active_editor().cloned() else {
                    return self.set_notice("Open a Markdown file to preview it.".into(), cx);
                };
                let beside = id == "preview_beside";
                if let Err(message) = editor.update(cx, |e, cx| e.toggle_preview(beside, cx)) {
                    self.set_notice(message, cx);
                }
            }
            "file_history" => self.open_history(true, window, cx),
            "trust_folder" => self.trust_active_folder(window, cx),
            "restrict_folder" => self.restrict_active_folder(window, cx),
            "forget_trusted_folders" => self.forget_trusted_folders(window, cx),
            "open_settings_json" => {
                let path = impulse_kit::settings::settings_path(cfg!(debug_assertions));
                if !path.exists() {
                    let defaults = impulse_kit::settings::Settings::default().encode();
                    let _ = std::fs::write(&path, defaults);
                }
                self.open_file(&path, window, cx);
            }
            "open_settings" => self.open_settings(None, window, cx),
            "open_keybindings" => self.open_keybindings(window, cx),
            "show_problems" => self.show_problems(window, cx),
            "open_workspace" => self.choose_workspace_folder(window, cx),
            "close_workspace" => self.close_workspace(self.active, window, cx),
            "quick_terminal" => crate::quick_terminal::toggle(cx),
            "component_gallery" => crate::component_gallery::show(None, false, cx),
            "manage_branches" => self.show_branch_manager(window, cx),
            "diff_view" => match self.active_editor().cloned() {
                Some(editor) => {
                    if let Err(message) =
                        editor.update(cx, |editor, cx| editor.toggle_diff_view(window, cx))
                    {
                        self.set_notice(message, cx);
                    }
                }
                None => self.set_notice("Open a file to see its changes.".into(), cx),
            },
            "editor_integration" => {
                SettingsStore::update(cx, |s| {
                    s.terminal_editor_integration = !s.terminal_editor_integration
                });
                let message = if SettingsStore::get(cx).terminal_editor_integration {
                    "New terminals use Impulse as $EDITOR (git commit opens a tab)."
                } else {
                    "New terminals keep your own $EDITOR."
                };
                self.toast(crate::toasts::Toast::success(message), window, cx);
            }
            "rename_workspace" => self.rename_workspace(self.active, window, cx),
            "clear_recent_workspaces" => self.clear_recent_workspaces(window, cx),
            "new_window" => {
                let args = Args {
                    theme: self.theme_override.clone(),
                    cwd: self.workspace().root.to_string_lossy().into_owned(),
                    commands: Vec::new(),
                    open: Vec::new(),
                    trust: false,
                    trust_store: self.trust.clone(),
                    session: None,
                };
                crate::open_main_window(args, cx);
            }
            git if crate::git_actions::GIT_COMMANDS.contains(&git) => {
                self.run_git_command(git, window, cx)
            }
            // Find, block commands and hints in the focused terminal.
            "find" if self.active_terminal().is_some() => {
                if let Some(terminal) = self.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.toggle_find(window, cx));
                }
            }
            // In an editor, GPUI Kit's search panel.
            "find" if self.active_editor().is_some() => {
                if let Some(editor) = self.active_editor().cloned() {
                    editor.update(cx, |tab, cx| {
                        tab.state
                            .update(cx, |state, cx| state.open_search(false, cx))
                    });
                }
            }
            // Review, History, images and tool tabs have no find, as in Swift.
            "find" => {}
            block if crate::terminal_blocks::BLOCK_COMMANDS.contains(&block) => {
                if let Some(terminal) = self.active_terminal().cloned() {
                    terminal.update(cx, |t, cx| t.perform_block_command(block, window, cx));
                }
            }
            "new_task" | "finish_task" | "archive_task" | "new_task_from_branch" => {
                self.run_task_command(id, window, cx)
            }
            agent if crate::agent_views::AGENT_COMMANDS.contains(&agent) => {
                self.run_agent_command(agent, window, cx)
            }
            "project_setup" => self.open_project_setup(None, None, window, cx),
            "edit_project_config" => self.edit_project_config(window, cx),
            custom if custom.starts_with("custom_") => {
                self.run_custom(&custom["custom_".len()..], window, cx)
            }
            other => {
                let title = crate::commands::catalog(SettingsStore::get(cx))
                    .into_iter()
                    .find(|c| c.id == other)
                    .map(|c| c.title)
                    .unwrap_or_else(|| other.to_string());
                self.set_notice(format!("{title} isn't in the Rust app yet."), cx);
            }
        }
    }

    /// A Run in Terminal shortcut: its command line in the focused terminal
    /// when its shell is at the prompt and the input bar is on, as the bar
    /// runs a command; otherwise (no terminal focused, one running
    /// something, or the bar off, when the shell's own line may hold typed
    /// text the command would join) in a new terminal tab in the active
    /// tab's folder, as Swift's `executeCustomCommand` does.
    fn run_custom(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(custom) = SettingsStore::get(cx)
            .custom_keybindings
            .iter()
            .find(|c| c.name == name)
            .cloned()
        else {
            return;
        };
        let shell =
            impulse_kit::shell::ShellType::detect(&impulse_kit::shell::login::default_shell_path());
        let line = std::iter::once(custom.command.trim().to_string())
            .chain(
                custom
                    .args
                    .iter()
                    .map(|a| impulse_kit::shell::quoting::quoted(a, shell)),
            )
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() {
            return;
        }
        let bar = SettingsStore::get(cx).terminal_context_bar;
        match self
            .active_terminal()
            .filter(|t| bar && t.read(cx).at_prompt())
        {
            Some(terminal) => terminal.update(cx, |t, _| t.run_command(&line)),
            None => {
                let dir = self.current_directory(cx);
                let terminal = self.new_terminal(dir, vec![line], window, cx);
                let active = self.active;
                self.add_tab(active, terminal, window, cx);
            }
        }
    }

    /// Open Folder as Workspace…
    fn choose_workspace_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |workbench, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = workbench.update_in(cx, |workbench, window, cx| {
                        workbench.open_workspace(&path, window, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Show the workspace for `folder`, adding one if there's none.
    pub(crate) fn open_workspace(
        &mut self,
        folder: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = crate::workspace::normalize(folder);
        let existing = self
            .workspaces
            .iter()
            .position(|w| w.kind == Kind::Folder && w.root == folder);
        let index = match existing {
            Some(index) => index,
            None => {
                let id = self.next_id();
                // Before Scratch, which stays last.
                let at = self
                    .workspaces
                    .iter()
                    .position(|w| w.kind == Kind::Scratch)
                    .unwrap_or(self.workspaces.len());
                impulse_kit::recent_workspaces::note(
                    &crate::workbench_workspaces::recents_file(),
                    &folder.to_string_lossy(),
                );
                self.workspaces
                    .insert(at, Workspace::new(id, Kind::Folder, folder));
                at
            }
        };
        self.activate_workspace(index, window, cx);
    }

    /// Close a folder workspace and its tabs (Scratch can't be closed),
    /// asking first about unsaved files and running commands as closing a
    /// tab does.
    pub(crate) fn close_workspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self
            .workspaces
            .get(index)
            .filter(|w| w.kind != Kind::Scratch)
            .map(|w| w.id)
        else {
            return;
        };
        self.confirm_close(
            crate::close_flow::Target::Workspace(id),
            CloseRiskAction::CloseTab,
            Default::default(),
            window,
            cx,
            Rc::new(move |w, window, cx| {
                if let Some(index) = w.workspaces.iter().position(|ws| ws.id == id) {
                    w.remove_workspace(index, true, window, cx);
                    w.offer_undo_close(window, cx);
                }
            }),
        );
    }

    /// Take the workspace at `index` out now, remembered for Undo Close
    /// when `remember`. Closing the last folder leaves Scratch to land in.
    pub(crate) fn remove_workspace(
        &mut self,
        index: usize,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.workspaces.len() {
            return;
        }
        if self.workspaces.len() == 1 {
            let id = self.next_id();
            self.workspaces
                .push(Workspace::new(id, Kind::Scratch, crate::workspace::home()));
        }
        let removed = self.workspaces.remove(index);
        for surface in removed.all_surfaces() {
            self.editor_closed(surface);
        }
        if remember {
            let saved = crate::session::session_workspace(&removed, true, cx);
            self.remember(
                Closed::Workspace {
                    id: removed.id,
                    position: index,
                    saved,
                },
                format!("workspace {}", removed.name()),
            );
        }
        self.recent_workspaces.retain(|w| *w != removed.id);
        // The workspace used before it, else its neighbour.
        let last_used = self
            .recent_workspaces
            .iter()
            .rev()
            .find_map(|id| self.workspaces.iter().position(|w| w.id == *id));
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        let active = last_used
            .unwrap_or(self.active)
            .min(self.workspaces.len() - 1);
        self.activate_workspace(active, window, cx);
    }

    fn format_document(&mut self, _: &FormatDocument, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.format_document(window, cx));
        }
    }

    fn rename_symbol(&mut self, _: &RenameSymbol, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.rename_symbol(window, cx));
        }
    }

    fn find_references(&mut self, _: &FindReferences, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.find_references(window, cx));
        }
    }
}

/// Identity of the entity behind a surface.
pub(crate) fn surface_id(surface: &Surface) -> gpui_kit::EntityId {
    match surface {
        Surface::Terminal(t) => t.entity_id(),
        Surface::Editor(e) => e.entity_id(),
        Surface::Review(r) => r.entity_id(),
        Surface::History(h) => h.entity_id(),
        Surface::Image(i) => i.entity_id(),
        Surface::Settings(s) => s.entity_id(),
        Surface::Keybindings(k) => k.entity_id(),
        Surface::TaskFinish(f) => f.entity_id(),
        Surface::Problems(p) => p.entity_id(),
        Surface::ProjectSetup(s) => s.entity_id(),
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_git(cx);
        self.sync_editor_watch(cx);
        self.sync_tree_watch();
        let sidebar = self.sidebar_shown;
        let root = self.menu_actions(div().id("workbench"), cx);
        let p = &self.palette;
        root.track_focus(&self.focus)
            .key_context("Workbench")
            .on_action(cx.listener(Self::run_command_action))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::undo_close_action))
            .on_mouse_move(cx.listener(Self::drag_divider))
            .on_mouse_up(gpui_kit::MouseButton::Left, cx.listener(Self::end_drag))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(crate::workbench_chrome::hsla(p.window))
            .font_family(UI_FONT)
            .text_size(px(13.))
            // SwiftUI's line height for the UI font, not GPUI's 1.6.
            .line_height(gpui_kit::relative(UI_LINE_HEIGHT))
            // The chrome's monospaced text shows no ligatures (Swift's is SF
            // Mono); the editor sets its own.
            .font_features(gpui_kit::FontFeatures::disable_ligatures())
            .text_color(crate::workbench_chrome::hsla(p.text))
            .child(crate::overlays::frame_start())
            .child(self.titlebar(window, cx))
            .children(self.settings_banner(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(sidebar, |row| row.child(self.sidebar(window, cx)))
                    .child(self.content(window, cx)),
            )
            .child(self.status_bar(cx))
            .child(gpui_kit::deferred(self.command_palette.clone()).with_priority(1))
            .children(crate::toasts::layer(&self.palette, window, cx))
            .children(crate::window_frame::resize_edges(window))
    }
}
