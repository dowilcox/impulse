//! Session restore (the Swift app's `SessionState` and
//! `MainWindowController+Session`): every window's workspaces, tabs, split
//! layouts, files and terminal folders, with each terminal's recent output,
//! saved to `session-state.json` on quit and, so a crash loses little,
//! every 15 seconds when the structure changed; brought back at launch when
//! `restore_session` is on.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Global, WeakEntity, Window};
use impulse_kit::session::{
    SessionState, SessionSurface, SessionTab, SessionWindow, SessionWorkspace,
};

use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;
use crate::workspace::{Kind, PaneId, Surface, Tab, Workspace};

/// Rows of output kept per terminal, as the Swift app keeps.
const SCROLLBACK_ROWS: usize = 2000;

/// The open windows, for saving the session.
struct Sessions {
    windows: Vec<(WeakEntity<Workbench>, gpui_kit::AnyWindowHandle)>,
    /// What the last autosave saw (`SessionState::autosave_key`).
    autosaved: String,
}

impl Global for Sessions {}

fn persistence() -> bool {
    std::env::var_os("IMPULSE_NO_PERSISTENCE").is_none()
}

/// The saved session to restore at launch, if there is one and the
/// settings want it.
pub fn saved(cx: &App) -> Option<SessionState> {
    if !SettingsStore::get(cx).restore_session || !persistence() {
        return None;
    }
    let home = crate::workspace::home().to_string_lossy().into_owned();
    let mut state = SessionState::load(&SessionState::path(cfg!(debug_assertions)), &home)?;
    if state.windows.is_empty() {
        return None;
    }
    let dir = impulse_kit::session::scrollback_dir(cfg!(debug_assertions));
    if SettingsStore::get(cx).restore_scrollback {
        for window in &mut state.windows {
            if let Some(workspaces) = window.workspaces.as_mut() {
                impulse_kit::session::load_scrollback(workspaces, &dir);
            }
        }
    }
    Some(state)
}

/// Start saving: every 15 seconds when something changed, and at quit.
pub fn init(cx: &mut App) {
    cx.set_global(Sessions {
        windows: Vec::new(),
        autosaved: String::new(),
    });
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_secs(15))
            .await;
        cx.update(autosave);
    })
    .detach();
    cx.on_app_quit(|cx| {
        save(cx);
        async {}
    })
    .detach();
}

pub fn register(workbench: WeakEntity<Workbench>, window: gpui_kit::AnyWindowHandle, cx: &mut App) {
    if cx.has_global::<Sessions>() {
        cx.global_mut::<Sessions>()
            .windows
            .push((workbench, window));
    }
}

/// Every open window's workbench, with its window.
pub fn workbenches(cx: &App) -> Vec<(gpui_kit::Entity<Workbench>, gpui_kit::AnyWindowHandle)> {
    if !cx.has_global::<Sessions>() {
        return Vec::new();
    }
    cx.global::<Sessions>()
        .windows
        .iter()
        .filter_map(|(w, handle)| w.upgrade().map(|w| (w, *handle)))
        .collect()
}

fn windows(cx: &App, with_scrollback: bool) -> Vec<SessionWindow> {
    open_windows(cx, with_scrollback)
        .into_iter()
        .map(|(window, _)| window)
        .collect()
}

/// The open windows' sessions, each with whether it's the active one.
fn open_windows(cx: &App, with_scrollback: bool) -> Vec<(SessionWindow, bool)> {
    let active = cx.active_window();
    cx.global::<Sessions>()
        .windows
        .iter()
        .filter_map(|(w, handle)| w.upgrade().map(|w| (w, *handle)))
        .map(|(w, handle)| {
            (
                w.read(cx).session_window(with_scrollback, cx),
                Some(handle) == active,
            )
        })
        .collect()
}

/// A window's frame for the session, in AppKit's coordinates (the origin
/// at the primary screen's bottom left), as the Swift app writes it.
fn frame_of(bounds: gpui_kit::Bounds<gpui_kit::Pixels>, cx: &App) -> String {
    let screen = cx
        .primary_display()
        .map(|d| f32::from(d.bounds().size.height))
        .unwrap_or(0.);
    let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    impulse_kit::session::frame_string(
        x.round() as f64,
        (screen - y - height).round() as f64,
        width.round() as f64,
        height.round() as f64,
    )
}

