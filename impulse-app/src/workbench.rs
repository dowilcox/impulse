//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace and tabs, the left dock (workspaces,
//! then files), the content pane with the terminal's context chips and
//! input bar, and the 24pt status bar. Sizes come from
//! `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use gpui_kit::{
    actions, div, prelude::*, px, svg, AnyElement, App, Context, Entity, FocusHandle, FontWeight,
    Hsla, KeyBinding, KeyDownEvent, SharedString, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};

use crate::editor_tab::{EditorTab, EditorTabChanged, ShowDocument};
use crate::lsp_hub::LspHub;
use crate::terminal_view::{rgba, TerminalView};
use crate::Args;

const UI_FONT: &str = "Inter";

actions!(impulse, [Save, CloseTab]);

/// Keys for the workbench's commands: `secondary` is Cmd on macOS and Ctrl
/// on Linux.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
    ]);
}

enum Tab {
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
    tabs: Vec<Tab>,
    active: usize,
    focus: FocusHandle,
    input: String,
    root: PathBuf,
    workspace: String,
    branch: String,
    /// Each listed folder's entries (folders first, then by name), read
    /// when the folder is first expanded.
    children: HashMap<PathBuf, Vec<(String, bool)>>,
    expanded: HashSet<PathBuf>,
    /// A message for the status bar, such as why a file didn't open.
    notice: Option<String>,
    /// Language servers, when the folder is trusted.
    lsp: Option<std::rc::Rc<LspHub>>,
    _lsp_events: Option<gpui_kit::Task<()>>,
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
        let theme = impulse_kit::theme::builtin_theme(&args.theme)
            .unwrap_or_else(|| impulse_kit::theme::builtin_theme("nord").unwrap());
        let palette = ChromePalette::new(&theme, false);
        crate::kit_theme::apply(&theme, &palette, cx);
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
        window.focus(&focus, cx);
        let mut workbench = Self {
            palette,
            tabs: vec![Tab::Terminal],
            terminal,
            active: 0,
            focus,
            input: String::new(),
            workspace: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            branch: git(&args.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]),
            children: HashMap::from([(root.clone(), list_dir(&root))]),
            expanded: HashSet::new(),
            root: root.clone(),
            notice: None,
            lsp: None,
            _lsp_events: None,
        };
        if args.trust {
            let (hub, events) = LspHub::new(root);
            workbench.lsp = Some(hub);
            workbench._lsp_events = Some(cx.spawn(async move |workbench, cx| {
                while let Ok(event) = events.recv().await {
                    let alive =
                        workbench.update(cx, |workbench, cx| workbench.server_event(event, cx));
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        }
        for path in &args.open {
            let path = workbench.root.join(path);
            workbench.open_file(&path, window, cx);
        }
        workbench
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
        let lsp = self.lsp.clone();
        let show_document = self.show_document_handler(cx);
        match impulse_kit::text_file::open(path).map(|opened| {
            cx.new(|cx| {
                let lsp = lsp.as_deref().map(|hub| (hub, show_document));
                EditorTab::new(path, opened, lsp, window, cx)
            })
        }) {
            Ok(editor) => {
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

    /// Go-to-definition into another file: open it in a tab at the target.
    /// Targets in the same file are left to the editor.
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
            impulse_lsp::ServerEvent::Diagnostics(params) => {
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
                                editor.set_diagnostics(params.diagnostics.clone(), cx)
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
            Tab::Terminal => window.focus(&self.focus, cx),
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

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.active_editor().is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        let terminal = self.terminal.read(cx);
        match keystroke.key.as_str() {
            "enter" => {
                terminal.write(format!("{}\r", self.input).as_bytes());
                self.input.clear();
            }
            "backspace" => {
                self.input.pop();
            }
            "c" if keystroke.modifiers.control => terminal.write(b"\x03"),
            _ => {
                if let Some(text) = keystroke
                    .key_char
                    .as_ref()
                    .filter(|_| !keystroke.modifiers.control)
                {
                    self.input.push_str(text);
                }
            }
        }
        cx.notify();
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
            .child(self.icon("panel-left", metrics::ICON_LARGE, p.text_secondary))
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
            .w(px(metrics::LEFT_DOCK_DEFAULT_WIDTH))
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
        let input: SharedString = if self.input.is_empty() {
            "Run a command…".into()
        } else {
            self.input.clone().into()
        };
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
                    .child(chips)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(36.))
                            .px(px(12.))
                            .rounded(px(metrics::RADIUS_LARGE))
                            .border_1()
                            .border_color(self.c(p.focus_ring))
                            .font_family("JetBrains Mono")
                            .text_color(self.c(if self.input.is_empty() {
                                p.text_tertiary
                            } else {
                                p.text
                            }))
                            .child(self.icon("chevron-right", metrics::ICON, p.accent))
                            .child(input),
                    ),
            )
    }

    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Everything but the terminal's own area, from the layout above.
        let viewport = window.viewport_size();
        let width = viewport.width - px(metrics::LEFT_DOCK_DEFAULT_WIDTH + 1.);
        let height =
            viewport.height - px(metrics::TITLEBAR_HEIGHT + metrics::STATUS_BAR_HEIGHT + 2. + 98.);
        self.terminal.update(cx, |t, _| t.fit(width, height));

        let p = &self.palette;
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::close_tab))
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
                    .child(self.sidebar(cx))
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
