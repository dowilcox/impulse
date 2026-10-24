//! `session-state.json`, as the Swift `SessionState` writes it: every
//! window, each with its workspaces in sidebar order, whose tabs are one
//! surface or several arranged by a `LayoutTree` of indexes into `panes`.
//! Terminals' recent output is kept beside it under `scrollback/`, one file
//! per terminal named by a digest of its bytes, so output that didn't
//! change isn't written again and the session file stays small. Version 1
//! files (one window's flat tab list) are migrated on load.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::layout_tree::LayoutTree;

const VERSION: i64 = 2;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub windows: Vec<SessionWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_window_index: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionWindow {
    /// Version 1 only: the file tree root and the flat tab list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<V1Tab>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab_index: Option<i64>,
    /// Version 2: the window's workspaces in sidebar order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<SessionWorkspace>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_workspace_index: Option<i64>,
    /// The window frame (`NSStringFromRect` on the Mac: "{{x, y}, {w, h}}").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces_height: Option<f64>,
}

/// A workspace and its tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionWorkspace {
    /// "folder" or "scratch".
    pub kind: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    pub tabs: Vec<SessionTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab_index: Option<i64>,
    /// Scratch only: where the file tree was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_tree_root: Option<String>,
}

/// A tab: one surface, or several arranged by `layout`, whose pane ids are
/// indexes into `panes`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionTab {
    #[serde(default)]
    pub pinned: bool,
    pub panes: Vec<SessionSurface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutTree<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_pane: Option<i64>,
}

/// A restorable surface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionSurface {
    /// "terminal", "file", "review" or "history".
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Editor cursor, 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<i64>,
    /// Terminal: file name of its saved output under `scrollback/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<String>,
    /// Terminal: a command that resumes the agent that was running in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    /// Terminal: its agents' recorded turns, kept as the Swift app wrote them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_turns: Option<serde_json::Value>,
    /// Review: the scope it showed, kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<serde_json::Value>,
    /// History: the file or folder it followed (repository-relative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    /// Terminal output carried in memory; never in the session file.
    #[serde(skip)]
    pub transcript: Option<String>,
}

impl SessionSurface {
    pub fn terminal(cwd: &str, title: Option<String>, shell: Option<String>) -> Self {
        SessionSurface {
            kind: "terminal".into(),
            cwd: Some(cwd.into()),
            title,
            shell,
            ..Default::default()
        }
    }

    pub fn file(path: &str, line: Option<i64>, column: Option<i64>) -> Self {
        SessionSurface {
            kind: "file".into(),
            path: Some(path.into()),
            line,
            column,
            ..Default::default()
        }
    }
}

/// A version 1 tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V1Tab {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<V1Pane>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pane_index: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V1Pane {
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

impl SessionWindow {
    /// A version 1 window becomes one Scratch workspace. Each old tab is a
    /// single-pane tab; old terminal splits keep only their active pane.
    pub fn migrated_from_v1(&self, home: &str) -> SessionWindow {
        let old = self.tabs.clone().unwrap_or_default();
        let survives = |tab: &V1Tab| tab.kind == "terminal" || tab.path.is_some();
        let tabs: Vec<SessionTab> = old
            .iter()
            .filter_map(|tab| match tab.kind.as_str() {
                "terminal" => {
                    let mut cwd = tab.cwd.clone().unwrap_or_default();
                    if let Some(panes) = tab.panes.as_ref().filter(|p| !p.is_empty()) {
                        let index = tab.active_pane_index.unwrap_or(0).max(0) as usize;
                        let pane = panes.get(index).unwrap_or(&panes[0]);
                        if !pane.cwd.is_empty() {
                            cwd = pane.cwd.clone();
                        }
                    }
                    Some(SessionTab {
                        pinned: tab.pinned,
                        panes: vec![SessionSurface::terminal(
                            &cwd,
                            tab.title.clone(),
                            tab.shell.clone(),
                        )],
                        ..Default::default()
                    })
                }
                "editor" => tab.path.as_ref().map(|path| SessionTab {
                    pinned: tab.pinned,
                    panes: vec![SessionSurface::file(path, None, None)],
                    ..Default::default()
                }),
                _ => None,
            })
            .collect();
        let active = self
            .active_tab_index
            .filter(|i| *i >= 0 && (*i as usize) < old.len())
            .map(|i| old[..i as usize].iter().filter(|t| survives(t)).count() as i64);
        SessionWindow {
            workspaces: Some(vec![SessionWorkspace {
                kind: "scratch".into(),
                root: home.into(),
                tabs,
                active_tab_index: active,
                file_tree_root: self.project_root.clone(),
                ..Default::default()
            }]),
            active_workspace_index: Some(0),
            ..Default::default()
        }
    }
}

impl SessionState {
    pub fn new(windows: Vec<SessionWindow>, active_window_index: Option<i64>) -> Self {
        SessionState {
            version: VERSION,
            windows,
            active_window_index,
        }
    }

