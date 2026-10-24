//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace and tabs, the left dock (workspaces,
//! then files), the content pane with the terminal's context chips and
//! input bar, and the 24pt status bar. Sizes come from
//! `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    actions, div, prelude::*, px, svg, AnyElement, App, Context, Entity, FocusHandle, FontWeight,
    Hsla, KeyBinding, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};
use impulse_kit::trust::WorkspaceTrust;

use crate::editor_tab::{ApplyEdit, EditorHooks, EditorTab, EditorTabChanged, ShowDocument};
use crate::lsp_hub::LspHub;
use crate::settings_store::SettingsStore;
use crate::terminal_view::{rgba, TerminalView};
use crate::Args;

const UI_FONT: &str = "Inter";

actions!(
    impulse,
    [Save, CloseTab, FormatDocument, RenameSymbol, FindReferences]
);

/// Keys for the workbench's commands: `secondary` is Cmd on macOS and Ctrl
/// on Linux.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        // Monaco's Format Document.
        KeyBinding::new("alt-shift-f", FormatDocument, None),
        KeyBinding::new("f2", RenameSymbol, None),
        KeyBinding::new("shift-f12", FindReferences, None),
    ]);
}

pub(crate) enum Tab {
    /// The workspace's terminal (`Workbench::terminal`).
    Terminal,
    Editor(Entity<EditorTab>),
}

/// One row of the file tree.
struct TreeRow {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

pub struct Workbench {
    palette: ChromePalette,
    terminal: Entity<TerminalView>,
    pub(crate) tabs: Vec<Tab>,
    active: usize,
    focus: FocusHandle,
    root: PathBuf,
    workspace: String,
    branch: String,
    /// Each listed folder's entries (folders first, then by name), read
    /// when the folder is first expanded.
    children: HashMap<PathBuf, Vec<(String, bool)>>,
    expanded: HashSet<PathBuf>,
    /// A message for the status bar, such as why a file didn't open.
    notice: Option<String>,
    /// The trusted folders (shared by every window).
    pub(crate) trust: Arc<WorkspaceTrust>,
    /// The user kept this folder restricted this session: don't ask again.
    declined: bool,
    /// Language servers, when the folder is trusted.
    lsp: Option<std::rc::Rc<LspHub>>,
    _lsp_events: Option<gpui_kit::Task<()>>,
    /// `--theme`, which wins over `color_scheme` for this run.
    theme_override: Option<String>,
    /// The theme the window shows.
    theme_id: String,
    _settings: gpui_kit::Subscription,
}

/// Not the home folder or the disk: trusting those trusts everything.
fn offers_parent(parent: &str) -> bool {
    let normalized = impulse_kit::trust::normalize(parent);
    let home = dirs_home();
    !["/", "/Users", "/Volumes", "/home", home.as_str()].contains(&normalized.as_str())
}

fn dirs_home() -> String {
    impulse_kit::trust::normalize(&std::env::var("HOME").unwrap_or_default())
}

/// `~/…` for paths in the home folder.
fn abbreviate_home(path: &str) -> String {
    let home = dirs_home();
    match path.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

fn list_dir(dir: &Path) -> Vec<(String, bool)> {
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        e.path().is_dir(),
                    )
                })
                .filter(|(name, _)| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    entries
}

