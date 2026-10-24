//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace, its branch and its tabs; the left dock
//! (workspaces, then the file tree); and the 24pt status bar. Sizes come
//! from `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::path::{Path, PathBuf};

use gpui_kit::{div, prelude::*, px, svg, AnyElement, Context, FontWeight, Hsla};
use impulse_kit::chrome::{metrics, Rgba as KitRgba};

use crate::settings_store::SettingsStore;
use crate::workbench::{SidebarPanel, Workbench};
use crate::workspace::{Kind, Surface, Workspace};

pub(crate) fn hsla(color: KitRgba) -> Hsla {
    crate::terminal_view::rgba(color).into()
}

pub(crate) fn icon(name: &str, size: f32, color: KitRgba) -> impl IntoElement {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(hsla(color))
}

/// `~/…` for paths in the home folder.
pub(crate) fn abbreviate_home(path: &str) -> String {
    let home = crate::workspace::home().to_string_lossy().into_owned();
    match path.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

/// One row of the file tree.
struct TreeRow {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

/// The visible rows: the root's entries, and those of every expanded
/// folder under them.
fn tree_rows(workspace: &Workspace, root: &Path) -> Vec<TreeRow> {
    fn walk(w: &Workspace, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        for (name, is_dir) in w.children.get(dir).map(Vec::as_slice).unwrap_or_default() {
            let path = dir.join(name);
            let expanded = *is_dir && w.expanded_dirs.contains(&path);
            rows.push(TreeRow {
                path: path.clone(),
                name: name.clone(),
                is_dir: *is_dir,
                depth,
            });
            if expanded {
                walk(w, &path, depth + 1, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(workspace, root, 0, &mut rows);
    rows
}

impl Workbench {
    /// The folder the file tree shows: a folder workspace's root, or for
    /// Scratch the active tab's folder.
    fn tree_root(&self, cx: &gpui_kit::App) -> PathBuf {
        let workspace = self.workspace();
        match workspace.kind {
            Kind::Folder => workspace.root.clone(),
            Kind::Scratch => self
                .focused_surface()
                .and_then(|s| s.directory(cx))
                .filter(|d| d.is_dir())
                .unwrap_or_else(|| workspace.root.clone()),
        }
    }

    fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let show_hidden = SettingsStore::get(cx).sidebar_show_hidden;
        let workspace = self.workspace_mut();
        if !workspace.expanded_dirs.remove(&path) {
            let entries = crate::workspace::list_dir(&path, show_hidden);
            workspace.children.insert(path.clone(), entries);
            workspace.expanded_dirs.insert(path);
        }
        cx.notify();
    }

    pub(crate) fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        // macOS draws its traffic lights in this inset; on Linux the window
        // controls are Impulse's own (client-side decorations).
        let leading = if cfg!(target_os = "macos") {
            metrics::TRAFFIC_LIGHT_INSET
        } else {
            12.0
        };
        let workspace = self.workspace();
        let shortcut = impulse_kit::keybindings::display_for(
            "command_palette",
            &SettingsStore::get(cx).keybinding_overrides,
            impulse_kit::keybindings::Platform::current(),
        )
        .unwrap_or_default();
        let mut tabs = div().flex().items_center().gap(px(2.)).min_w_0();
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let active = index == workspace.active;
            let surface = tab.focused_surface();
            let dirty = tab.panes.values().any(|s| s.is_dirty(cx));
            let color = if active { p.text } else { p.text_secondary };
            let mut t = div()
                .id(("tab", tab.id))
                .group("tab")
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::TAB_HEIGHT))
                .pl(px(12.))
                .pr(px(8.))
                .max_w(px(220.))
                .rounded(px(metrics::RADIUS))
                .text_color(hsla(color))
                .hover(|s| s.bg(hsla(p.hover)))
                .on_click(cx.listener(move |w, _, window, cx| w.activate_tab(index, window, cx)))
                .child(icon(
                    if tab.is_split() {
                        "columns-2"
                    } else {
                        surface.icon()
                    },
                    metrics::ICON,
                    color,
                ))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(surface.title(cx)),
                )
                .child(
                    div()
                        .id(("close", tab.id))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.))
                        .h(px(16.))
                        .rounded(px(metrics::RADIUS_SMALL))
                        .hover(|s| s.bg(hsla(p.pressed)))
                        .on_click(cx.listener(move |w, _, window, cx| {
                            cx.stop_propagation();
                            w.close_tab_at(index, window, cx)
                        }))
                        .child(if dirty {
                            div()
                                .w(px(8.))
                                .h(px(8.))
                                .rounded_full()
                                .bg(hsla(p.text_secondary))
                                .into_any_element()
                        } else {
                            div()
                                .invisible()
                                .group_hover("tab", |s| s.visible())
                                .when(active, |d| d.visible())
                                .child(icon("x", metrics::ICON_SMALL, p.text_tertiary))
                                .into_any_element()
                        }),
                );
            if active {
                t = t
                    .bg(hsla(p.raised))
                    .border_b_2()
                    .border_color(hsla(p.accent));
            }
            tabs = tabs.child(t);
        }
        div()
            .flex()
            .items_center()
            .h(px(metrics::TITLEBAR_HEIGHT))
            .pl(px(leading))
            .pr(px(10.))
            .gap(px(10.))
            .bg(hsla(p.chrome))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .id("toggle-sidebar")
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|w, _, window, cx| w.run_command("toggle_sidebar", window, cx)),
                    )
                    .child(icon("panel-left", metrics::ICON_LARGE, p.text_secondary)),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(workspace.name()),
            )
            .when(!workspace.branch.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .text_color(hsla(p.text_secondary))
                        .child(icon("git-branch", metrics::ICON_SMALL, p.text_secondary))
                        .child(workspace.branch.clone()),
                )
            })
            .child(div().w(px(8.)).flex_none())
            .child(tabs)
            .child(
                div()
                    .id("new-tab")
                    .flex_none()
                    .p(px(4.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(|w, _, window, cx| w.run_command("new_tab", window, cx)))
                    .child(icon("plus", metrics::ICON, p.text_secondary)),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("search-field")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .w(px(300.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.overlay))
                    .border_1()
                    .border_color(hsla(p.hairline))
                    .text_color(hsla(p.text_tertiary))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|w, _, window, cx| {
                            w.run_command("command_palette", window, cx)
                        }),
                    )
                    .child(icon("search", metrics::ICON_SMALL, p.text_tertiary))
                    .child("Search or run a command")
                    .child(div().flex_1())
                    .child(shortcut),
            )
    }

    fn workspace_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let workspace = &self.workspaces[index];
        let selected = index == self.active;
        let id = workspace.id;
        let expanded = workspace.expanded;
        let mut column = div().flex().flex_col();
        let row = div()
            .id(("workspace", id))
            .group("workspace")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(metrics::ROW_HEIGHT + 4.))
            .pl(px(6.))
            .pr(px(8.))
            .mx(px(6.))
            .rounded(px(metrics::RADIUS))
            .when(selected, |d| d.bg(hsla(p.selection)))
            .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
            .on_click(cx.listener(move |w, _, window, cx| w.activate_workspace(index, window, cx)))
            .child(
                div()
                    .id(("expand", id))
                    .w(px(12.))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(ws) = w.workspaces.get_mut(index) {
                            ws.expanded = !ws.expanded;
                        }
                        cx.notify();
                    }))
                    .child(icon(
                        if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        metrics::ICON_SMALL,
                        p.text_tertiary,
                    )),
            )
            .child(icon(
                match workspace.kind {
                    Kind::Folder => "folder-git-2",
                    Kind::Scratch => "square-terminal",
                },
                metrics::ICON,
                p.text_secondary,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .child(workspace.name()),
            )
            .when(!workspace.branch.is_empty(), |d| {
                d.child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child(workspace.branch.clone()),
                )
            })
            .when(workspace.kind == Kind::Scratch, |d| {
                d.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child("follows tab"),
                )
            })
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.))
                    .text_color(hsla(p.text_tertiary))
                    .child(workspace.tabs.len().to_string()),
            );
        column = column.child(row);
        if expanded {
            for (t, tab) in workspace.tabs.iter().enumerate() {
                let surface = tab.focused_surface();
                let active_tab = selected && t == workspace.active;
                column = column.child(
                    div()
                        .id(("workspace-tab", tab.id))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(metrics::ROW_HEIGHT))
                        .pl(px(36.))
                        .pr(px(8.))
                        .mx(px(6.))
                        .rounded(px(metrics::RADIUS))
                        .text_size(px(11.5))
                        .text_color(hsla(if active_tab { p.text } else { p.text_secondary }))
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(cx.listener(move |w, _, window, cx| {
                            w.active = index;
                            w.activate_tab(t, window, cx);
                        }))
                        .child(icon(surface.icon(), metrics::ICON_SMALL, p.text_tertiary))
                        .child(surface.title(cx)),
                );
            }
        }
        column.into_any_element()
    }

    pub(crate) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let settings = SettingsStore::get(cx).clone();
        let open_path = self.active_editor().map(|e| e.read(cx).path.clone());
        let root = self.tree_root(cx);
        let workspace = self.workspace();
        let mut files = div()
            .id("files")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(px(4.));
        if self.panel == SidebarPanel::Files {
            // Scratch's tree follows the tab: list its folder when it's new.
            let rows = if workspace.children.contains_key(&root) {
                tree_rows(workspace, &root)
            } else {
                crate::workspace::list_dir(&root, settings.sidebar_show_hidden)
                    .into_iter()
                    .map(|(name, is_dir)| TreeRow {
                        path: root.join(&name),
                        name,
                        is_dir,
                        depth: 0,
                    })
                    .collect()
            };
            for (index, row) in rows.into_iter().enumerate() {
                let expanded = workspace.expanded_dirs.contains(&row.path);
                let selected = open_path.as_deref() == Some(row.path.as_path());
                let path = row.path.clone();
                let is_dir = row.is_dir;
                files = files.child(
                    div()
                        .id(("file", index))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .h(px(metrics::ROW_HEIGHT))
                        .pl(px(10. + 14. * row.depth as f32))
                        .pr(px(10.))
                        .when(selected, |d| d.bg(hsla(p.selection)))
                        .hover(|style| style.bg(hsla(p.hover)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if is_dir {
                                this.toggle_dir(path.clone(), cx);
                            } else {
                                this.open_file(&path, window, cx);
                            }
                        }))
                        .child(div().w(px(12.)).child(if row.is_dir {
                            icon(
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
                        .child(icon(
                            match (row.is_dir, expanded) {
                                (true, true) => "folder-open",
                                (true, false) => "folder",
                                _ => "file",
                            },
                            metrics::ICON,
                            if row.is_dir { p.info } else { p.text_secondary },
                        ))
                        .child(div().overflow_hidden().whitespace_nowrap().child(row.name)),
                );
            }
        } else {
            files = files.child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .text_color(hsla(p.text_tertiary))
                    .child(match self.panel {
                        SidebarPanel::Changes => "Changes aren't in the Rust app yet.",
                        _ => "Search isn't in the Rust app yet.",
                    }),
            );
        }
        let panel_button = |id: &'static str,
                            panel: SidebarPanel,
                            name: &'static str,
                            label: &'static str,
                            cx: &mut Context<Self>| {
            let selected = self.panel == panel;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .h(px(24.))
                .rounded(px(metrics::RADIUS))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(hsla(p.raised)).font_weight(FontWeight::SEMIBOLD)
                })
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.panel = panel;
                    cx.notify();
                }))
                .child(icon(
                    name,
                    metrics::ICON,
                    if selected { p.text } else { p.text_tertiary },
                ))
                .when(selected, |d| d.child(label))
        };
        let mut workspaces = div()
            .flex()
            .flex_col()
            .py(px(8.))
            .gap(px(2.))
            .border_b_1()
            .border_color(hsla(p.hairline));
        // Folder workspaces under their name, then Scratch.
        for index in 0..self.workspaces.len() {
            if self.workspaces[index].kind == Kind::Folder {
                workspaces = workspaces.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(14.))
                        .h(px(metrics::ROW_HEIGHT))
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(hsla(p.text_secondary))
                        .child(self.workspaces[index].name()),
                );
            }
            workspaces = workspaces.child(self.workspace_row(index, cx));
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(settings.sidebar_width as f32))
            .h_full()
            .bg(hsla(p.panel))
            .border_r_1()
            .border_color(hsla(p.hairline))
            .child(workspaces)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(metrics::PANE_HEADER_HEIGHT + 6.))
                    .px(px(8.))
                    .child(panel_button(
                        "panel-files",
                        SidebarPanel::Files,
                        "folder-tree",
                        "Files",
                        cx,
                    ))
                    .child(panel_button(
                        "panel-changes",
                        SidebarPanel::Changes,
                        "git-branch",
                        "Changes",
                        cx,
                    ))
                    .child(panel_button(
                        "panel-search",
                        SidebarPanel::Search,
                        "search",
                        "Search",
                        cx,
                    )),
            )
            .child(files)
    }

    pub(crate) fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let item = |name: &str, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(name, metrics::ICON_SMALL, p.text_secondary))
                .child(text)
        };
        let workspace = self.workspace();
        let right: Vec<AnyElement> = match self.focused_surface() {
            Some(Surface::Editor(editor)) => {
                let editor = editor.read(cx);
                let (line, column) = editor.cursor(cx);
                [
                    editor.notice.clone().map(|n| {
                        div()
                            .text_color(hsla(p.warning))
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
                            .child(if editor.use_spaces {
                                format!("Spaces: {}", editor.tab_size)
                            } else {
                                format!("Tab Size: {}", editor.tab_size)
                            })
                            .into_any_element(),
                    ),
                    Some(div().child("UTF-8").into_any_element()),
                    Some(div().child(editor.language.clone()).into_any_element()),
                    Some(editor.server_names())
                        .filter(|names| !names.is_empty())
                        .map(|names| {
                            div()
                                .text_color(hsla(p.text_tertiary))
                                .child(names.join(", "))
                                .into_any_element()
                        }),
                ]
                .into_iter()
                .flatten()
                .collect()
            }
            Some(Surface::Review(_)) => Vec::new(),
            Some(Surface::Terminal(terminal)) => {
                let terminal = terminal.read(cx);
                vec![
                    item("folder", abbreviate_home(&terminal.cwd)).into_any_element(),
                    item("square-terminal", crate::workspace::shell_name()).into_any_element(),
                ]
            }
            None => Vec::new(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .h(px(metrics::STATUS_BAR_HEIGHT))
            .px(px(12.))
            .bg(hsla(p.chrome))
            .border_t_1()
            .border_color(hsla(p.hairline))
            .text_size(px(12.))
            .text_color(hsla(p.text_secondary))
            .when(!workspace.branch.is_empty(), |d| {
                d.child(item("git-branch", workspace.branch.clone()))
            })
            .children(self.is_restricted().then(|| {
                div()
                    .id("restricted")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(hsla(p.warning))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_command("trust_folder", window, cx)
                    }))
                    .child(icon("shield-alert", metrics::ICON_SMALL, p.warning))
                    .child("Restricted")
            }))
            .children(
                self.notice
                    .clone()
                    .map(|notice| div().text_color(hsla(p.warning)).child(notice)),
            )
            .child(div().flex_1())
            .children(right)
    }
}
