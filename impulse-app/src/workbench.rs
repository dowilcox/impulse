//! The window: its workspaces, their tabs and panes, and every command
//! (`RunCommand`) run against them, as `MainWindowController` and
//! `TabManager` do in the Swift app. Drawing is in `workbench_chrome`
//! (titlebar, sidebar, status bar) and `workbench_panes` (a tab's split
//! panes); trust and language servers in `workbench_trust`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{
    actions, div, prelude::*, px, App, Context, Entity, FocusHandle, KeyBinding, Window,
};
use impulse_kit::chrome::ChromePalette;
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

actions!(impulse, [FormatDocument, RenameSymbol, FindReferences]);

/// Monaco's keys for the editor commands that go through the workbench
/// (they reach other files).
pub fn bind_keys(cx: &mut App) {
    let c = Some(crate::editor_keys::CONTEXT);
    cx.bind_keys([
        KeyBinding::new("alt-shift-f", FormatDocument, c),
        KeyBinding::new("f2", RenameSymbol, c),
        KeyBinding::new("shift-f12", FindReferences, c),
    ]);
    crate::changes_panel::bind_keys(cx);
}

/// A closed tab that Reopen Closed Tab brings back: files reopen, terminals
/// get a new shell in the same folder.
enum Closed {
    File(PathBuf),
    Terminal(PathBuf),
}

/// What the sidebar shows below the workspaces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarPanel {
    Files,
    Changes,
    Search,
}

pub struct Workbench {
    pub(crate) palette: ChromePalette,
    pub(crate) theme: ResolvedTheme,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) active: usize,
    next_id: u64,
    pub(crate) focus: FocusHandle,
    /// A message for the status bar, such as why a file didn't open.
    pub(crate) notice: Option<String>,
    /// The trusted folders (shared by every window).
    pub(crate) trust: Arc<WorkspaceTrust>,
    pub(crate) panel: SidebarPanel,
    closed: Vec<Closed>,
    pub(crate) drag: Option<crate::workbench_panes::DividerDrag>,
    pub(crate) split_bounds: crate::workbench_panes::SplitBounds,
    /// `--theme`, which wins over `color_scheme` for this run.
    theme_override: Option<String>,
    theme_id: String,
    _settings: gpui_kit::Subscription,
    /// Repositories and the Changes panel (`changes_panel`, `git_actions`).
    pub(crate) git: crate::changes_panel::GitPanel,
}

impl Workbench {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = SettingsStore::get(cx).clone();
        crate::workspace::SCRATCH.with(|s| *s.borrow_mut() = settings.scratch_directory.clone());
        let theme_id = args
            .theme
            .clone()
            .unwrap_or_else(|| settings.color_scheme.clone());
        let theme = impulse_kit::theme::get_theme(&theme_id);
        let palette = ChromePalette::new(&theme, false);
        crate::kit_theme::apply(&theme, &palette, &settings, cx);
        let settings_subscription = cx
            .observe_global_in::<SettingsStore>(window, |workbench, window, cx| {
                workbench.apply_settings(window, cx)
            });
        let mut workbench = Self {
            palette,
            theme,
            workspaces: Vec::new(),
            active: 0,
            next_id: 1,
            focus: cx.focus_handle(),
            notice: None,
            trust: args.trust_store.clone(),
            panel: SidebarPanel::Files,
            closed: Vec::new(),
            drag: None,
            split_bounds: Default::default(),
            theme_override: args.theme.clone(),
            theme_id,
            _settings: settings_subscription,
            git: crate::changes_panel::GitPanel::new(window, cx),
        };
        let cwd = crate::workspace::normalize(Path::new(&args.cwd));
        if args.trust {
            workbench.trust.trust(&cwd.to_string_lossy());
        }
        // A folder workspace for the folder Impulse started in (unless
        // that's home), and Scratch.
        let home = crate::workspace::home();
        if cwd != home {
            let id = workbench.next_id();
            workbench
                .workspaces
                .push(Workspace::new(id, Kind::Folder, cwd.clone()));
        }
        let id = workbench.next_id();
        workbench
            .workspaces
            .push(Workspace::new(id, Kind::Scratch, home));
        let dir = workbench.workspaces[0].default_directory();
        let terminal = workbench.new_terminal(dir, args.commands.clone(), window, cx);
        workbench.add_tab(0, terminal, window, cx);
        for path in &args.open {
            let path = cwd.join(path);
            workbench.open_file(&path, window, cx);
        }
        // Once the window (and its dialog layer) exists: language servers if
        // the folder is trusted, otherwise ask.
        cx.defer_in(window, |workbench, window, cx| {
            workbench.request_trust(workbench.active, window, cx)
        });
        workbench
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