    /// `session-state.json` in the data directory, beside settings.json.
    pub fn path(dev: bool) -> PathBuf {
        crate::paths::data_dir(dev).join("session-state.json")
    }

    /// Read a session, migrating version 1. `None` when there's none or it
    /// can't be read.
    pub fn load(path: &Path, home: &str) -> Option<SessionState> {
        let data = std::fs::read(path).ok()?;
        let mut state: SessionState = match serde_json::from_slice(&data) {
            Ok(state) => state,
            Err(error) => {
                log::error!(
                    "Failed to decode session state from {}: {error}",
                    path.display()
                );
                return None;
            }
        };
        match state.version {
            VERSION => Some(state),
            1 => {
                state.windows = state
                    .windows
                    .iter()
                    .map(|w| w.migrated_from_v1(home))
                    .collect();
                state.version = VERSION;
                Some(state)
            }
            other => {
                log::error!("Unsupported session state version {other}");
                None
            }
        }
    }

    pub fn active_window(&self) -> Option<&SessionWindow> {
        self.active_window_index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.windows.get(i))
            .or(self.windows.first())
    }

    /// Write the session file (pretty, keys sorted, 0600).
    pub fn save(&self, path: &Path) -> bool {
        let value = match serde_json::to_value(self) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Failed to encode session state: {error}");
                return false;
            }
        };
        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
        crate::settings::write_private_file(path, text.as_bytes())
            .inspect_err(|error| {
                log::error!(
                    "Failed to write session state to {}: {error}",
                    path.display()
                )
            })
            .is_ok()
    }

    /// Save the session and its terminals' output. Output files the new
    /// session doesn't use are removed only once its file is written, so a
    /// crash or a failed write leaves the previous session whole.
    pub fn persist(&self, path: &Path, scrollback: &Path) {
        let mut state = self.clone();
        let kept = store_scrollback(&mut state, scrollback);
        if state.save(path) {
            remove_scrollback_except(&kept, scrollback);
        }
    }

    /// What autosave compares to tell whether the session changed: its
    /// workspaces, tabs, folders and files, without what changes all the
    /// time (terminal titles, cursors, the selected tab, window frames).
    pub fn autosave_key(windows: &[SessionWindow]) -> String {
        let structure: Vec<SessionWindow> = windows
            .iter()
            .map(|window| {
                let mut window = window.clone();
                window.active_workspace_index = None;
                window.frame = None;
                window.sidebar_visible = None;
                window.sidebar_width = None;
                window.workspaces_height = None;
                if let Some(workspaces) = window.workspaces.as_mut() {
                    for workspace in workspaces {
                        workspace.active_tab_index = None;
                        for tab in &mut workspace.tabs {
                            tab.focused_pane = None;
                            for pane in &mut tab.panes {
                                pane.title = None;
                                pane.line = None;
                                pane.column = None;
                            }
                        }
                    }
                }
                window
            })
            .collect();
        serde_json::to_string(&serde_json::to_value(structure).unwrap_or_default())
            .unwrap_or_default()
    }
}

/// The scrollback folder beside the session file.
pub fn scrollback_dir(dev: bool) -> PathBuf {
    crate::paths::data_dir(dev).join("scrollback")
}

/// A transcript's file name: a digest of its bytes.
pub fn scrollback_file_name(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    name.push_str(".ansi");
    name
}

/// Point each terminal at a file holding its in-memory transcript, writing
/// the files that aren't there yet. Returns the names the session uses.
pub fn store_scrollback(state: &mut SessionState, dir: &Path) -> HashSet<String> {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut kept = HashSet::new();
    for window in &mut state.windows {
        for workspace in window.workspaces.iter_mut().flatten() {
            for tab in &mut workspace.tabs {
                for surface in &mut tab.panes {
                    let Some(text) = surface.transcript.as_ref().filter(|t| !t.is_empty()) else {
                        continue;
                    };
                    let name = scrollback_file_name(text.as_bytes());
                    let path = dir.join(&name);
                    if !kept.contains(&name)
                        && !path.exists()
                        && crate::settings::write_private_file(&path, text.as_bytes()).is_err()
                    {
                        continue;
                    }
                    surface.scrollback = Some(name.clone());
                    kept.insert(name);
                }
            }
        }
    }
    kept
}

/// Forget output from earlier saves: every file but `kept`.
pub fn remove_scrollback_except(kept: &HashSet<String>, dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !kept.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Read saved output into each terminal's transcript.
pub fn load_scrollback(workspaces: &mut [SessionWorkspace], dir: &Path) {
    for workspace in workspaces {
        for tab in &mut workspace.tabs {
            for surface in &mut tab.panes {
                let Some(name) = surface.scrollback.as_ref().filter(|n| !n.contains('/')) else {
                    continue;
                };
                if let Ok(data) = std::fs::read(dir.join(name)) {
                    surface.transcript = Some(String::from_utf8_lossy(&data).into_owned());
                }
            }
        }
    }
}

/// A window frame as `NSStringFromRect` writes it, "{{x, y}, {w, h}}",
/// the origin at the bottom left of the primary screen (AppKit's
/// coordinates), so the Swift and Rust apps read each other's sessions.
pub fn frame_string(x: f64, y: f64, width: f64, height: f64) -> String {
    format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}")
}

