//! A window's workspaces, their tabs and the panes in a tab, as the Swift
//! app's `Workspace`, `TabEntry` and `SplitTab` hold them: a workspace is a
//! folder (or Scratch, which follows the home folder or the Scratch folder
//! setting) with its own tabs; a tab is one surface or a split of several,
//! laid out by an `impulse_kit::layout_tree::LayoutTree` of pane ids.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::{App, Entity, FocusHandle, Window};
use impulse_kit::layout_tree::{LayoutTree, SplitAxis};

use crate::editor_tab::EditorTab;
use crate::lsp_hub::LspHub;
use crate::terminal_view::TerminalView;

pub type PaneId = u64;

/// What a pane shows.
#[derive(Clone)]
pub enum Surface {
    Terminal(Entity<TerminalView>),
    Editor(Entity<EditorTab>),
}

impl Surface {
    /// The tab strip's title: a terminal's title (or its shell), a file's name.
    pub fn title(&self, cx: &App) -> String {
        match self {
            Surface::Terminal(t) => {
                let title = t.read(cx).title.clone();
                if title.is_empty() {
                    crate::workspace::shell_name()
                } else {
                    title
                }
            }
            Surface::Editor(e) => e.read(cx).title.clone(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Surface::Terminal(_) => "square-terminal",
            Surface::Editor(_) => "file-code",
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Surface::Terminal(_) => false,
            Surface::Editor(e) => e.read(cx).dirty,
        }
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Surface::Terminal(t) => t.read(cx).focus_handle().clone(),
            Surface::Editor(e) => e.read(cx).focus_handle(cx),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            // The input bar when it's showing, otherwise the grid.
            Surface::Terminal(t) => t.update(cx, |t, cx| t.focus(window, cx)),
            Surface::Editor(e) => e.update(cx, |e, cx| e.focus(window, cx)),
        }
    }

    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle(cx).contains_focused(window, cx)
    }

    /// The folder it's in: a terminal's working directory, a file's folder.
    pub fn directory(&self, cx: &App) -> Option<PathBuf> {
        match self {
            Surface::Terminal(t) => Some(PathBuf::from(&t.read(cx).cwd)),
            Surface::Editor(e) => e.read(cx).path.parent().map(Path::to_path_buf),
        }
    }

    pub fn editor(&self) -> Option<&Entity<EditorTab>> {
        match self {
            Surface::Editor(e) => Some(e),
            _ => None,
        }
    }

    pub fn terminal(&self) -> Option<&Entity<TerminalView>> {
        match self {
            Surface::Terminal(t) => Some(t),
            _ => None,
        }
    }
}

/// The name shown for terminals: the login shell's.
pub fn shell_name() -> String {
    impulse_kit::shell::login::default_shell_name()
}

/// One tab: a surface, or several in a split.
pub struct Tab {
    pub id: u64,
    pub layout: LayoutTree<PaneId>,
    pub panes: HashMap<PaneId, Surface>,
    pub focused: PaneId,
    /// A pane shown alone while the others stay in the layout.
    pub zoomed: Option<PaneId>,
}

impl Tab {
    pub fn new(id: u64, pane: PaneId, surface: Surface) -> Self {
        Tab {
            id,
            layout: LayoutTree::Leaf(pane),
            panes: HashMap::from([(pane, surface)]),
            focused: pane,
            zoomed: None,
        }
    }

    pub fn focused_surface(&self) -> &Surface {
        &self.panes[&self.focused]
    }

    /// Panes in reading order.
    pub fn surfaces(&self) -> Vec<(PaneId, &Surface)> {
        self.layout
            .leaves()
            .into_iter()
            .filter_map(|id| self.panes.get(&id).map(|s| (id, s)))
            .collect()
    }

    pub fn is_split(&self) -> bool {
        self.panes.len() > 1
    }

    /// Put `surface` beside the focused pane and focus it.
    pub fn split(&mut self, pane: PaneId, surface: Surface, axis: SplitAxis, before: bool) {
        self.layout = self.layout.splitting(&self.focused, pane, axis, before);
        self.panes.insert(pane, surface);
        self.focused = pane;
        self.zoomed = None;
    }