fn git(cwd: &str, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

impl Workbench {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = SettingsStore::get(cx).clone();
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
        let terminal = cx.new(|cx| {
            TerminalView::new(
                theme.clone(),
                palette.clone(),
                args.cwd.clone(),
                args.commands.clone(),
                window,
                cx,
            )
        });
        cx.observe(&terminal, |_, _, cx| cx.notify()).detach();

        let root = PathBuf::from(&args.cwd);
        let focus = cx.focus_handle();
        let terminal_focus = terminal.read(cx).focus_handle().clone();
        window.focus(&terminal_focus, cx);
        let mut workbench = Self {
            palette,
            tabs: vec![Tab::Terminal],
            terminal,
            active: 0,
            focus,
            workspace: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            branch: git(&args.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]),
            children: HashMap::from([(root.clone(), list_dir(&root))]),
            expanded: HashSet::new(),
            root: root.clone(),
            notice: None,
            trust: args.trust_store.clone(),
            declined: false,
            lsp: None,
            _lsp_events: None,
            theme_override: args.theme.clone(),
            theme_id,
            _settings: settings_subscription,
        };
        if args.trust {
            workbench.trust.trust(&root.to_string_lossy());
        }
        for path in &args.open {
            let path = workbench.root.join(path);
            workbench.open_file(&path, window, cx);
        }
        // Once the window (and its dialog layer) exists: language servers if
        // the folder is trusted, otherwise ask.
        cx.defer_in(window, |workbench, window, cx| {
            workbench.request_trust(window, cx)
        });
        workbench
    }

    /// Settings changed: the theme, the editor's font and options.
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsStore::get(cx).clone();
        let theme_id = self
            .theme_override
            .clone()
            .unwrap_or_else(|| settings.color_scheme.clone());
        let theme = impulse_kit::theme::get_theme(&theme_id);
        if theme_id != self.theme_id {
            self.theme_id = theme_id;
            self.palette = ChromePalette::new(&theme, false);
        }
        crate::kit_theme::apply(&theme, &self.palette, &settings, cx);
        let palette = self.palette.clone();
        self.terminal
            .update(cx, |terminal, cx| terminal.set_theme(theme.clone(), palette, window, cx));
        for tab in &self.tabs {
            if let Tab::Editor(editor) = tab {
                editor.update(cx, |editor, cx| {
                    editor.apply_settings(&settings, window, cx)
                });
            }
        }
        cx.notify();
    }

    /// Open `path` in an editor tab, or go to the tab it's already in.
    fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.tabs.iter().position(|tab| match tab {
            Tab::Editor(editor) => editor.read(cx).path == path,
            Tab::Terminal => false,
        });
        if let Some(index) = existing {
            self.activate(index, window, cx);
            return;
        }
        match impulse_kit::text_file::open(path)
            .map(|opened| cx.new(|cx| EditorTab::new(path, opened, window, cx)))
        {
            Ok(editor) => {
                self.start_lsp(&editor, window, cx);
                cx.subscribe(&editor, |_, _, _: &EditorTabChanged, cx| cx.notify())
                    .detach();
                cx.observe(&editor, |_, _, cx| cx.notify()).detach();
                self.tabs.push(Tab::Editor(editor));
                self.notice = None;
                self.activate(self.tabs.len() - 1, window, cx);
            }
            Err(refusal) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.notice = Some(refusal.message(&name));
                cx.notify();
            }
        }
    }

    /// The folder is trusted: run language servers for it, else ask once
    /// (the Swift app's `requestTrustIfNeeded`).
    fn request_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.to_string_lossy().into_owned();
        if self.trust.is_trusted(&root) {
            self.enable_lsp(window, cx);
        } else if !self.declined {
            self.present_trust_prompt(window, cx);
        }
    }

    /// "Do you trust the files in …?", as the Swift app asks it.
    fn present_trust_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = impulse_kit::trust::normalize(&self.root.to_string_lossy());
        let name = Path::new(&folder)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.clone());
        let parent = Path::new(&folder)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let offer_parent = offers_parent(&parent);
        let trust_parent = std::rc::Rc::new(std::cell::Cell::new(false));
        let workbench = cx.entity().downgrade();
        let shown_folder = abbreviate_home(&folder);
        let shown_parent = abbreviate_home(&parent);
        window.open_dialog(cx, move |dialog, _, _| {
            let (trust_workbench, stay_workbench) = (workbench.clone(), workbench.clone());
            let (folder, parent, trust_parent) =
                (folder.clone(), parent.clone(), trust_parent.clone());
            let checkbox_value = trust_parent.clone();
            dialog
                .title(format!("Do you trust the files in “{name}”?"))
                .w(px(480.))
                .keyboard(false)
                .overlay_closable(false)
                .close_button(false)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            "Impulse can run code from this folder on its own: language servers \
                             (some run a project's build scripts and tools), formatters and \
                             commands on save, and background git fetch. Until you trust it, \
                             they stay off. The terminal and editing work as usual.",
                        )
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(12.))
                                .child(shown_folder.clone()),
                        )
                        .children(offer_parent.then(|| {
                            Checkbox::new("trust-parent")
                                .label(format!("Trust everything in “{shown_parent}”"))
                                .checked(checkbox_value.get())
                                .on_change(move |checked, window, _| {
                                    checkbox_value.set(*checked);
                                    window.refresh();
                                })
                        })),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("stay-restricted")
                                .label("Stay Restricted")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = stay_workbench.update(cx, |workbench, cx| {
                                        workbench.declined = true;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("trust-folder")
                                .primary()
                                .label("Trust Folder")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = trust_workbench.update(cx, |workbench, cx| {
                                        // The folder itself first: trusting the parent
                                        // doesn't lift a restriction on a folder inside it.
                                        workbench.trust.trust(&folder);
                                        if trust_parent.get() {
                                            workbench.trust.trust(&parent);
                                        }
                                        workbench.declined = false;
                                        workbench.enable_lsp(window, cx);
                                    });
                                }),
                        ),
                )
        });
    }

    /// Language servers for this workspace, and for the files already open.
    fn enable_lsp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lsp.is_none() {
            let (hub, events) = LspHub::new(self.root.clone(), self.trust.clone());
            self.lsp = Some(hub);
            self._lsp_events = Some(cx.spawn(async move |workbench, cx| {
                while let Ok(event) = events.recv().await {
                    let alive =
                        workbench.update(cx, |workbench, cx| workbench.server_event(event, cx));
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        }
        let editors: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::Editor(editor) => Some(editor.clone()),
                Tab::Terminal => None,
            })
            .collect();
        for editor in editors {
            self.start_lsp(&editor, window, cx);
        }
        cx.notify();
    }

    /// Hand a tab its language server, if its file is in a trusted folder.
    fn start_lsp(
        &mut self,
        editor: &Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hub) = self.lsp.clone() else { return };
        let path = editor.read(cx).path.to_string_lossy().into_owned();
        if !self.trust.is_trusted(&path) {
            return;
        }
        let hooks = EditorHooks {
            show_document: self.show_document_handler(cx),
            apply_edit: self.apply_edit_handler(cx),
        };
        editor.update(cx, |editor, cx| editor.start_lsp(&hub, hooks, window, cx));
    }

    fn is_restricted(&self) -> bool {
        !self.trust.is_trusted(&self.root.to_string_lossy())
    }

    /// Go-to-definition into another file: open it in a tab at the target.
    /// Targets in the same file are left to the editor.
    fn apply_edit_handler(&self, cx: &mut Context<Self>) -> ApplyEdit {
        let workbench = cx.entity().downgrade();
        std::rc::Rc::new(move |pending, window, cx| {
            let _ = workbench.update(cx, |workbench, cx| {
                workbench.apply_workspace_edit(pending, window, cx)
            });
        })
    }

    fn show_document_handler(&self, cx: &mut Context<Self>) -> ShowDocument {
        let workbench = cx.entity().downgrade();
        std::rc::Rc::new(move |params, window, cx| {
            let path =
                std::path::PathBuf::from(impulse_kit::uri::uri_to_file_path(params.uri.as_str()));
            let Some(workbench) = workbench.upgrade() else {
                return false;
            };
            if workbench.read(cx).active_path(cx).as_deref() == Some(path.as_path())
                || !path.is_file()
            {
                return false;
            }
            let start = params.selection.map(|range| range.start);
            workbench.update(cx, |workbench, cx| {
                workbench.open_file(&path, window, cx);
                if let (Some(start), Some(editor)) = (start, workbench.active_editor().cloned()) {
                    editor.update(cx, |editor, cx| editor.go_to(start, window, cx));
                }
            });
            true
        })
    }

    fn active_path(&self, cx: &App) -> Option<PathBuf> {
        self.active_editor()
            .map(|editor| editor.read(cx).path.clone())
    }

    /// What a language server sent on its own.
    fn server_event(&mut self, event: impulse_lsp::ServerEvent, cx: &mut Context<Self>) {
        match event {
            impulse_lsp::ServerEvent::Diagnostics(server, params) => {
                log::debug!(
                    "diagnostics for {}: {:?}",
                    params.uri.as_str(),
                    params
                        .diagnostics
                        .iter()
                        .map(|d| (d.source.clone(), d.message.clone()))
                        .collect::<Vec<_>>()
                );
                for tab in &self.tabs {
                    if let Tab::Editor(editor) = tab {
                        if editor.read(cx).is_document(&params.uri) {
                            editor.update(cx, |editor, cx| {
                                editor.set_diagnostics(
                                    server.clone(),
                                    params.diagnostics.clone(),
                                    cx,
                                )
                            });
                        }
                    }
                }
            }
            impulse_lsp::ServerEvent::Message(message) => log::info!("language server: {message}"),
            impulse_lsp::ServerEvent::Exited => log::warn!("a language server exited"),
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        match &self.tabs[self.active] {
            Tab::Editor(editor) => editor.update(cx, |editor, cx| editor.focus(window, cx)),
            Tab::Terminal => {
                let focus = self.terminal.read(cx).focus_handle().clone();
                window.focus(&focus, cx)
            }
        }
        cx.notify();
    }

    fn active_editor(&self) -> Option<&Entity<EditorTab>> {
        match self.tabs.get(self.active) {
            Some(Tab::Editor(editor)) => Some(editor),
            _ => None,
        }
    }

    fn save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.save(cx));
        }
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

    /// Close the active tab. The terminal stays; an editor with unsaved
    /// changes asks first, by needing a second close.
    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, window, cx);
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Editor(editor)) = self.tabs.get(index) else {
            return;
        };
        let (dirty, title) = {
            let editor = editor.read(cx);
            (editor.dirty, editor.title.clone())
        };
        let warning = format!("{title} has unsaved changes. Close again to discard them.");
        if dirty && self.notice.as_deref() != Some(warning.as_str()) {
            self.notice = Some(warning);
            cx.notify();
            return;
        }
        self.notice = None;
        self.tabs.remove(index);
        let next = if self.active >= index {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.activate(next, window, cx);
    }

    /// A message in the status bar.
    pub(crate) fn set_notice(&mut self, notice: String, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }

    /// Read every listed folder again (files were created, moved or deleted).
    pub(crate) fn reload_tree(&mut self, cx: &mut Context<Self>) {
        for (dir, entries) in self.children.iter_mut() {
            *entries = list_dir(dir);
        }
        cx.notify();
    }

    fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.children
                .entry(path.clone())
                .or_insert_with(|| list_dir(&path));
            self.expanded.insert(path);
        }
        cx.notify();
    }

    /// The visible rows: the root's entries, and those of every expanded
    /// folder under them.
    fn tree_rows(&self) -> Vec<TreeRow> {
        fn walk(wb: &Workbench, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
            for (name, is_dir) in wb.children.get(dir).map(Vec::as_slice).unwrap_or_default() {
                let path = dir.join(name);
                let expanded = *is_dir && wb.expanded.contains(&path);
                rows.push(TreeRow {
                    path: path.clone(),
                    name: name.clone(),
                    is_dir: *is_dir,
                    depth,
                });
                if expanded {
                    walk(wb, &path, depth + 1, rows);
                }
            }
        }
        let mut rows = Vec::new();
        walk(self, &self.root, 0, &mut rows);
        rows
    }

    fn c(&self, color: KitRgba) -> Hsla {
        rgba(color).into()
    }

    fn icon(&self, name: &str, size: f32, color: KitRgba) -> impl IntoElement {
        svg()
            .path(format!("icons/{name}.svg"))
            .size(px(size))
            .flex_none()
            .text_color(self.c(color))
    }

    fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        // macOS draws its traffic lights in this inset; on Linux the window
        // controls are Impulse's own (client-side decorations).
        let leading = if cfg!(target_os = "macos") {
            metrics::TRAFFIC_LIGHT_INSET
        } else {
            12.0
        };
        let tab = |icon: &str, title: &str, active: bool| {
            let mut t = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::TAB_HEIGHT))
                .px(px(12.))
                .rounded(px(metrics::RADIUS))
                .text_color(self.c(if active { p.text } else { p.text_secondary }))
                .child(self.icon(
                    icon,
                    metrics::ICON,
                    if active { p.text } else { p.text_secondary },
                ))
                .child(title.to_string());
            if active {
                t = t
                    .bg(self.c(p.raised))
                    .border_b_2()
                    .border_color(self.c(p.accent));
            }
            t
        };
        div()
            .flex()
            .items_center()
            .h(px(metrics::TITLEBAR_HEIGHT))
            .pl(px(leading))
            .pr(px(10.))
            .gap(px(10.))
            .bg(self.c(p.chrome))
            .border_b_1()
            .border_color(self.c(p.hairline))
            .child(
                div()
                    .id("toggle-sidebar")
                    .cursor_pointer()
                    .on_click(|_, _, cx| {
                        SettingsStore::update(cx, |s| s.sidebar_visible = !s.sidebar_visible)
                    })
                    .child(self.icon("panel-left", metrics::ICON_LARGE, p.text_secondary)),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.workspace.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(self.c(p.text_secondary))
                    .child(self.icon("git-branch", metrics::ICON_SMALL, p.text_secondary))
                    .child(self.branch.clone()),
            )
            .child(div().w(px(8.)))
            .children(self.tabs.iter().enumerate().map(|(index, tab_kind)| {
                let active = index == self.active;
                let (icon, title, dirty) = match tab_kind {
                    Tab::Terminal => ("terminal", "bash".to_string(), false),
                    Tab::Editor(editor) => {
                        let editor = editor.read(cx);
                        ("file-code", editor.title.clone(), editor.dirty)
                    }
                };
                let mut t = tab(icon, &title, active).id(("tab", index)).on_click(
                    cx.listener(move |this, _, window, cx| this.activate(index, window, cx)),
                );
                if matches!(tab_kind, Tab::Editor(_)) {
                    // A dot while there are unsaved changes, otherwise a close button.
                    t = t.child(if dirty {
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(self.c(p.text_secondary))
                            .into_any_element()
                    } else {
                        div()
                            .id(("close", index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close(index, window, cx)
                            }))
                            .child(self.icon("x", metrics::ICON_SMALL, p.text_tertiary))
                            .into_any_element()
                    });
                }
                t
            }))
            .child(self.icon("plus", metrics::ICON, p.text_secondary))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .w(px(300.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .bg(self.c(p.overlay))
                    .border_1()
                    .border_color(self.c(p.hairline))
                    .text_color(self.c(p.text_tertiary))
                    .child(self.icon("search", metrics::ICON_SMALL, p.text_tertiary))
                    .child("Search or run a command")
                    .child(div().flex_1())
                    .child(if cfg!(target_os = "macos") {
                        "⇧⌘P"
                    } else {
                        "Ctrl+Shift+P"
                    }),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let row = |icon: &str, label: String, selected: bool, indent: f32| {
            let mut r = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(metrics::ROW_HEIGHT + 4.))
                .pl(px(indent))
                .pr(px(8.))
                .mx(px(6.))
                .rounded(px(metrics::RADIUS))
                .child(self.icon(icon, metrics::ICON, p.text_secondary))
                .child(label);
            if selected {
                r = r.bg(self.c(p.selection)).font_weight(FontWeight::SEMIBOLD);
            }
            r
        };
        let open_path = self.active_editor().map(|e| e.read(cx).path.clone());
        let mut files = div()
            .id("files")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(px(4.));
        for (index, row) in self.tree_rows().into_iter().enumerate() {
            let expanded = self.expanded.contains(&row.path);
            let selected = open_path.as_deref() == Some(row.path.as_path());
            let path = row.path.clone();
            let is_dir = row.is_dir;
            let mut r = div()
                .id(("file", index))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::ROW_HEIGHT))
                .pl(px(10. + 14. * row.depth as f32))
                .pr(px(10.))
                .hover(|style| style.bg(rgba(p.hover)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if is_dir {
                        this.toggle_dir(path.clone(), cx);
                    } else {
                        this.open_file(&path, window, cx);
                    }
                }))
                .child(div().w(px(12.)).child(if row.is_dir {
                    self.icon(
                        if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        metrics::ICON_SMALL,
                        p.text_tertiary,
                    )
                    .into_any_element()
                } else {
                    div().into_any_element()
                }))
                .child(self.icon(
                    match (row.is_dir, expanded) {
                        (true, true) => "folder-open",
                        (true, false) => "folder",
                        _ => "file",
                    },
                    metrics::ICON,
                    if row.is_dir { p.info } else { p.text_secondary },
                ))
                .child(row.name);
            if selected {
                r = r.bg(self.c(p.selection));
            }
            files = files.child(r);
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SettingsStore::get(cx).sidebar_width as f32))
            .h_full()
            .bg(self.c(p.panel))
            .border_r_1()
            .border_color(self.c(p.hairline))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .py(px(8.))
                    .gap(px(2.))
                    .border_b_1()
                    .border_color(self.c(p.hairline))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(14.))
                            .h(px(metrics::ROW_HEIGHT))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(self.c(p.text_secondary))
                            .child(self.icon("folder-git-2", metrics::ICON_SMALL, p.text_secondary))
                            .child(self.workspace.clone()),
                    )
                    .child(row("folder-git-2", self.workspace.clone(), true, 18.))
                    .child(row("terminal", "Scratch".into(), false, 10.)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(metrics::PANE_HEADER_HEIGHT + 6.))
                    .px(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .h(px(24.))
                            .rounded(px(metrics::RADIUS))
                            .bg(self.c(p.raised))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.icon("folder-tree", metrics::ICON, p.text))
                            .child("Files"),
                    )
                    .child(self.icon("git-branch", metrics::ICON, p.text_tertiary))
                    .child(self.icon("search", metrics::ICON, p.text_tertiary)),
            )
            .child(files)
    }

    fn chip(&self, icon: &str, text: String, color: KitRgba) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(8.))
            .rounded(px(metrics::RADIUS))
            .bg(self.c(p.raised))
            .font_family("JetBrains Mono")
            .text_size(px(12.))
            .text_color(self.c(color))
            .child(self.icon(icon, metrics::ICON_SMALL, p.text_secondary))
            .child(text)
            .into_any_element()
    }

    fn content(&self, cx: &Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let terminal = self.terminal.read(cx);
        let home = std::env::var("HOME").unwrap_or_default();
        let cwd = match terminal.cwd.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => terminal.cwd.clone(),
        };
        let mut chips = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.chip("terminal", "bash".into(), p.text))
            .child(self.chip("folder", cwd, p.text))
            .child(self.chip("git-branch", self.branch.clone(), p.text));
        if let Some(ms) = terminal.last_duration_ms {
            chips = chips.child(self.chip("history", format!("{ms}ms"), p.success));
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.terminal.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(10.))
                    .bg(self.c(p.content))
                    .child(chips),
            )
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let item = |icon: &str, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(self.icon(icon, metrics::ICON_SMALL, p.text_secondary))
                .child(text)
        };
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .h(px(metrics::STATUS_BAR_HEIGHT))
            .px(px(12.))
            .bg(self.c(p.chrome))
            .border_t_1()
            .border_color(self.c(p.hairline))
            .text_size(px(12.))
            .text_color(self.c(p.text_secondary))
            .child(item("git-branch", self.branch.clone()))
            .children(self.is_restricted().then(|| {
                div()
                    .id("restricted")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(self.c(p.warning))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.present_trust_prompt(window, cx)),
                    )
                    .child(self.icon("shield-alert", metrics::ICON_SMALL, p.warning))
                    .child("Restricted")
            }))
            .children(
                self.notice
                    .clone()
                    .map(|notice| div().text_color(self.c(p.warning)).child(notice)),
            )
            .child(div().flex_1())
            .children(match self.active_editor() {
                Some(editor) => {
                    let editor = editor.read(cx);
                    let (line, column) = editor.cursor(cx);
                    vec![
                        editor.notice.clone().map(|n| {
                            div()
                                .text_color(self.c(p.warning))
                                .child(n)
                                .into_any_element()
                        }),
                        Some(
                            div()
                                .child(format!("Ln {line}, Col {column}"))
                                .into_any_element(),
                        ),
                        Some(
                            div()
                                .child(format!("Spaces: {}", editor.tab_size))
                                .into_any_element(),
                        ),
                        Some(div().child("UTF-8").into_any_element()),
                        Some(div().child(editor.language.clone()).into_any_element()),
                        Some(editor.server_names())
                            .filter(|names| !names.is_empty())
                            .map(|names| {
                                div()
                                    .text_color(self.c(p.text_tertiary))
                                    .child(names.join(", "))
                                    .into_any_element()
                            }),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                }
                None => vec![
                    item("folder", self.terminal.read(cx).cwd.clone()).into_any_element(),
                    item("terminal", "bash".into()).into_any_element(),
                ],
            })
    }
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = SettingsStore::get(cx);
        let sidebar = settings.sidebar_visible;
        let p = &self.palette;
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .flex()
            .flex_col()
            .size_full()
            .bg(self.c(p.window))
            .font_family(UI_FONT)
            .text_size(px(13.))
            .text_color(self.c(p.text))
            .child(self.titlebar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(sidebar, |row| row.child(self.sidebar(cx)))
                    .child(match self.active_editor() {
                        Some(editor) => div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(editor.clone())
                            .into_any_element(),
                        None => self.content(cx).into_any_element(),
                    }),
            )
            .child(self.status_bar(cx))
    }
}
