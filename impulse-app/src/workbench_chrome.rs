//! The window's chrome, laid out as `WorkbenchView` lays it out: a 40pt
//! titlebar band with the workspace, its branch and its tabs; the left dock
//! (workspaces, then the file tree); and the 24pt status bar. Sizes come
//! from `impulse_kit::chrome::metrics`, colors from `ChromePalette`.

use std::path::{Path, PathBuf};

use std::rc::Rc;

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::{div, prelude::*, px, svg, AnyElement, Context, FontWeight, Hsla, Window};
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
    pub(crate) fn tree_root(&self, cx: &gpui_kit::App) -> PathBuf {
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

    /// A tab's context menu. Its actions find the tab by id when chosen:
    /// tabs can close or move while the menu is open.
    fn tab_menu(
        &self,
        ws: usize,
        index: usize,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        type Run = Rc<dyn Fn(&mut Workbench, usize, usize, &mut Window, &mut Context<Workbench>)>;
        let workspace = &self.workspaces[ws];
        let tab = &workspace.tabs[index];
        let current = ws == self.active;
        let mut items: Vec<(&'static str, Option<Run>)> = vec![
            (
                if tab.pinned { "Unpin Tab" } else { "Pin Tab" },
                Some(Rc::new(|w, ws, t, _, cx| w.toggle_pin(ws, t, cx))),
            ),
            (
                "Close Tab",
                Some(Rc::new(|w, ws, t, window, cx| {
                    w.active = ws;
                    w.close_tab_at(t, window, cx)
                })),
            ),
            ("", None),
        ];
        if current && index != workspace.active {
            items.push((
                "Move into Current Tab, Right",
                Some(Rc::new(|w, _, t, window, cx| {
                    w.join_tab(t, false, window, cx)
                })),
            ));
            items.push((
                "Move into Current Tab, Below",
                Some(Rc::new(|w, _, t, window, cx| {
                    w.join_tab(t, true, window, cx)
                })),
            ));
            items.push(("", None));
        } else if current && tab.is_split() {
            items.push((
                "Even Out Panes",
                Some(Rc::new(|w, _, _, window, cx| {
                    w.run_command("equalize_panes", window, cx)
                })),
            ));
            items.push((
                "Move Pane to New Tab",
                Some(Rc::new(|w, _, _, window, cx| {
                    w.run_command("move_pane_to_tab", window, cx)
                })),
            ));
            items.push(("", None));
        }
        items.push((
            "New Tab",
            Some(Rc::new(|w, _, _, window, cx| {
                w.run_command("new_tab", window, cx)
            })),
        ));
        let (id, workbench) = (tab.id, cx.entity().downgrade());
        move |mut menu, _, _| {
            for (title, run) in &items {
                let Some(run) = run.clone() else {
                    menu = menu.separator();
                    continue;
                };
                let workbench = workbench.clone();
                menu = menu.item(PopupMenuItem::new(*title).on_click(move |_, window, cx| {
                    let _ = workbench.update(cx, |w, cx| {
                        let found = w.workspaces.iter().enumerate().find_map(|(ws, workspace)| {
                            let t = workspace.tabs.iter().position(|t| t.id == id)?;
                            Some((ws, t))
                        });
                        if let Some((ws, t)) = found {
                            run(w, ws, t, window, cx)
                        }
                    });
                }));
            }
            menu
        }
    }

    /// `sidebar_tabs`: the sidebar lists the tabs (while it's shown) and the
    /// titlebar doesn't.
    fn tabs_in_sidebar(&self, cx: &gpui_kit::App) -> bool {
        let settings = SettingsStore::get(cx);
        settings.sidebar_tabs && settings.sidebar_visible
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
            let attention = !active && Self::tab_needs_attention(tab, cx);
            let color = if active { p.text } else { p.text_secondary };
            let compact = tab.pinned;
            let drop_color = hsla(p.selection);
            let title = surface.title(cx);
            let help = if tab.preview {
                format!("{title}\nPreview: double-click to keep")
            } else {
                title.clone()
            };
            let mut t = div()
                .id(("tab", tab.id))
                .group("tab")
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(metrics::TAB_HEIGHT))
                .pl(px(if compact { 8. } else { 12. }))
                .pr(px(8.))
                .max_w(px(220.))
                .rounded(px(metrics::RADIUS))
                .text_color(hsla(color))
                .hover(|s| s.bg(hsla(p.hover)))
                .on_click(
                    cx.listener(move |w, event: &gpui_kit::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            w.keep_tab(index, cx);
                        }
                        w.activate_tab(index, window, cx)
                    }),
                )
                .on_drag(
                    DraggedTab {
                        index,
                        title: title.clone(),
                    },
                    |dragged, _, _, cx| {
                        cx.new(|_| crate::file_tree_actions::DragPreview(dragged.title.clone()))
                    },
                )
                .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(drop_color))
                .on_drop(cx.listener(move |w, dragged: &DraggedTab, _, cx| {
                    w.move_tab(dragged.index, index, cx)
                }))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                })
                .child(self.tab_icon(
                    tab,
                    if tab.is_split() {
                        "columns-2"
                    } else {
                        surface.icon()
                    },
                    metrics::ICON,
                    color,
                    cx,
                ));
            if !compact {
                t = t
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when(tab.preview, |d| d.italic())
                            .child(title),
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
                            .child(if attention {
                                crate::agent_views::status_dot(7., p.attention)
                            } else if dirty {
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
            }
            if active {
                t = t
                    .bg(hsla(p.raised))
                    .border_b_2()
                    .border_color(hsla(p.accent));
            }
            let t = t.context_menu(self.tab_menu(self.active, index, cx));
            tabs = tabs.child(t);
        }
        let tabs_in_sidebar = self.tabs_in_sidebar(cx);
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
            .children(crate::menus::menu_button(cx))
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
                        .id("titlebar-branch")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .text_color(hsla(p.text_secondary))
                        .cursor_pointer()
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("switch_branch", window, cx)
                        }))
                        .child(icon("git-branch", metrics::ICON_SMALL, p.text_secondary))
                        .child(workspace.branch.clone())
                        .children(self.branch_sync_text(cx).map(|sync| {
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(10.5))
                                .text_color(hsla(p.text_tertiary))
                                .child(sync)
                        })),
                )
            })
            .child(div().w(px(8.)).flex_none())
            // With `sidebar_tabs`, the sidebar lists the tabs while it's open.
            .when(!tabs_in_sidebar, |d| {
                d.child(tabs).child(
                    div()
                        .id("new-tab")
                        .flex_none()
                        .p(px(4.))
                        .rounded(px(metrics::RADIUS_SMALL))
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(
                            cx.listener(|w, _, window, cx| w.run_command("new_tab", window, cx)),
                        )
                        .child(icon("plus", metrics::ICON, p.text_secondary)),
                )
            })
            .child(div().flex_1())
            .children(self.agent_inbox_button(cx))
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

    fn workspace_row(&self, index: usize, indented: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let workspace = &self.workspaces[index];
        let selected = index == self.active;
        let id = workspace.id;
        let tabs_in_sidebar = self.tabs_in_sidebar(cx);
        // Listed tabs stay open for the active workspace.
        let expanded = workspace.expanded || tabs_in_sidebar && selected;
        let drop_color = hsla(p.selection);
        let mut column = div().flex().flex_col();
        let row = div()
            .id(("workspace", id))
            .group("workspace")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(metrics::ROW_HEIGHT + 4.))
            .pl(px(if indented { 14. } else { 6. }))
            .pr(px(8.))
            .mx(px(6.))
            .rounded(px(metrics::RADIUS))
            .when(selected, |d| d.bg(hsla(p.selection)))
            .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
            .on_click(cx.listener(move |w, _, window, cx| w.activate_workspace(index, window, cx)))
            .on_drag(
                crate::task_sidebar::DraggedWorkspace {
                    id,
                    name: workspace.name(),
                },
                |dragged, _, _, cx| {
                    cx.new(|_| crate::file_tree_actions::DragPreview(dragged.name.clone()))
                },
            )
            .drag_over::<crate::task_sidebar::DraggedWorkspace>(move |style, _, _, _| {
                style.bg(drop_color)
            })
            .on_drop(cx.listener(
                move |w, dragged: &crate::task_sidebar::DraggedWorkspace, _, cx| {
                    w.drop_workspace(dragged.id, id, cx)
                },
            ))
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
            .when(
                !workspace.branch.is_empty() && workspace.branch != workspace.name(),
                |d| {
                    d.child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(10.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(workspace.branch.clone()),
                    )
                },
            )
            .when(workspace.kind == Kind::Scratch, |d| {
                d.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child("follows tab"),
                )
            })
            .children(self.workspace_agent_counts(index, cx))
            .children(self.task_chips(index, cx))
            .tooltip({
                let help = self.workspace_help(index);
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                }
            })
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.))
                    .text_color(hsla(p.text_tertiary))
                    .child(workspace.tabs.len().to_string()),
            );
        column = column.child(row.context_menu(self.workspace_menu(index, cx)));
        if expanded {
            for (t, tab) in workspace.tabs.iter().enumerate() {
                let surface = tab.focused_surface();
                let active_tab = selected && t == workspace.active;
                let menu = self.tab_menu(index, t, cx);
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
                        .on_click(cx.listener(
                            move |w, event: &gpui_kit::ClickEvent, window, cx| {
                                w.active = index;
                                if event.click_count() >= 2 {
                                    w.keep_tab(t, cx);
                                }
                                w.activate_tab(t, window, cx);
                            },
                        ))
                        .child(self.tab_icon(
                            tab,
                            surface.icon(),
                            metrics::ICON_SMALL,
                            p.text_tertiary,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .when(tab.preview, |d| d.italic())
                                .child(surface.title(cx)),
                        )
                        .children(
                            (!active_tab && Self::tab_needs_attention(tab, cx))
                                .then(|| crate::agent_views::status_dot(6., p.attention)),
                        )
                        .children(tab.pinned.then(|| icon("pin", 10., p.text_tertiary)))
                        .when(active_tab && tabs_in_sidebar, |d| d.bg(hsla(p.selection)))
                        .context_menu(menu),
                );
            }
            if tabs_in_sidebar && selected {
                column = column.child(
                    div()
                        .id(("workspace-new-tab", id))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(metrics::ROW_HEIGHT))
                        .pl(px(36.))
                        .mx(px(6.))
                        .rounded(px(metrics::RADIUS))
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_tertiary))
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(
                            cx.listener(|w, _, window, cx| w.run_command("new_tab", window, cx)),
                        )
                        .child(icon("plus", metrics::ICON_SMALL, p.text_tertiary))
                        .child("New Tab"),
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
        let (root_drop, root_external) = (root.clone(), root.clone());
        let mut files = div()
            .id("files")
            .on_drop(cx.listener(
                move |this, dragged: &crate::file_tree_actions::DraggedPath, window, cx| {
                    this.tree_drop(vec![dragged.0.clone()], &root_drop, window, cx)
                },
            ))
            .on_drop(
                cx.listener(move |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                    this.tree_drop(paths.paths().to_vec(), &root_external, window, cx)
                }),
            )
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(px(4.));
        if self.panel == SidebarPanel::Files {
            let ignored = KitRgba::from_hex(&self.theme.git_ignored);
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
                let status = self.git.tree_status(&row.path, p, ignored, cx);
                let menu = self.tree_menu(row.path.clone(), row.is_dir, cx);
                let row_element = div()
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
                    .on_click(
                        cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                            if is_dir {
                                this.toggle_dir(path.clone(), cx);
                            } else if event.click_count() >= 2 {
                                // The second click of a double-click keeps the
                                // preview.
                                this.open_file(&path, window, cx);
                            } else {
                                this.preview_file(&path, window, cx);
                            }
                        }),
                    )
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
                    .child(
                        gpui_kit::img(crate::file_icons::icon_path(
                            &row.name, row.is_dir, expanded,
                        ))
                        .size(px(16.))
                        .flex_none(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when_some(status, |d, (_, color)| d.text_color(hsla(color)))
                            .child(row.name),
                    )
                    .children(status.filter(|(letter, _)| !letter.is_empty()).map(
                        |(letter, color)| {
                            div()
                                .flex_none()
                                .font_family("JetBrains Mono")
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(color))
                                .child(letter)
                        },
                    ));
                let (drag_path, drop_target, external_target) =
                    (row.path.clone(), row.path.clone(), row.path.clone());
                let drag_name = row
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let row_element = row_element
                    .on_drag(
                        crate::file_tree_actions::DraggedPath(drag_path),
                        move |_, _, _, cx| {
                            cx.new(|_| crate::file_tree_actions::DragPreview(drag_name.clone()))
                        },
                    )
                    .drag_over::<crate::file_tree_actions::DraggedPath>(|style, _, _, _| {
                        style.bg(gpui_kit::black().opacity(0.12))
                    })
                    .on_drop(cx.listener(
                        move |this, dragged: &crate::file_tree_actions::DraggedPath, window, cx| {
                            this.tree_drop(vec![dragged.0.clone()], &drop_target, window, cx)
                        },
                    ))
                    .on_drop(cx.listener(
                        move |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                            this.tree_drop(paths.paths().to_vec(), &external_target, window, cx)
                        },
                    ));
                files = files.child(row_element.context_menu(menu));
            }
        }
        if self.panel == SidebarPanel::Search {
            let root = root.to_string_lossy().into_owned();
            self.search
                .update(cx, |search, cx| search.set_root(root, cx));
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
        // A repository's workspaces (its tasks) under its name.
        for (name, group) in self.workspace_groups() {
            let indented = name.is_some();
            if let Some(name) = name {
                workspaces = workspaces.child(self.repository_header(name));
            }
            for index in group {
                workspaces = workspaces.child(self.workspace_row(index, indented, cx));
            }
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
            .child(match self.panel {
                SidebarPanel::Changes => self.changes_panel(cx),
                SidebarPanel::Search => self.search.clone().into_any_element(),
                SidebarPanel::Files => files.into_any_element(),
            })
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
                    editor.vim_status(cx).map(|status| {
                        div()
                            .font_family("JetBrains Mono")
                            .child(status)
                            .into_any_element()
                    }),
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
            Some(Surface::Review(_)) | Some(Surface::History(_)) => Vec::new(),
            Some(Surface::Terminal(terminal)) => {
                let terminal = terminal.read(cx);
                vec![
                    item("folder", abbreviate_home(&terminal.cwd)).into_any_element(),
                    item("square-terminal", crate::workspace::shell_name()).into_any_element(),
                ]
            }
            _ => Vec::new(),
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
                d.child(
                    div()
                        .id("status-branch")
                        .cursor_pointer()
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("switch_branch", window, cx)
                        }))
                        .child(item("git-branch", workspace.branch.clone())),
                )
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
            .children(self.problems_status(cx))
            .children(
                self.notice
                    .clone()
                    .map(|notice| div().text_color(hsla(p.warning)).child(notice)),
            )
            .child(div().flex_1())
            .children(self.update_item(cx))
            .children(right)
    }

    /// "Update x.y.z" when a newer Impulse is out, opening its page.
    fn update_item(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let update = cx
            .try_global::<crate::update_check::AvailableUpdate>()?
            .0
            .clone()?;
        let p = &self.palette;
        let help = format!("Impulse {} is available", update.version);
        Some(
            div()
                .id("update")
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .text_color(hsla(p.success))
                .on_click(move |_, _, cx| cx.open_url(&update.url))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                })
                .child(icon("arrow-down", metrics::ICON_SMALL, p.success))
                .child(format!("Update {}", update.version))
                .into_any_element(),
        )
    }
}

/// A titlebar tab being dragged to a new place.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    index: usize,
    title: String,
}
