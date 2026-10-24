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
/// "+41 −9" in the git colors (the Swift `DiffStat`).
pub(crate) fn diff_stat(
    added: usize,
    removed: usize,
    p: &impulse_kit::chrome::ChromePalette,
) -> impl IntoElement {
    div()
        .flex()
        .gap(px(4.))
        .font_family("JetBrains Mono")
        .text_size(px(11.))
        .when(added > 0 || removed == 0, |d| {
            d.child(
                div()
                    .text_color(hsla(p.git_added))
                    .child(format!("+{added}")),
            )
        })
        .when(removed > 0, |d| {
            d.child(
                div()
                    .text_color(hsla(p.git_deleted))
                    .child(format!("\u{2212}{removed}")),
            )
        })
}

/// One status-bar entry (the Swift `StatusItem`): an icon and a label,
/// highlighted on hover when it does something.
pub(crate) fn status_item(
    id: impl Into<gpui_kit::ElementId>,
    icon_name: Option<&str>,
    help: &str,
    tint: Option<KitRgba>,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let color = tint.unwrap_or(p.text_secondary);
    let help: gpui_kit::SharedString = help.to_string().into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(20.))
        .px(px(6.))
        .rounded(px(metrics::RADIUS_SMALL))
        .whitespace_nowrap()
        .text_color(hsla(color))
        .hover(|s| s.bg(hsla(p.hover)))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
        })
        .children(icon_name.map(|name| icon(name, 11.5, color)))
}

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
        workspace.save_expanded();
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
                .pl(px(if compact { 8. } else { 9. }))
                .pr(px(if compact { 8. } else { 5. }))
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
                    surface.icon(),
                    metrics::ICON,
                    if active { p.text } else { p.text_tertiary },
                    cx,
                ));
            if !compact {
                let panes = tab.panes.len();
                t = t
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(12.))
                            .when(active, |d| d.font_weight(gpui_kit::FontWeight::MEDIUM))
                            .when(tab.preview, |d| d.italic())
                            .child(title),
                    )
                    .when(panes > 1, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .child(icon(
                                    if tab.zoomed.is_some() {
                                        "maximize-2"
                                    } else {
                                        "columns-2"
                                    },
                                    10.,
                                    p.text_tertiary,
                                ))
                                .child(
                                    div()
                                        .font_family("JetBrains Mono")
                                        .text_size(px(10.))
                                        .text_color(hsla(p.text_tertiary))
                                        .child(panes.to_string()),
                                ),
                        )
                    })
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
                            // The close button on hover; otherwise a dot for
                            // attention or unsaved changes, as in Swift.
                            .child(
                                div()
                                    .invisible()
                                    .group_hover("tab", |s| s.visible())
                                    .absolute()
                                    .child(icon("x", metrics::ICON_SMALL, p.text_secondary)),
                            )
                            .when(attention || dirty, |d| {
                                d.child(div().group_hover("tab", |s| s.invisible()).child(
                                    crate::agent_views::status_dot(
                                        7.,
                                        if attention {
                                            p.attention
                                        } else {
                                            p.text_secondary
                                        },
                                    ),
                                ))
                            }),
                    );
            }
            if active {
                t = t.bg(hsla(p.raised)).relative().child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(8.))
                        .right(px(8.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(hsla(p.accent)),
                );
            }
            let t = t.context_menu(self.tab_menu(self.active, index, cx));
            tabs = tabs.child(t);
        }
        let tabs_in_sidebar = self.tabs_in_sidebar(cx);
        let (sidebar_visible, sidebar_width) = {
            let settings = SettingsStore::get(cx);
            (settings.sidebar_visible, settings.sidebar_width)
        };
        // Over the sidebar the leading part fills its width, so tabs start
        // where the content column starts; without it, it hugs its content.
        let leading_width = sidebar_visible.then(|| (sidebar_width as f32).max(220.));
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let leading_part =
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .h_full()
                .pl(px(leading))
                .pr(px(8.))
                .overflow_hidden()
                .when_some(leading_width, |d, width| d.w(px(width)))
                .children(crate::menus::menu_button(cx))
                .child(
                    div()
                        .id("toggle-sidebar")
                        .flex_none()
                        .p(px(3.))
                        .rounded(px(metrics::RADIUS_SMALL))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("toggle_sidebar", window, cx)
                        }))
                        .child(icon(
                            "panel-left",
                            metrics::ICON_LARGE,
                            if sidebar_visible {
                                p.text
                            } else {
                                p.text_secondary
                            },
                        )),
                )
                .child(
                    div()
                        .id("titlebar-workspace")
                        .flex_shrink(1.)
                        .min_w(px(30.))
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(p.text))
                        .cursor_pointer()
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("switch_workspace", window, cx)
                        }))
                        .child(workspace.name()),
                )
                .when(!workspace.branch.is_empty(), |d| {
                    d.child(
                        div()
                            .id("titlebar-branch")
                            .flex()
                            // The branch truncates before the workspace's name.
                            .flex_shrink(10.)
                            .min_w_0()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("switch_branch", window, cx)
                            }))
                            .child(icon("git-branch", 11., p.text_tertiary))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(hsla(p.text_secondary))
                                    .child(workspace.branch.clone()),
                            )
                            .children(self.branch_sync_text(cx).map(|sync| {
                                div()
                                    .flex_none()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.5))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(sync)
                            })),
                    )
                });
        div()
            .flex()
            .items_center()
            .h(px(metrics::TITLEBAR_HEIGHT))
            .pr(px(8.))
            .bg(hsla(p.chrome))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(leading_part)
            // With `sidebar_tabs`, the sidebar lists the tabs while it's open.
            .when(!tabs_in_sidebar, |d| {
                d.child(
                    tabs.child(
                        div()
                            .id("new-tab")
                            .flex_none()
                            .ml(px(4.))
                            .p(px(4.))
                            .rounded(px(metrics::RADIUS_SMALL))
                            .hover(|s| s.bg(hsla(p.hover)))
                            .on_click(
                                cx.listener(|w, _, window, cx| {
                                    w.run_command("new_tab", window, cx)
                                }),
                            )
                            .child(icon("plus", metrics::ICON, p.text_secondary)),
                    ),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .children(self.agent_inbox_button(cx))
                    .child(
                        div()
                            .id("search-field")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .h(px(24.))
                            .pl(px(8.))
                            .pr(px(4.))
                            .rounded(px(metrics::RADIUS))
                            .bg(hsla(p.raised))
                            .hover(|s| s.bg(hsla(p.pressed)))
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_tertiary))
                            .cursor_pointer()
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("command_palette", window, cx)
                            }))
                            .child(icon("search", 12., p.text_tertiary))
                            .child("Search or run a command")
                            .child(
                                // The Swift `KeyHint`.
                                div()
                                    .h(px(16.))
                                    .px(px(4.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(hsla(p.hairline_strong))
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(shortcut),
                            ),
                    )
                    .children(repo.filter(|r| r.changed > 0).map(|r| {
                        let help: gpui_kit::SharedString = format!(
                            "Review {} changed file{}",
                            r.changed,
                            if r.changed == 1 { "" } else { "s" }
                        )
                        .into();
                        div()
                            .id("diff-pill")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .h(px(24.))
                            .px(px(8.))
                            .rounded(px(metrics::RADIUS))
                            .bg(hsla(p.raised))
                            .hover(|s| s.bg(hsla(p.pressed)))
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(help.clone())
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|w, _, window, cx| {
                                w.run_command("review_changes", window, cx)
                            }))
                            .child(icon("file-diff", 12., p.text_secondary))
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.))
                                    .text_color(hsla(p.text_secondary))
                                    .child(r.changed.to_string()),
                            )
                            .child(diff_stat(r.added, r.removed, p))
                    })),
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
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let attention = workspace
            .tabs
            .iter()
            .filter(|tab| Self::tab_needs_attention(tab, cx))
            .count();
        let mut column = div().flex().flex_col();
        let row = div()
            .id(("workspace", id))
            .group("workspace")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .pl(px(if indented { 16. } else { 8. }))
            .pr(px(6.))
            .mx(px(6.))
            .rounded(px(metrics::RADIUS))
            // When its tabs are listed, the selected tab carries the highlight.
            .when(selected && !expanded, |d| d.bg(hsla(p.selection)))
            .when(!(selected && !expanded), |d| {
                d.hover(|s| s.bg(hsla(p.hover)))
            })
            .relative()
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(3.))
                        .top(px(6.))
                        .w(px(2.))
                        .h(px(14.))
                        .rounded_full()
                        .bg(hsla(p.accent)),
                )
            })
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
                    .flex_none()
                    // Shown on hover or while the tabs are listed, as Swift's.
                    .when(!expanded, |d| {
                        d.invisible().group_hover("workspace", |s| s.visible())
                    })
                    .when(tabs_in_sidebar && selected, |d| d.invisible())
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
                        10.,
                        p.text_tertiary,
                    )),
            )
            .child(icon(
                match (workspace.kind, repo.is_some()) {
                    (Kind::Scratch, _) => "square-terminal",
                    (Kind::Folder, true) => "folder-git-2",
                    (Kind::Folder, false) => "folder",
                },
                13.,
                if selected { p.accent } else { p.text_tertiary },
            ))
            .child(
                div()
                    .flex_none()
                    .max_w(px(150.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(hsla(if selected { p.text } else { p.text_secondary }))
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
                            .min_w_0()
                            .truncate()
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
                        .min_w_0()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(hsla(p.text_tertiary))
                        .child("follows tab"),
                )
            })
            .child(div().flex_1().min_w(px(4.)))
            // The badges keep their size; the name and branch truncate.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .children(self.task_chips(index, cx))
                    .children(
                        impulse_kit::listening_ports::chip(&workspace.ports).map(|chip| {
                            let help = workspace
                                .ports
                                .iter()
                                .map(|p| format!(":{} {}", p.port, p.process))
                                .collect::<Vec<_>>()
                                .join("\n");
                            div()
                                .id(("ports", id))
                                .font_family("JetBrains Mono")
                                .text_size(px(10.))
                                .text_color(hsla(p.text_tertiary))
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(help.clone())
                                        .build(window, cx)
                                })
                                .child(chip)
                        }),
                    )
                    .children(self.workspace_agent_counts(index, cx))
                    .children(repo.as_ref().filter(|r| r.behind > 0).map(|r| {
                        let behind = r.behind;
                        let upstream = r.upstream.clone().unwrap_or_default();
                        let root = r.root.clone();
                        div()
                            .id(("behind", id))
                            .font_family("JetBrains Mono")
                            .text_size(px(10.))
                            .text_color(hsla(p.text_tertiary))
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(format!(
                                    "{behind} commit{} behind {upstream}: click to pull",
                                    if behind == 1 { "" } else { "s" }
                                ))
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation();
                                let repo =
                                    crate::repository_state::RepositoryStore::state(&root, cx);
                                w.git_pull(&repo, None, window, cx);
                            }))
                            .child(format!("\u{2193}{behind}"))
                    }))
                    .children(
                        repo.as_ref()
                            .filter(|r| r.changed > 0)
                            .map(|r| crate::workbench_chrome::diff_stat(r.added, r.removed, p)),
                    )
                    .when(attention > 0, |d| {
                        // The Swift `Badge`: the count on the attention color.
                        d.child(
                            div()
                                .min_w(px(16.))
                                .h(px(15.))
                                .px(px(5.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(hsla(p.attention))
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(hsla(p.text_on_accent))
                                .child(attention.to_string()),
                        )
                    })
                    .when(
                        attention == 0 && !expanded && workspace.tabs.len() > 1,
                        |d| {
                            d.child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(workspace.tabs.len().to_string()),
                            )
                        },
                    ),
            )
            .tooltip({
                let help = self.workspace_help(index);
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                }
            });
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
                        .h(px(24.))
                        .pl(px(40.))
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
                        .when(active_tab, |d| d.bg(hsla(p.selection)))
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
                    // Swift's FlatFileRowView: 22 pt rows, 16 pt a level.
                    .h(px(22.))
                    .mx(px(4.))
                    .pl(px(4. + 16. * row.depth as f32))
                    .pr(px(8.))
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .when(selected, |d| d.bg(hsla(p.accent).opacity(0.2)))
                    .when(!selected, |d| d.hover(|style| style.bg(hsla(p.hover))))
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
                    .child(div().w(px(16.)).flex_none().flex().justify_center().child(
                        if row.is_dir {
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
                        },
                    ))
                    .child(
                        gpui_kit::img(crate::file_icons::icon_path(
                            &row.name, row.is_dir, expanded,
                        ))
                        .size(px(16.))
                        .flex_none()
                        // Right after the chevron, as Swift's row has it.
                        .ml(px(-6.)),
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
        // With the Files panel's actions there's room for the selected
        // label only; otherwise every segment shows its label (Swift's
        // DockSegmentedTabs).
        let all_labels = self.panel != SidebarPanel::Files;
        let changed = crate::repository_state::RepositoryStore::summary(&workspace.root, cx)
            .map_or(0, |r| r.changed);
        let changes_label = if changed > 0 {
            format!("Changes {changed}")
        } else {
            "Changes".to_string()
        };
        let panel_button = |id: &'static str,
                            panel: SidebarPanel,
                            name: &'static str,
                            label: String,
                            cx: &mut Context<Self>| {
            let selected = self.panel == panel;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(7.))
                .h(px(24.))
                .rounded(px(metrics::RADIUS))
                .cursor_pointer()
                .text_size(px(11.5))
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .when(selected, |d| d.bg(hsla(p.raised)))
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.panel = panel;
                    cx.notify();
                }))
                .child(icon(
                    name,
                    12.,
                    if selected { p.text } else { p.text_tertiary },
                ))
                .when(selected || all_labels, |d| {
                    d.text_color(hsla(if selected { p.text } else { p.text_secondary }))
                        .child(label)
                })
        };
        let show_hidden = settings.sidebar_show_hidden;
        let tree_action =
            |id: &'static str, name: &'static str, help: &'static str, active: bool| {
                div()
                    .id(id)
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(metrics::RADIUS_SMALL))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(p.hover)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(help).build(window, cx)
                    })
                    .child(icon(
                        name,
                        13.,
                        if active { p.accent } else { p.text_secondary },
                    ))
            };
        // Swift's WorkspacesSection: rows 1 pt apart, 6 pt above and below,
        // scrolling past 280 pt.
        let mut workspaces = div()
            .id("workspaces")
            .flex()
            .flex_col()
            .flex_none()
            .max_h(px(280.))
            .overflow_y_scroll()
            .py(px(6.))
            .gap(px(1.))
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
                        "Files".into(),
                        cx,
                    ))
                    .child(panel_button(
                        "panel-changes",
                        SidebarPanel::Changes,
                        "git-branch",
                        changes_label,
                        cx,
                    ))
                    .child(panel_button(
                        "panel-search",
                        SidebarPanel::Search,
                        "search",
                        "Search".into(),
                        cx,
                    ))
                    .when(self.panel == SidebarPanel::Files, |d| {
                        let (new_file_root, new_folder_root) = (root.clone(), root.clone());
                        d.child(div().flex_1().min_w(px(4.)))
                            .child(
                                tree_action("tree-new-file", "file-plus", "New File", false)
                                    .on_click(cx.listener(move |w, _, window, cx| {
                                        w.tree_new(&new_file_root, false, window, cx)
                                    })),
                            )
                            .child(
                                tree_action("tree-new-folder", "folder-plus", "New Folder", false)
                                    .on_click(cx.listener(move |w, _, window, cx| {
                                        w.tree_new(&new_folder_root, true, window, cx)
                                    })),
                            )
                            .child(
                                tree_action(
                                    "tree-refresh",
                                    "refresh-cw",
                                    "Refresh File Tree",
                                    false,
                                )
                                .on_click(cx.listener(|w, _, _, cx| w.reload_tree(cx))),
                            )
                            .child(
                                tree_action(
                                    "tree-collapse",
                                    "chevrons-down-up",
                                    "Collapse All Folders",
                                    false,
                                )
                                .on_click(cx.listener(
                                    |w, _, _, cx| {
                                        let workspace = w.workspace_mut();
                                        workspace.expanded_dirs.clear();
                                        workspace.save_expanded();
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                tree_action(
                                    "tree-hidden",
                                    if show_hidden { "eye" } else { "eye-off" },
                                    if show_hidden {
                                        "Hide Hidden Files"
                                    } else {
                                        "Show Hidden Files"
                                    },
                                    show_hidden,
                                )
                                .on_click(cx.listener(
                                    |w, _, _, cx| {
                                        SettingsStore::update(cx, |s| {
                                            s.sidebar_show_hidden = !s.sidebar_show_hidden
                                        });
                                        w.reload_tree(cx);
                                    },
                                )),
                            )
                    }),
            )
            .child(match self.panel {
                SidebarPanel::Changes => self.changes_panel(cx),
                SidebarPanel::Search => self.search.clone().into_any_element(),
                SidebarPanel::Files => files.into_any_element(),
            })
    }

    pub(crate) fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let workspace = self.workspace();
        let repo = crate::repository_state::RepositoryStore::summary(&workspace.root, cx);
        let mut left: Vec<AnyElement> = Vec::new();
        let mut right: Vec<AnyElement> = Vec::new();
        if self.is_restricted() {
            left.push(
                status_item(
                    "restricted",
                    Some("shield-alert"),
                    "Restricted: language servers, formatters on save and background fetch are off in this folder. Click to trust it.",
                    Some(p.warning),
                    &p,
                )
                .on_click(cx.listener(|w, _, window, cx| w.run_command("trust_folder", window, cx)))
                .child("Restricted")
                .into_any_element(),
            );
        }
        if !workspace.branch.is_empty() {
            left.push(
                status_item(
                    "status-branch",
                    Some("git-branch"),
                    "Switch branch",
                    None,
                    &p,
                )
                .on_click(
                    cx.listener(|w, _, window, cx| w.run_command("switch_branch", window, cx)),
                )
                .child(workspace.branch.clone())
                .into_any_element(),
            );
        }
        if let Some(repo) = repo.filter(|r| r.changed > 0) {
            left.push(
                status_item(
                    "status-changes",
                    Some("file-diff"),
                    "Review changes",
                    None,
                    &p,
                )
                .on_click(
                    cx.listener(|w, _, window, cx| w.run_command("review_changes", window, cx)),
                )
                .child(repo.changed.to_string())
                .child(diff_stat(repo.added, repo.removed, &p))
                .into_any_element(),
            );
        }
        left.extend(self.problems_status(cx));
        for port in workspace.ports.iter().take(3) {
            let number = port.port;
            left.push(
                status_item(
                    ("status-port", number as usize),
                    Some("globe"),
                    &format!(
                        "{} is listening on {number}: open in the browser",
                        port.process
                    ),
                    None,
                    &p,
                )
                .on_click(move |_, _, cx| cx.open_url(&format!("http://localhost:{number}")))
                .child(format!(":{number}"))
                .into_any_element(),
            );
        }
        if workspace.ports.len() > 3 {
            let help = workspace.ports[3..]
                .iter()
                .map(|p| format!(":{} {}", p.port, p.process))
                .collect::<Vec<_>>()
                .join("\n");
            left.push(
                status_item("status-more-ports", None, &help, None, &p)
                    .child(format!("+{}", workspace.ports.len() - 3))
                    .into_any_element(),
            );
        }
        match self.focused_surface() {
            Some(Surface::Editor(editor)) => {
                let editor = editor.read(cx);
                let (line, column) = editor.cursor(cx);
                if let Some(status) = editor.vim_status(cx) {
                    right.push(
                        div()
                            .font_family("JetBrains Mono")
                            .child(status)
                            .into_any_element(),
                    );
                }
                if let Some(notice) = editor.notice.clone() {
                    right.push(
                        div()
                            .text_color(hsla(p.warning))
                            .child(notice)
                            .into_any_element(),
                    );
                }
                right.push(
                    status_item("status-line", None, "Go to Line", None, &p)
                        .on_click(
                            cx.listener(|w, _, window, cx| w.run_command("go_to_line", window, cx)),
                        )
                        .child(format!("Ln {line}, Col {column}"))
                        .into_any_element(),
                );
                let indent = if editor.use_spaces {
                    format!("Spaces: {}", editor.tab_size)
                } else {
                    format!("Tab Size: {}", editor.tab_size)
                };
                right.push(
                    status_item("status-indent", None, "Indentation", None, &p)
                        .child(indent)
                        .into_any_element(),
                );
                right.push(
                    status_item("status-encoding", None, "Encoding", None, &p)
                        .child("UTF-8")
                        .into_any_element(),
                );
                right.push(
                    status_item("status-language", Some("code"), "Language", None, &p)
                        .child(impulse_kit::uri::editor_language_for_path(&editor.path))
                        .into_any_element(),
                );
                if impulse_kit::markdown_preview::is_previewable_file(
                    &editor.path.to_string_lossy(),
                ) {
                    let previewing = editor.preview.is_some();
                    right.push(
                        status_item(
                            "status-preview",
                            Some(if previewing { "eye-off" } else { "eye" }),
                            if previewing {
                                "Hide preview"
                            } else {
                                "Show preview"
                            },
                            previewing.then_some(p.accent),
                            &p,
                        )
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.run_command("toggle_markdown_preview", window, cx)
                        }))
                        .child("Preview")
                        .into_any_element(),
                    );
                }
            }
            Some(Surface::Terminal(terminal)) => {
                let cwd = terminal.read(cx).cwd.clone();
                if !cwd.is_empty() {
                    left.push(
                        status_item("status-cwd", Some("folder"), &cwd, None, &p)
                            .child(abbreviate_home(&cwd))
                            .into_any_element(),
                    );
                }
                right.push(
                    status_item("status-shell", Some("terminal"), "Shell", None, &p)
                        .child(crate::workspace::shell_name())
                        .into_any_element(),
                );
            }
            // Review, History and tools have no shell or cursor, but the
            // folder still follows the tab (Swift's `currentCwd`).
            Some(surface) => {
                let folder = surface.directory(cx).or_else(|| {
                    (self.workspace().kind == Kind::Folder).then(|| self.workspace().root.clone())
                });
                if let Some(folder) = folder {
                    let cwd = folder.to_string_lossy().into_owned();
                    left.push(
                        status_item("status-cwd", Some("folder"), &cwd, None, &p)
                            .child(abbreviate_home(&cwd))
                            .into_any_element(),
                    );
                }
            }
            None => {}
        }
        if let Some(notice) = self.notice.clone() {
            left.push(
                div()
                    .px(px(6.))
                    .text_color(hsla(p.warning))
                    .child(notice)
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(metrics::STATUS_BAR_HEIGHT))
            .px(px(6.))
            .bg(hsla(p.chrome))
            .border_t_1()
            .border_color(hsla(p.hairline))
            .text_size(px(11.))
            .text_color(hsla(p.text_secondary))
            .children(left)
            .child(div().flex_1().min_w(px(8.)))
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