    /// Settings changed: the theme, the editor's and terminals' fonts.
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsStore::get(cx).clone();
        crate::workspace::SCRATCH.with(|s| *s.borrow_mut() = settings.scratch_directory.clone());
        let theme_id = self
            .theme_override
            .clone()
            .unwrap_or_else(|| settings.color_scheme.clone());
        if theme_id != self.theme_id {
            self.theme_id = theme_id.clone();
            self.theme = impulse_kit::theme::get_theme(&theme_id);
            self.palette = ChromePalette::new(&self.theme, false);
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
        let (theme, palette) = (self.theme.clone(), self.palette.clone());
        let terminal = cx.new(|cx| {
            TerminalView::new(
                theme,
                palette,
                dir.to_string_lossy().into_owned(),
                commands,
                window,
                cx,
            )
        });
        cx.observe(&terminal, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            &terminal,
            window,
            |workbench, terminal, event, window, cx| {
                if let TerminalViewEvent::Exited(_) = event {
                    // The shell exited: close its pane, as Terminal.app does.
                    let surface = Surface::Terminal(terminal.clone());
                    workbench.close_surface(&surface, window, cx);
                }
            },
        )
        .detach();
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
        // After the active tab, as the Swift app inserts them.
        let at = if workspace.tabs.is_empty() {
            0
        } else {
            workspace.active + 1
        };
        workspace.tabs.insert(at, Tab::new(tab_id, pane, surface));
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
        self.active = index;
        self.workspaces[index].refresh_branch();
        if self.workspaces[index].tabs.is_empty() {
            let dir = self.workspaces[index].default_directory();
            let terminal = self.new_terminal(dir, Vec::new(), window, cx);
            self.add_tab(index, terminal, window, cx);
        }
        self.focus_active(window, cx);
        self.request_trust(index, window, cx);
        cx.notify();
    }

