//! A window's workspaces, their tabs and the panes in a tab, as the Swift
//! app's `Workspace`, `TabEntry` and `SplitTab` hold them: a workspace is a
//! folder (or Scratch, which follows the home folder or the Scratch folder
//! setting) with its own tabs; a tab is one surface or a split of several,
//! laid out by an `impulse_kit::layout_tree::LayoutTree` of pane ids.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::{App, Entity, Window};
use impulse_kit::layout_tree::{LayoutTree, SplitAxis};

use crate::editor_tab::EditorTab;
use crate::history_view::HistoryView;
use crate::image_view::ImageView;
use crate::keybindings_view::KeybindingsView;
use crate::lsp_hub::LspHub;
use crate::problems_view::ProblemsView;
use crate::review_view::ReviewView;
use crate::settings_view::SettingsView;
use crate::terminal_view::TerminalView;

pub type PaneId = u64;

/// What a pane shows.
#[derive(Clone)]
pub enum Surface {
    Terminal(Entity<TerminalView>),
    Editor(Entity<EditorTab>),
    Review(Entity<ReviewView>),
    History(Entity<HistoryView>),
    Image(Entity<ImageView>),
    /// The Settings and Keyboard Shortcuts tools (one of each per workspace).
    Settings(Entity<SettingsView>),
    Keybindings(Entity<KeybindingsView>),
    /// A task's Finish tab (`task_finish`).
    TaskFinish(Entity<crate::task_finish::TaskFinishView>),
    /// The window's language-server diagnostics (one per workspace).
    Problems(Entity<ProblemsView>),
    /// Project Setup (`project_setup`), one per workspace.
    ProjectSetup(Entity<crate::project_setup::ProjectSetupView>),
}

impl Surface {
    /// The tab strip's title: the name, with " *" while there are unsaved
    /// changes (Swift's `TabEntry.title`).
    pub fn title(&self, cx: &App) -> String {
        let name = self.name(cx);
        if self.is_dirty(cx) {
            format!("{name} *")
        } else {
            name
        }
    }