/// `frame_string`'s rectangle back, as `NSRectFromString` reads it: the
/// first four numbers, in order (`None` without four).
pub fn parse_frame(text: &str) -> Option<(f64, f64, f64, f64)> {
    let numbers: Vec<f64> = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == '+'))
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    match numbers[..] {
        [x, y, width, height, ..] => Some((x, y, width, height)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionState {
        SessionState::new(
            vec![SessionWindow {
                workspaces: Some(vec![SessionWorkspace {
                    kind: "folder".into(),
                    root: "/repo".into(),
                    tabs: vec![SessionTab {
                        panes: vec![
                            SessionSurface {
                                transcript: Some("hello\r\n".into()),
                                ..SessionSurface::terminal("/repo", Some("zsh".into()), None)
                            },
                            SessionSurface::file("/repo/a.rs", Some(3), Some(1)),
                        ],
                        layout: Some(LayoutTree::Split {
                            axis: crate::layout_tree::SplitAxis::Horizontal,
                            children: vec![LayoutTree::Leaf(0), LayoutTree::Leaf(1)],
                            ratios: vec![0.5, 0.5],
                        }),
                        focused_pane: Some(1),
                        pinned: false,
                    }],
                    active_tab_index: Some(0),
                    ..Default::default()
                }]),
                active_workspace_index: Some(0),
                ..Default::default()
            }],
            Some(0),
        )
    }

    #[test]
    fn round_trip_with_scrollback() {
        let dir = tempfile::tempdir().unwrap();
        let (path, scroll) = (
            dir.path().join("session-state.json"),
            dir.path().join("scrollback"),
        );
        std::fs::create_dir_all(&scroll).unwrap();
        std::fs::write(scroll.join("stale.ansi"), "old").unwrap();
        sample().persist(&path, &scroll);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"active_window_index\""));
        assert!(
            !text.contains("hello"),
            "output stays out of the session file"
        );
        let mut loaded = SessionState::load(&path, "/home/u").unwrap();
        assert!(!scroll.join("stale.ansi").exists());
        let workspaces = loaded.windows[0].workspaces.as_mut().unwrap();
        load_scrollback(workspaces, &scroll);
        let tab = &workspaces[0].tabs[0];
        assert_eq!(tab.panes[0].transcript.as_deref(), Some("hello\r\n"));
        assert_eq!(tab.panes[1].line, Some(3));
        assert_eq!(tab.layout.as_ref().unwrap().leaves(), vec![0, 1]);
    }

    #[test]
    fn migrates_version_1() {
        let v1 = r#"{"version":1,"windows":[{"project_root":"/p","active_tab_index":2,
            "tabs":[{"kind":"terminal","cwd":"/a","pinned":false,
                     "panes":[{"cwd":"/b"},{"cwd":"/c"}],"active_pane_index":1},
                    {"kind":"image","pinned":false},
                    {"kind":"editor","path":"/p/x.rs","pinned":true}]}]}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, v1).unwrap();
        let state = SessionState::load(&path, "/home/u").unwrap();
        let workspace = &state.windows[0].workspaces.as_ref().unwrap()[0];
        assert_eq!(workspace.kind, "scratch");
        assert_eq!(workspace.root, "/home/u");
        assert_eq!(workspace.file_tree_root.as_deref(), Some("/p"));
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.tabs[0].panes[0].cwd.as_deref(), Some("/c"));
        assert_eq!(workspace.active_tab_index, Some(1));
    }

    #[test]
    fn autosave_key_ignores_volatile_state() {
        let a = sample();
        let mut b = sample();
        let w = &mut b.windows[0];
        w.frame = Some("{{0, 0}, {10, 10}}".into());
        let tab = &mut w.workspaces.as_mut().unwrap()[0].tabs[0];
        tab.focused_pane = Some(0);
        tab.panes[0].title = Some("vim".into());
        assert_eq!(
            SessionState::autosave_key(&a.windows),
            SessionState::autosave_key(&b.windows)
        );
        b.windows[0].workspaces.as_mut().unwrap()[0].tabs[0].panes[1].path =
            Some("/repo/b.rs".into());
        assert_ne!(
            SessionState::autosave_key(&a.windows),
            SessionState::autosave_key(&b.windows)
        );
    }

    #[test]
    fn frames_read_back_as_appkit_writes_them() {
        assert_eq!(
            frame_string(10.0, 20.5, 1200.0, 800.0),
            "{{10, 20.5}, {1200, 800}}"
        );
        assert_eq!(
            parse_frame("{{10, 20.5}, {1200, 800}}"),
            Some((10.0, 20.5, 1200.0, 800.0))
        );
        assert_eq!(
            parse_frame("{{-1440, 0}, {900, 600}}"),
            Some((-1440.0, 0.0, 900.0, 600.0))
        );
        assert_eq!(parse_frame("{{1, 2}, {3}}"), None);
    }
}