    /// Open `path` in an editor tab, or go to the tab it's already in.
    pub(crate) fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((w, t, pane)) = self.find_editor(path, cx) {
            self.active = w;
            self.workspaces[w].active = t;
            self.workspaces[w].tabs[t].focused = pane;
            self.focus_active(window, cx);
            cx.notify();
            return;
        }
        match impulse_kit::text_file::open(path)
            .map(|opened| cx.new(|cx| EditorTab::new(path, opened, window, cx)))
        {
            Ok(editor) => {
                let index = self.active;
                self.start_lsp(index, &editor, window, cx);
                cx.subscribe(&editor, |_, _, _: &EditorTabChanged, cx| cx.notify())
                    .detach();
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                self.notice = None;
                self.add_tab(index, Surface::Editor(editor), window, cx);
            }
            Err(refusal) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.set_notice(refusal.message(&name), cx);
            }
        }
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

    /// Close the focused pane (the tab with it when it's the only one). An
    /// editor with unsaved changes needs a second close, as a warning.
    pub(crate) fn close_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(surface) = self.focused_surface().cloned() else {
            return;
        };
        if surface.is_dirty(cx) {
            let warning = format!(
                "{} has unsaved changes. Close again to discard them.",
                surface.title(cx)
            );
            if self.notice.as_deref() != Some(warning.as_str()) {
                self.set_notice(warning, cx);
                return;
            }
        }
        self.close_surface(&surface, window, cx);
    }

    /// Close the tab at `index` of the active workspace, every pane in it.
    pub(crate) fn close_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace().tabs.get(index) else {
            return;
        };
        let dirty: Vec<String> = tab
            .panes
            .values()
            .filter(|s| s.is_dirty(cx))
            .map(|s| s.title(cx))
            .collect();
        if !dirty.is_empty() {
            let warning = format!(
                "{} has unsaved changes. Close again to discard them.",
                dirty.join(", ")
            );
            if self.notice.as_deref() != Some(warning.as_str()) {
                self.set_notice(warning, cx);
                return;
            }
        }
        let tab = self.workspace_mut().tabs.remove(index);
        for surface in tab.panes.values() {
            self.remember_closed(surface, cx);
        }
        self.after_tab_removed(index, window, cx);
    }

    fn remember_closed(&mut self, surface: &Surface, cx: &App) {
        self.closed.push(match surface {
            Surface::Editor(e) => Closed::File(e.read(cx).path.clone()),
            Surface::Terminal(t) => Closed::Terminal(PathBuf::from(&t.read(cx).cwd)),
        });
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
                self.remember_closed(surface, cx);
                if !self.workspaces[w].tabs[t].remove(pane) {
                    self.workspaces[w].tabs.remove(t);
                    let previous = self.active;
                    self.active = w;
                    self.after_tab_removed(t, window, cx);
                    if previous != w && previous < self.workspaces.len() {
                        self.active = previous;
                        self.focus_active(window, cx);
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
    fn after_tab_removed(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace_mut();
        if workspace.tabs.is_empty() {
            let dir = workspace.default_directory();
            let terminal = self.new_terminal(dir, Vec::new(), window, cx);
            let active = self.active;
            self.add_tab(active, terminal, window, cx);
            return;
        }
        if workspace.active >= index && workspace.active > 0 {
            workspace.active -= 1;
        }
        workspace.active = workspace.active.min(workspace.tabs.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// A message in the status bar.
    pub(crate) fn set_notice(&mut self, notice: String, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }

    /// Read every listed folder again (files were created, moved or deleted).
    pub(crate) fn reload_tree(&mut self, cx: &mut Context<Self>) {
        for workspace in &mut self.workspaces {
            workspace.reload_tree();
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
            "reopen_tab" => match self.closed.pop() {
                Some(Closed::File(path)) => self.open_file(&path, window, cx),
                Some(Closed::Terminal(dir)) => {
                    let terminal = self.new_terminal(dir, Vec::new(), window, cx);
                    let active = self.active;
                    self.add_tab(active, terminal, window, cx);
                }
                None => {}
            },
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
                    editor.update(cx, |editor, cx| editor.save(cx));
                }
            }
            "toggle_sidebar" => {
                SettingsStore::update(cx, |s| s.sidebar_visible = !s.sidebar_visible)
            }
            "show_files" | "show_changes" | "project_search" | "search_text" => {
                self.panel = match id {
                    "show_files" => SidebarPanel::Files,
                    "show_changes" => SidebarPanel::Changes,
                    _ => SidebarPanel::Search,
                };
                SettingsStore::update(cx, |s| s.sidebar_visible = true);
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
            "trust_folder" => {
                let index = self.active;
                self.workspaces[index].declined = false;
                self.present_trust_prompt(index, window, cx);
            }
            "restrict_folder" => {
                let root = self.workspace().root.to_string_lossy().into_owned();
                self.trust.restrict(&root);
                self.disable_lsp(self.active, cx);
                let name = self.workspace().name();
                self.set_notice(
                    format!("{name} is restricted: language servers are off."),
                    cx,
                );
            }
            "forget_trusted_folders" => {
                self.trust.revoke_all();
                for index in 0..self.workspaces.len() {
                    self.disable_lsp(index, cx);
                }
                self.set_notice("Every folder is restricted again.".into(), cx);
            }
            "open_settings_json" => {
                let path = impulse_kit::settings::settings_path(cfg!(debug_assertions));
                if !path.exists() {
                    let defaults = impulse_kit::settings::Settings::default().encode();
                    let _ = std::fs::write(&path, defaults);
                }
                self.open_file(&path, window, cx);
            }
            "open_workspace" => self.choose_workspace_folder(window, cx),
            "close_workspace" => self.close_workspace(self.active, window, cx),
            "new_window" => {
                let args = Args {
                    theme: self.theme_override.clone(),
                    cwd: self.workspace().root.to_string_lossy().into_owned(),
                    commands: Vec::new(),
                    open: Vec::new(),
                    trust: false,
                    trust_store: self.trust.clone(),
                };
                crate::open_main_window(args, cx);
            }
            git if crate::git_actions::GIT_COMMANDS.contains(&git) => {
                self.run_git_command(git, window, cx)
            }
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

    /// A custom shortcut: its command line in the focused terminal, or in a
    /// new terminal tab when none is focused.
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
        match self.active_terminal() {
            Some(terminal) => terminal.read(cx).write(format!("{line}\r").as_bytes()),
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
                self.workspaces
                    .insert(at, Workspace::new(id, Kind::Folder, folder));
                at
            }
        };
        self.activate_workspace(index, window, cx);
    }

    /// Close a folder workspace and its tabs (Scratch stays).
    pub(crate) fn close_workspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspaces
            .get(index)
            .is_none_or(|w| w.kind == Kind::Scratch)
        {
            return;
        }
        let dirty = self.workspaces[index]
            .all_surfaces()
            .any(|s| s.is_dirty(cx));
        let warning = format!(
            "{} has unsaved files. Close it again to discard them.",
            self.workspaces[index].name()
        );
        if dirty && self.notice.as_deref() != Some(warning.as_str()) {
            self.set_notice(warning, cx);
            return;
        }
        self.workspaces.remove(index);
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        let active = self.active.min(self.workspaces.len() - 1);
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
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_git(cx);
        let sidebar = SettingsStore::get(cx).sidebar_visible;
        let p = &self.palette;
        div()
            .id("workbench")
            .track_focus(&self.focus)
            .key_context("Workbench")
            .on_action(cx.listener(Self::run_command_action))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .on_mouse_move(cx.listener(Self::drag_divider))
            .on_mouse_up(gpui_kit::MouseButton::Left, cx.listener(Self::end_drag))
            .flex()
            .flex_col()
            .size_full()
            .bg(crate::workbench_chrome::hsla(p.window))
            .font_family(UI_FONT)
            .text_size(px(13.))
            .text_color(crate::workbench_chrome::hsla(p.text))
            .child(self.titlebar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(sidebar, |row| row.child(self.sidebar(cx)))
                    .child(self.content(window, cx)),
            )
            .child(self.status_bar(cx))
    }
}