    /// A terminal's title (or its shell), a file's name, a tool's name.
    pub fn name(&self, cx: &App) -> String {
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
            Surface::Review(r) => r.read(cx).title(),
            Surface::History(h) => h.read(cx).title(),
            Surface::Image(i) => i.read(cx).title(),
            Surface::Settings(_) => "Settings".into(),
            Surface::Keybindings(_) => "Keyboard Shortcuts".into(),
            Surface::TaskFinish(f) => f.read(cx).title(),
            Surface::Problems(_) => "Problems".into(),
            Surface::ProjectSetup(_) => "Project Setup".into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Surface::Terminal(_) => "terminal",
            Surface::Editor(_) => "file-code",
            Surface::Review(_) => "split",
            Surface::History(_) => "history",
            Surface::Image(_) => "file",
            Surface::Settings(_) => "settings",
            Surface::Keybindings(_) => "keyboard",
            // Impulse's own (`resources::own_icon`): Lucide has no
            // checkered flag, SF Symbols' `flag.checkered` in Swift.
            Surface::TaskFinish(_) => "flag-checkered",
            Surface::Problems(_) => "triangle-alert",
            Surface::ProjectSetup(_) => "sliders-horizontal",
        }
    }

    /// The file's own icon (material, by name) for editors and images.
    pub fn file_icon(&self, cx: &App) -> Option<String> {
        let path = match self {
            Surface::Editor(e) => e.read(cx).path.clone(),
            Surface::Image(i) => i.read(cx).path.clone(),
            _ => return None,
        };
        let name = path.file_name()?.to_string_lossy().into_owned();
        Some(crate::file_icons::icon_path(&name, false, false))
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Surface::Editor(e) => e.read(cx).dirty,
            Surface::ProjectSetup(s) => s.read(cx).is_modified(),
            _ => false,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            // The input bar when it's showing, otherwise the grid.
            Surface::Terminal(t) => t.update(cx, |t, cx| t.focus(window, cx)),
            Surface::Editor(e) => e.update(cx, |e, cx| e.focus(window, cx)),
            Surface::Review(r) => {
                let handle = r.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            Surface::History(h) => {
                let handle = h.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            Surface::Image(i) => {
                let handle = i.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            Surface::Settings(s) => s.update(cx, |s, cx| s.focus(window, cx)),
            Surface::Keybindings(k) => k.update(cx, |k, cx| k.focus(window, cx)),
            Surface::TaskFinish(f) => {
                let handle = f.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
            }
            Surface::Problems(p) => p.update(cx, |p, cx| p.focus(window, cx)),
            Surface::ProjectSetup(s) => {
                let handle = s.read(cx).focus.clone();
                window.focus(&handle, cx);
            }
        }
    }

    /// The folder it's in: a terminal's working directory, a file's folder.
    pub fn directory(&self, cx: &App) -> Option<PathBuf> {
        match self {
            Surface::Terminal(t) => Some(PathBuf::from(&t.read(cx).cwd)),
            Surface::Editor(e) => e.read(cx).path.parent().map(Path::to_path_buf),
            Surface::Review(r) => Some(PathBuf::from(&r.read(cx).root)),
            Surface::TaskFinish(f) => Some(PathBuf::from(&f.read(cx).root)),
            Surface::ProjectSetup(s) => Some(PathBuf::from(&s.read(cx).model.root)),
            Surface::History(h) => Some(PathBuf::from(&h.read(cx).root)),
            Surface::Image(i) => i.read(cx).path.parent().map(Path::to_path_buf),
            Surface::Settings(_) | Surface::Keybindings(_) | Surface::Problems(_) => None,
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
    /// A file shown from a single click in the file tree: the next one
    /// replaces it, until it's edited, split or double-clicked.
    pub preview: bool,
    /// Shown as its icon only, and closing it asks first.
    pub pinned: bool,
    /// The tab that was active when this one opened: closing this one goes
    /// back there (Swift's tab manager).
    pub opener: Option<u64>,
}

impl Tab {
    pub fn new(id: u64, pane: PaneId, surface: Surface) -> Self {
        Tab {
            id,
            layout: LayoutTree::Leaf(pane),
            panes: HashMap::from([(pane, surface)]),
            focused: pane,
            zoomed: None,
            preview: false,
            pinned: false,
            opener: None,
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
        self.preview = false;
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
    /// Its repository, whether it's a task, and the task's states.
    pub task: crate::tasks::TaskInfo,
    /// TCP ports its terminals' processes listen on (`port_scan`).
    pub ports: Vec<impulse_kit::listening_ports::ListeningPort>,
}

impl Workspace {
    pub fn new(id: u64, kind: Kind, root: PathBuf) -> Self {
        let root = match kind {
            Kind::Scratch => scratch_root(),
            Kind::Folder => normalize(&root),
        };
        let task = crate::tasks::TaskInfo::read(kind, &root);
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
            task,
            ports: Vec::new(),
        };
        workspace.refresh_branch();
        // The folders left open in this project last time.
        if kind == Kind::Folder {
            for dir in impulse_kit::tree_expansion::load(&expansion_file(), &workspace.root) {
                workspace.children.insert(dir.clone(), Vec::new());
                workspace.expanded_dirs.insert(dir);
            }
        }
        workspace.reload_tree(false);
        workspace
    }

    /// Remember the file tree's open folders for next time (the Swift
    /// app's per-root expanded paths).
    pub fn save_expanded(&self) {
        if self.kind == Kind::Folder {
            impulse_kit::tree_expansion::save(&expansion_file(), &self.root, &self.expanded_dirs);
        }
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
    pub fn reload_tree(&mut self, show_hidden: bool) {
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

/// Where the file tree's open folders are kept, per project.
fn expansion_file() -> PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("file-tree-expanded.json")
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
    impulse_git::cli::run(
        &["rev-parse", "--abbrev-ref", "HEAD"],
        &dir.to_string_lossy(),
        impulse_git::cli::RunOptions {
            timeout: Some(std::time::Duration::from_secs(5)),
            ..Default::default()
        },
    )
    .map(|o| o.stdout.trim().to_string())
    .unwrap_or_default()
}