    /// Take a pane out. False when it was the last one (the tab closes).
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let Some(layout) = self.layout.removing(&pane) else {
            return false;
        };
        let order = self.layout.leaves();
        self.layout = layout;
        self.panes.remove(&pane);
        if self.zoomed == Some(pane) {
            self.zoomed = None;
        }
        if self.focused == pane {
            // The pane before it, as closing a split in the Swift app does.
            let index = order.iter().position(|id| *id == pane).unwrap_or(0);
            let leaves = self.layout.leaves();
            self.focused = leaves[index.saturating_sub(1).min(leaves.len() - 1)];
        }
        true
    }

    pub fn pane_of(&self, surface_is: impl Fn(&Surface) -> bool) -> Option<PaneId> {
        self.surfaces()
            .into_iter()
            .find(|(_, s)| surface_is(s))
            .map(|(id, _)| id)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Folder,
    Scratch,
}

/// A folder the window works in, with its own tabs.
pub struct Workspace {
    pub id: u64,
    pub kind: Kind,
    pub root: PathBuf,
    pub custom_name: Option<String>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// The sidebar lists this workspace's tabs under its row.
    pub expanded: bool,
    pub branch: String,
    /// Each listed folder's entries (folders first, then by name), read
    /// when the folder is first expanded.
    pub children: HashMap<PathBuf, Vec<(String, bool)>>,
    pub expanded_dirs: HashSet<PathBuf>,
    /// Language servers, once the folder is trusted.
    pub lsp: Option<Rc<LspHub>>,
    pub lsp_events: Option<gpui_kit::Task<()>>,
    /// The user kept this folder restricted this session: don't ask again.
    pub declined: bool,
}

impl Workspace {
    pub fn new(id: u64, kind: Kind, root: PathBuf) -> Self {
        let root = match kind {
            Kind::Scratch => scratch_root(),
            Kind::Folder => normalize(&root),
        };
        let mut workspace = Workspace {
            id,
            kind,
            branch: String::new(),
            children: HashMap::new(),
            expanded_dirs: HashSet::new(),
            root,
            custom_name: None,
            tabs: Vec::new(),
            active: 0,
            expanded: false,
            lsp: None,
            lsp_events: None,
            declined: false,
        };
        workspace.refresh_branch();
        workspace.reload_tree();
        workspace
    }

    pub fn name(&self) -> String {
        if let Some(name) = self.custom_name.as_ref().filter(|n| !n.is_empty()) {
            return name.clone();
        }
        match self.kind {
            Kind::Scratch => "Scratch".into(),
            Kind::Folder => self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.root.display().to_string()),
        }
    }

    /// Where new terminals start.
    pub fn default_directory(&self) -> PathBuf {
        match self.kind {
            Kind::Scratch => scratch_root(),
            Kind::Folder => self.root.clone(),
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn refresh_branch(&mut self) {
        self.branch = if self.kind == Kind::Folder {
            git_branch(&self.root)
        } else {
            String::new()
        };
    }

    /// Read every listed folder again.
    pub fn reload_tree(&mut self) {
        let show_hidden = false;
        let mut dirs: Vec<PathBuf> = self.children.keys().cloned().collect();
        if !dirs.contains(&self.root) {
            dirs.push(self.root.clone());
        }
        for dir in dirs {
            let entries = list_dir(&dir, show_hidden);
            self.children.insert(dir, entries);
        }
    }

    /// Every surface in every tab.
    pub fn all_surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.tabs.iter().flat_map(|t| t.panes.values())
    }
}

/// `~` and symlinks resolved, as `Workspace.normalize` does.
pub fn normalize(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let expanded = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(format!("{}{rest}", home().display()))
        }
        _ => path.to_path_buf(),
    };
    std::fs::canonicalize(&expanded).unwrap_or(expanded)
}

pub fn home() -> PathBuf {
    dirs_home().unwrap_or_else(|| PathBuf::from("/"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Where Scratch starts: the Scratch folder setting when it names a
/// folder, else home.
pub fn scratch_root() -> PathBuf {
    let setting = SCRATCH.with(|s| s.borrow().clone());
    let setting = setting.trim();
    if setting.is_empty() {
        return home();
    }
    let path = normalize(Path::new(setting));
    if path.is_dir() {
        path
    } else {
        home()
    }
}

thread_local! {
    /// `scratch_directory`, kept here so models without an `App` can read it.
    pub static SCRATCH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn list_dir(dir: &Path, show_hidden: bool) -> Vec<(String, bool)> {
    let mut entries: Vec<(String, bool)> =
        impulse_kit::directory_lister::read_directory_entries(&dir.to_string_lossy(), show_hidden)
            .map(|entries| entries.into_iter().map(|e| (e.name, e.is_dir)).collect())
            .unwrap_or_default();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    entries
}

pub fn git_branch(dir: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