/// The bounds a saved frame asks for, when it's a usable size and still
/// on a screen (Swift's `restoreSessionWindow` checks the same).
pub fn saved_bounds(frame: &str, cx: &App) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    let (x, y, width, height) = impulse_kit::session::parse_frame(frame)?;
    if width <= 200. || height <= 200. {
        return None;
    }
    let screen = f32::from(cx.primary_display()?.bounds().size.height);
    let bounds = gpui_kit::Bounds::new(
        gpui_kit::point(
            gpui_kit::px(x as f32),
            gpui_kit::px(screen - y as f32 - height as f32),
        ),
        gpui_kit::size(gpui_kit::px(width as f32), gpui_kit::px(height as f32)),
    );
    cx.displays()
        .iter()
        .any(|d| d.bounds().intersects(&bounds))
        .then_some(bounds)
}

fn autosave(cx: &mut App) {
    if !persistence() || !cx.has_global::<Sessions>() {
        return;
    }
    let key = SessionState::autosave_key(&windows(cx, false));
    if key == cx.global::<Sessions>().autosaved {
        return;
    }
    cx.global_mut::<Sessions>().autosaved = key;
    if let Some(state) = current(cx) {
        // The files are written off the main thread, one save at a time.
        let order = next_order();
        cx.background_spawn(async move { write(&state, order) })
            .detach();
    }
}

/// Write the session now (at quit, and as the last window closes), and
/// keep the front window's size for new windows (Swift's
/// `window_width`/`window_height`).
pub fn save(cx: &mut App) {
    if let Some(state) = current(cx) {
        write(&state, next_order());
    }
    let front = cx.active_window();
    let size = workbenches(cx)
        .into_iter()
        .find(|(_, h)| Some(*h) == front)
        .or_else(|| workbenches(cx).into_iter().next())
        .map(|(w, _)| w.read(cx).bounds.size);
    if let Some(size) = size {
        let (width, height) = (f32::from(size.width) as i64, f32::from(size.height) as i64);
        if width > 0 && height > 0 {
            SettingsStore::update(cx, |s| {
                s.window_width = width;
                s.window_height = height;
            });
        }
    }
}

