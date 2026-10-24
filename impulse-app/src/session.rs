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
    cx.global::<Sessions>()
        .windows
        .iter()
        .filter_map(|(w, _)| w.upgrade())
        .map(|w| w.read(cx).session_window(with_scrollback, cx))
        .collect()
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
    save(cx);
}

/// Write the session now (output files off the main thread is a later
/// refinement; the files are small and named by content, so unchanged
/// output isn't written again).
pub fn save(cx: &mut App) {
    if !persistence() || !cx.has_global::<Sessions>() {
        return;
    }
    let with_scrollback = SettingsStore::get(cx).restore_scrollback;
    let windows = windows(cx, with_scrollback);
    if windows.is_empty() {
        return;
    }
    let state = SessionState::new(windows, Some(0));
    state.persist(
        &SessionState::path(cfg!(debug_assertions)),
        &impulse_kit::session::scrollback_dir(cfg!(debug_assertions)),
    );
}

fn surface_state(surface: &Surface, with_scrollback: bool, cx: &App) -> SessionSurface {
    match surface {
        Surface::Terminal(t) => {
            let t = t.read(cx);
            SessionSurface {
                transcript: with_scrollback.then(|| t.transcript(SCROLLBACK_ROWS)),
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
        // Settings and Keyboard Shortcuts aren't saved, as in the Swift app.
        Surface::Settings(_) | Surface::Keybindings(_) => SessionSurface {
            kind: "tool".into(),
            ..Default::default()
        },
        Surface::History(h) => SessionSurface {
            kind: "history".into(),
            path: Some(h.read(cx).root.clone()),
            subpath: h.read(cx).path.clone(),
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

impl Workbench {
    /// This window as the session file holds it.
    pub(crate) fn session_window(&self, with_scrollback: bool, cx: &App) -> SessionWindow {
        let settings = SettingsStore::get(cx);
        let workspaces = self
            .workspaces
            .iter()
            .map(|workspace| {
                let tabs = workspace
                    .tabs
                    .iter()
                    .map(|tab| {
                        let order = tab.layout.leaves();
                        let index =
                            |id: &PaneId| order.iter().position(|o| o == id).unwrap_or(0) as i64;
                        SessionTab {
                            pinned: false,
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
                    })
                    .collect();
                SessionWorkspace {
                    kind: match workspace.kind {
                        Kind::Folder => "folder",
                        Kind::Scratch => "scratch",
                    }
                    .into(),
                    root: workspace.root.to_string_lossy().into_owned(),
                    name: workspace.custom_name.clone(),
                    expanded: Some(workspace.expanded),
                    tabs,
                    active_tab_index: Some(workspace.active as i64),
                    file_tree_root: None,
                }
            })
            .collect();
        SessionWindow {
            workspaces: Some(workspaces),
            active_workspace_index: Some(self.active as i64),
            sidebar_visible: Some(settings.sidebar_visible),
            sidebar_width: Some(settings.sidebar_width as f64),
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
        for saved in state.workspaces.unwrap_or_default() {
            let kind = if saved.kind == "folder" {
                Kind::Folder
            } else {
                Kind::Scratch
            };
            let root = PathBuf::from(&saved.root);
            if kind == Kind::Folder && !root.is_dir() {
                continue;
            }
            if kind == Kind::Scratch && self.workspaces.iter().any(|w| w.kind == Kind::Scratch) {
                continue;
            }
            let id = self.next_id();
            let mut workspace = Workspace::new(id, kind, root);
            workspace.custom_name = saved.name.clone();
            workspace.expanded = saved.expanded.unwrap_or(false);
            let mut restored_active = 0;
            for (t, tab) in saved.tabs.iter().enumerate() {
                let Some(tab) = self.restore_tab(&workspace, tab, window, cx) else {
                    continue;
                };
                if saved.active_tab_index == Some(t as i64) {
                    restored_active = workspace.tabs.len();
                }
                workspace.tabs.push(tab);
            }
            workspace.active = restored_active.min(workspace.tabs.len().saturating_sub(1));
            self.workspaces.push(workspace);
        }
        if self.workspaces.is_empty() {
            return;
        }
        // Scratch is always there, last.
        if !self.workspaces.iter().any(|w| w.kind == Kind::Scratch) {
            let id = self.next_id();
            self.workspaces
                .push(Workspace::new(id, Kind::Scratch, crate::workspace::home()));
        }
        if let Some(scratch) = self.workspaces.iter().position(|w| w.kind == Kind::Scratch) {
            let scratch = self.workspaces.remove(scratch);
            self.workspaces.push(scratch);
        }
        let active = state
            .active_workspace_index
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < self.workspaces.len())
            .unwrap_or(0);
        self.activate_workspace(active, window, cx);
    }

    fn restore_tab(
        &mut self,
        workspace: &Workspace,
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
                        .unwrap_or_else(|| workspace.default_directory());
                    Some(self.new_terminal_restoring(
                        cwd,
                        Vec::new(),
                        surface.transcript.clone(),
                        window,
                        cx,
                    ))
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
                    .map(|root| self.make_history(root, surface.subpath.clone(), window, cx)),
                "file" => surface
                    .path
                    .as_deref()
                    .map(Path::new)
                    .filter(|p| p.is_file())
                    .and_then(|path| self.make_editor(path, window, cx)),
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
        cx.subscribe(
            &editor,
            |_, _, _: &crate::editor_tab::EditorTabChanged, cx| cx.notify(),
        )
        .detach();
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        Some(Surface::Editor(editor))
    }
}