/// The files open in editors in every window, each once (`open_files`,
/// opened again when there's no session to restore).
fn note_open_files(cx: &mut App) {
    let mut seen = std::collections::HashSet::new();
    let files: Vec<String> = workbenches(cx)
        .iter()
        .flat_map(|(w, _)| {
            w.read(cx)
                .workspaces
                .iter()
                .flat_map(|ws| ws.all_surfaces())
                .filter_map(Surface::editor)
                .filter(|e| e.read(cx).untitled.is_none())
                .map(|e| e.read(cx).path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .filter(|path| seen.insert(path.clone()))
        .collect();
    if SettingsStore::get(cx).open_files != files {
        SettingsStore::update(cx, |s| s.open_files = files);
    }
}

/// Each save's place in line: an older one never overwrites a newer.
static ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_order() -> u64 {
    ORDER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

/// Saves run one at a time (an autosave in the background, then the one
/// at quit).
/// The lock holds the order of the last save written.
static WRITING: std::sync::Mutex<u64> = std::sync::Mutex::new(0);

fn write(state: &SessionState, order: u64) {
    let mut written = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if order < *written {
        return;
    }
    *written = order;
    state.persist(
        &SessionState::path(cfg!(debug_assertions)),
        &impulse_kit::session::scrollback_dir(cfg!(debug_assertions)),
    );
}

/// The session as it stands, scrollback included when it's kept.
fn current(cx: &mut App) -> Option<SessionState> {
    if !persistence() || !cx.has_global::<Sessions>() {
        return None;
    }
    note_open_files(cx);
    let with_scrollback = SettingsStore::get(cx).restore_scrollback;
    let windows = open_windows(cx, with_scrollback);
    if windows.is_empty() {
        return None;
    }
    let active = windows.iter().position(|(_, active)| *active).unwrap_or(0);
    let windows = windows.into_iter().map(|(window, _)| window).collect();
    Some(SessionState::new(windows, Some(active as i64)))
}

fn surface_state(surface: &Surface, with_scrollback: bool, cx: &App) -> SessionSurface {
    match surface {
        Surface::Terminal(t) => {
            let t = t.read(cx);
            let turns = crate::agent_checkpoints::saved_turns(&t.agent.id);
            // An agent that told its session: the command that resumes it.
            let resume = t
                .agent
                .agent
                .as_ref()
                .and(t.agent.session.as_ref())
                .and_then(|(agent, session)| {
                    impulse_kit::agents::known_agents::resume_command(agent, session)
                });
            SessionSurface {
                transcript: with_scrollback.then(|| t.transcript(SCROLLBACK_ROWS)),
                resume,
                agent_turns: (!turns.is_empty())
                    .then(|| serde_json::to_value(&turns).ok())
                    .flatten(),
                ..SessionSurface::terminal(
                    &t.cwd,
                    Some(t.title.clone()).filter(|s| !s.is_empty()),
                    Some(crate::workspace::shell_name()),
                )
            }
        }
        Surface::Review(r) => SessionSurface {
            kind: "review".into(),
            path: Some(r.read(cx).root.clone()),
            scope: serde_json::to_value(&r.read(cx).scope).ok(),
            ..Default::default()
        },
        // Settings, Keyboard Shortcuts, Problems, Finish Task and Project
        // Setup aren't saved, as in the Swift app.
        Surface::Settings(_)
        | Surface::Keybindings(_)
        | Surface::Problems(_)
        | Surface::TaskFinish(_)
        | Surface::ProjectSetup(_) => SessionSurface {
            kind: "tool".into(),
            ..Default::default()
        },
        Surface::Image(i) => SessionSurface::file(&i.read(cx).path.to_string_lossy(), None, None),
        Surface::History(h) => SessionSurface {
            kind: "history".into(),
            path: Some(h.read(cx).root.clone()),
            subpath: h.read(cx).path.clone(),
            ..Default::default()
        },
        // A file never saved isn't either.
        Surface::Editor(e) if e.read(cx).untitled.is_some() => SessionSurface {
            kind: "tool".into(),
            ..Default::default()
        },
        Surface::Editor(e) => {
            let e = e.read(cx);
            let (line, column) = e.cursor(cx);
            SessionSurface::file(
                &e.path.to_string_lossy(),
                Some(line as i64),
                Some(column as i64),
            )
        }
    }
}

/// A tab as the session file holds it.
pub(crate) fn session_tab(tab: &Tab, with_scrollback: bool, cx: &App) -> SessionTab {
    let order = tab.layout.leaves();
    let index = |id: &PaneId| order.iter().position(|o| o == id).unwrap_or(0) as i64;
    SessionTab {
        pinned: tab.pinned,
        panes: order
            .iter()
            .filter_map(|id| tab.panes.get(id))
            .map(|s| surface_state(s, with_scrollback, cx))
            .collect(),
        layout: tab
            .is_split()
            .then(|| tab.layout.map_panes(&mut |id| index(id))),
        focused_pane: tab.is_split().then(|| index(&tab.focused)),
    }
}

/// One surface as a tab of its own (a pane closed out of a split).
pub(crate) fn session_surface_tab(surface: &Surface, cx: &App) -> SessionTab {
    SessionTab {
        panes: vec![surface_state(surface, true, cx)],
        ..Default::default()
    }
}

/// A workspace as the session file holds it.
pub(crate) fn session_workspace(
    workspace: &Workspace,
    with_scrollback: bool,
    cx: &App,
) -> SessionWorkspace {
    SessionWorkspace {
        kind: match workspace.kind {
            Kind::Folder => "folder",
            Kind::Scratch => "scratch",
        }
        .into(),
        root: workspace.root.to_string_lossy().into_owned(),
        name: workspace.custom_name.clone(),
        expanded: Some(workspace.expanded),
        tabs: workspace
            .tabs
            .iter()
            .map(|tab| session_tab(tab, with_scrollback, cx))
            .collect(),
        active_tab_index: Some(workspace.active as i64),
        file_tree_root: None,
    }
}

impl Workbench {
    /// This window as the session file holds it.
    pub(crate) fn session_window(&self, with_scrollback: bool, cx: &App) -> SessionWindow {
        // A subagent's worktree listed but never shown is the subagent's
        // while it runs, not one to come back as a workspace of its own.
        let kept =
            |workspace: &Workspace| workspace.subagent.is_none() || !workspace.tabs.is_empty();
        let workspaces = self
            .workspaces
            .iter()
            .filter(|workspace| kept(workspace))
            .map(|workspace| session_workspace(workspace, with_scrollback, cx))
            .collect();
        let active = self.workspaces[..self.active.min(self.workspaces.len())]
            .iter()
            .filter(|workspace| kept(workspace))
            .count();
        SessionWindow {
            workspaces: Some(workspaces),
            active_workspace_index: Some(active as i64),
            sidebar_visible: Some(self.sidebar_shown),
            sidebar_width: Some(self.sidebar_px as f64),
            frame: Some(frame_of(self.bounds, cx)),
            workspaces_height: self.workspaces_height.map(f64::from),
            ..Default::default()
        }
    }

    /// Bring back a saved window: its workspaces (folders that are gone
    /// are left out), tabs, layouts, files and terminals.
    pub(crate) fn restore(
        &mut self,
        state: SessionWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The sidebar as this window had it (Swift keeps it per window).
        self.workspaces_height = state.workspaces_height.map(|h| h.max(0.) as f32);
        if let Some(width) = state.sidebar_width.filter(|w| *w > 120.) {
            self.sidebar_px = width.round() as f32;
        }
        if let Some(visible) = state.sidebar_visible {
            self.sidebar_shown = visible;
        }
        for saved in state.workspaces.unwrap_or_default() {
            if saved.kind != "folder" && self.workspaces.iter().any(|w| w.kind == Kind::Scratch) {
                continue;
            }
            let Some(workspace) = self.restore_workspace(&saved, window, cx) else {
                continue;
            };
            self.workspaces.push(workspace);
        }
        if self.workspaces.is_empty() {
            return;
        }
        let mut active = state
            .active_workspace_index
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < self.workspaces.len())
            .unwrap_or(0);
        // An empty Scratch beside folders goes, unless it's the one shown
        // (Swift's `finishRestore`); closing the last folder brings it back.
        if let Some(scratch) = self.workspaces.iter().position(|w| w.kind == Kind::Scratch) {
            if self.workspaces.len() > 1
                && self.workspaces[scratch].tabs.is_empty()
                && scratch != active
            {
                self.workspaces.remove(scratch);
                if active > scratch {
                    active -= 1;
                }
            } else {
                // Scratch stays last.
                let workspace = self.workspaces.remove(scratch);
                self.workspaces.push(workspace);
                if active == scratch {
                    active = self.workspaces.len() - 1;
                } else if active > scratch {
                    active -= 1;
                }
            }
        }
        self.show_workspace(active, window, cx);
    }

    /// A saved workspace with its tabs; `None` for a folder that's gone.
    pub(crate) fn restore_workspace(
        &mut self,
        saved: &SessionWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Workspace> {
        let kind = if saved.kind == "folder" {
            Kind::Folder
        } else {
            Kind::Scratch
        };
        let root = PathBuf::from(&saved.root);
        if kind == Kind::Folder && !root.is_dir() {
            return None;
        }
        let id = self.next_id();
        let mut workspace = Workspace::new(id, kind, root);
        workspace.custom_name = saved.name.clone();
        workspace.expanded = saved.expanded.unwrap_or(false);
        let mut restored_active = 0;
        for (t, tab) in saved.tabs.iter().enumerate() {
            let dir = workspace.default_directory();
            let Some(tab) = self.restore_tab(&dir, tab, window, cx) else {
                continue;
            };
            if saved.active_tab_index == Some(t as i64) {
                restored_active = workspace.tabs.len();
            }
            workspace.tabs.push(tab);
        }
        workspace.active = restored_active.min(workspace.tabs.len().saturating_sub(1));
        Some(workspace)
    }

    pub(crate) fn restore_tab(
        &mut self,
        default_dir: &Path,
        saved: &SessionTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Tab> {
        // Saved pane index -> the new pane id, for the surfaces that came back.
        let mut panes: Vec<(i64, PaneId, Surface)> = Vec::new();
        for (index, surface) in saved.panes.iter().enumerate() {
            let restored = match surface.kind.as_str() {
                "terminal" => {
                    let cwd = surface
                        .cwd
                        .as_deref()
                        .map(PathBuf::from)
                        .filter(|d| d.is_dir())
                        .unwrap_or_else(|| default_dir.to_path_buf());
                    let terminal = self.new_terminal_restoring(
                        cwd,
                        Vec::new(),
                        surface.transcript.clone(),
                        window,
                        cx,
                    );
                    // Its agents' turns, for Review Last Agent Turn and the
                    // toolbelt's Turns.
                    let saved = surface
                        .agent_turns
                        .clone()
                        .and_then(|turns| serde_json::from_value(turns).ok());
                    if let (Some(saved), Surface::Terminal(view)) = (saved, &terminal) {
                        crate::agent_checkpoints::restore(saved, &view.read(cx).agent.id);
                    }
                    // An agent was running here: offer to pick its session
                    // back up.
                    if let (Some(resume), Surface::Terminal(view)) = (&surface.resume, &terminal) {
                        view.update(cx, |view, cx| view.set_draft(resume, cx));
                    }
                    Some(terminal)
                }
                "review" => surface
                    .path
                    .clone()
                    .filter(|p| std::path::Path::new(p).is_dir())
                    .map(|root| {
                        let scope = surface
                            .scope
                            .clone()
                            .and_then(|s| serde_json::from_value(s).ok());
                        self.make_review(root, scope, window, cx)
                    }),
                "history" => surface
                    .path
                    .clone()
                    .filter(|p| std::path::Path::new(p).is_dir())
                    .map(|root| self.make_history(root, surface.subpath.clone(), None, window, cx)),
                "file" => surface
                    .path
                    .as_deref()
                    .map(Path::new)
                    .filter(|p| p.is_file())
                    .and_then(|path| {
                        if crate::image_view::is_image_file(path) {
                            Some(self.make_image(path, cx))
                        } else {
                            let editor = self.make_editor(path, window, cx);
                            // Back where its cursor was (1-based in the file).
                            if let (Some(Surface::Editor(e)), Some(line)) = (&editor, surface.line)
                            {
                                let position = gpui_kit::component::input::Position::new(
                                    (line.max(1) - 1) as u32,
                                    (surface.column.unwrap_or(1).max(1) - 1) as u32,
                                );
                                e.update(cx, |e, cx| e.go_to(position, window, cx));
                            }
                            editor
                        }
                    }),
                _ => None,
            };
            if let Some(restored) = restored {
                let id = self.next_id();
                panes.push((index as i64, id, restored));
            }
        }
        let (_, first_id, first) = panes.first().cloned()?;
        let tab_id = self.next_id();
        let mut tab = Tab::new(tab_id, first_id, first);
        tab.pinned = saved.pinned;
        let id_of = |index: i64| {
            panes
                .iter()
                .find(|(i, _, _)| *i == index)
                .map(|(_, id, _)| *id)
        };
        // The saved layout, without the panes that didn't come back.
        let layout = saved.layout.as_ref().and_then(|layout| {
            let mut pruned = Some(layout.clone());
            for leaf in layout.leaves() {
                if id_of(leaf).is_none() {
                    pruned = pruned.and_then(|l| l.removing(&leaf));
                }
            }
            pruned.map(|l| l.map_panes(&mut |i| id_of(*i).unwrap_or(first_id)))
        });
        if let Some(layout) = layout {
            tab.layout = layout;
            for (_, id, surface) in panes.iter().skip(1) {
                tab.panes.insert(*id, surface.clone());
            }
            tab.focused = saved
                .focused_pane
                .and_then(id_of)
                .filter(|id| tab.panes.contains_key(id))
                .unwrap_or(first_id);
        }
        Some(tab)
    }

    /// An editor for `path`, wired to the workbench, not yet in a tab.
    pub(crate) fn make_editor(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Surface> {
        let opened = impulse_kit::text_file::open(path).ok()?;
        let editor = cx.new(|cx| crate::editor_tab::EditorTab::new(path, opened, window, cx));
        self.wire_editor_git(&editor, window, cx);
        cx.subscribe(
            &editor,
            |_, _, _: &crate::editor_tab::EditorTabChanged, cx| cx.notify(),
        )
        .detach();
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        Some(Surface::Editor(editor))
    }
}
