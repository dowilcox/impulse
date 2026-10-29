//! Tasks in the sidebar (the task parts of the Swift `WorkspacesSection`):
//! a repository's workspaces grouped under its name, a task's "merged"
//! chip and its overlap chip (the files it changes that another workspace
//! of the repository changes too, with a conflict check), its summary in
//! the row's tooltip, and the row's context menu with the task commands.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, AnyElement, Context, FontWeight, Window};
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::git::task_overlap::Pair;
use impulse_kit::git::task_registry;

use crate::context_menu::{MenuSpec, RightClickMenu as _};
use crate::tasks;
use crate::workbench::Workbench;
use crate::workbench_chrome::{chrome_button, hsla, icon, ButtonKind};
use crate::workspace::{Kind, Workspace};

/// A context menu entry: a title and what it does to the workspace (by id),
/// or a separator.
type MenuAction = Rc<dyn Fn(&mut Workbench, usize, &mut Window, &mut Context<Workbench>)>;

fn action(
    f: impl Fn(&mut Workbench, usize, &mut Window, &mut Context<Workbench>) + 'static,
) -> Option<MenuAction> {
    Some(Rc::new(f))
}

impl Workbench {
    /// The task commands from the palette, menus and keys.
    pub(crate) fn run_task_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            "new_task" => self.present_new_task(None, false, window, cx),
            "finish_task" => self.open_finish_task(None, window, cx),
            "archive_task" => {
                let index = self.active;
                self.archive_task(index, window, cx)
            }
            "new_task_from_branch" => self.show_palette("task:", window, cx),
            _ => {}
        }
    }

    /// The sidebar's groups: workspaces in their order, those of one
    /// repository gathered at the first one's place. Each group: its
    /// header's name (a repository with two or more workspaces), its rows.
    pub(crate) fn workspace_groups(&self) -> Vec<(Option<String>, Vec<usize>)> {
        let indices: Vec<usize> = (0..self.workspaces.len()).collect();
        impulse_kit::grouped_order::groups(&indices, |i| tasks::group_key(&self.workspaces[*i]))
            .into_iter()
            .map(|group| {
                let name = (group.len() > 1)
                    .then(|| self.workspaces[group[0]].task.repository_name())
                    .flatten();
                (name, group)
            })
            .collect()
    }

    /// Put the workspaces in the order of `order` (indexes into them),
    /// keeping the same one shown. The order is saved with the session.
    fn reorder_workspaces(&mut self, order: Vec<usize>, cx: &mut Context<Self>) {
        if order.len() != self.workspaces.len() {
            return;
        }
        let active = self.workspaces[self.active].id;
        let mut old: Vec<Option<Workspace>> = std::mem::take(&mut self.workspaces)
            .into_iter()
            .map(Some)
            .collect();
        self.workspaces = order.into_iter().filter_map(|i| old[i].take()).collect();
        self.active = self
            .workspaces
            .iter()
            .position(|w| w.id == active)
            .unwrap_or(0);
        cx.notify();
    }

    /// Where workspace `index` lands one row up (-1) or down (+1) as the
    /// sidebar shows them (a repository's workspaces together).
    fn moved_order(&self, index: usize, step: isize) -> Option<Vec<usize>> {
        let indices: Vec<usize> = (0..self.workspaces.len()).collect();
        impulse_kit::grouped_order::moving(&indices, index, step, |i| {
            tasks::group_key(&self.workspaces[*i])
        })
    }

    pub(crate) fn move_workspace(&mut self, index: usize, step: isize, cx: &mut Context<Self>) {
        if let Some(order) = self.moved_order(index, step) {
            self.reorder_workspaces(order, cx);
        }
    }

    /// A workspace row dropped on another: among its repository's rows when
    /// they share one, else its whole group to the target's group.
    pub(crate) fn drop_workspace(&mut self, dragged: u64, target: u64, cx: &mut Context<Self>) {
        let find = |id: u64| self.workspaces.iter().position(|w| w.id == id);
        let (Some(from), Some(to)) = (find(dragged), find(target)) else {
            return;
        };
        if from == to {
            return;
        }
        let indices: Vec<usize> = (0..self.workspaces.len()).collect();
        let key = |i: &usize| tasks::group_key(&self.workspaces[*i]);
        let groups = impulse_kit::grouped_order::groups(&indices, key);
        let group_of = |i: usize| groups.iter().position(|g| g.contains(&i));
        let (Some(from_group), Some(to_group)) = (group_of(from), group_of(to)) else {
            return;
        };
        let order = if from_group == to_group {
            let position = groups[to_group].iter().position(|&i| i == to).unwrap_or(0);
            impulse_kit::grouped_order::moving_to_position(&indices, from, position, key)
        } else {
            impulse_kit::grouped_order::moving_group(&indices, from, to_group, key)
        };
        if let Some(order) = order {
            self.reorder_workspaces(order, cx);
        }
    }

    /// A repository's header over its workspaces.
    /// `first` is the group's first workspace: a right-click offers Archive
    /// Merged Tasks… while the repository has some, as Swift's header does.
    pub(crate) fn repository_header(
        &self,
        name: String,
        first: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let header = div()
            .id(gpui_kit::SharedString::from(format!("repository-{name}")))
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(14.))
            .h(px(22.))
            .child(icon("git-fork", 11., p.text_tertiary))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(hsla(p.text_secondary))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(name),
            );
        let Some(workspace) = self
            .workspaces
            .get(first)
            .filter(|w| w.task.repository_has_merged_tasks)
        else {
            return header.into_any_element();
        };
        let (id, workbench) = (workspace.id, cx.entity().downgrade());
        header
            .right_click_menu(move |_, _| {
                let workbench = workbench.clone();
                MenuSpec::new().item("Archive Merged Tasks…", move |window, cx| {
                    let _ = workbench.update(cx, |w, cx| {
                        if let Some(index) = w.workspaces.iter().position(|ws| ws.id == id) {
                            w.present_archive_merged_tasks(index, window, cx)
                        }
                    });
                })
            })
            .into_any_element()
    }

    /// The row's task chips: "merged", and the overlap count.
    pub(crate) fn task_chips(&self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = &self.palette;
        let workspace = &self.workspaces[index];
        let task = &workspace.task;
        let mut chips = Vec::new();
        if task.merged {
            chips.push(
                div()
                    .id(("merged", workspace.id))
                    .font_family("JetBrains Mono")
                    .text_size(px(10.))
                    .text_color(hsla(p.success))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(
                            "Its branch is merged into its base: archive it when you're done",
                        )
                        .build(window, cx)
                    })
                    .child("merged")
                    .into_any_element(),
            );
        }
        if !task.overlaps.is_empty() {
            let path = task_registry::canonical(&workspace.root.to_string_lossy());
            let mut shared: Vec<&String> = task.overlaps.iter().flat_map(|p| &p.files).collect();
            shared.sort();
            shared.dedup();
            let help = task
                .overlaps
                .iter()
                .map(|pair| {
                    let count = pair.files.len();
                    format!(
                        "Shares {count} file{} with {}",
                        if count == 1 { "" } else { "s" },
                        pair.other(&path).name
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let id = workspace.id;
            chips.push(
                div()
                    .id(("overlaps", id))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .cursor_pointer()
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(help.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |workbench, _, window, cx| {
                        cx.stop_propagation();
                        workbench.present_overlaps(id, window, cx)
                    }))
                    .child(icon("triangle-alert", 10., p.warning))
                    .child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(10.))
                            .text_color(hsla(p.warning))
                            .child(shared.len().to_string()),
                    )
                    .into_any_element(),
            );
        }
        chips
    }

    /// The row's tooltip: its folder, branch, task summary and state.
    pub(crate) fn workspace_help(&self, index: usize) -> String {
        let workspace = &self.workspaces[index];
        let mut lines = vec![match workspace.kind {
            Kind::Scratch => "Scratch: the file tree follows the active tab".to_string(),
            Kind::Folder => workspace.root.display().to_string(),
        }];
        if !workspace.branch.is_empty() {
            lines.push(format!("Branch: {}", workspace.branch));
        }
        if let Some(summary) = &workspace.task.summary {
            lines.push(summary.clone());
        }
        if workspace.task.merged {
            lines.push("Merged into its base".into());
        } else if workspace.task.upstream_gone {
            lines
                .push("Its branch was deleted on the remote; it may have been merged there".into());
        }
        let tabs = workspace.tabs.len();
        lines.push(format!("{tabs} tab{}", if tabs == 1 { "" } else { "s" }));
        lines.join("\n")
    }

    /// The row's context menu, as the Swift sidebar's (`WorkspaceRow`'s
    /// `contextMenu`; the parts the Rust app has).
    pub(crate) fn workspace_menu(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl Fn(&mut Window, &mut gpui_kit::App) -> MenuSpec + 'static {
        let workspace = &self.workspaces[index];
        let task = &workspace.task;
        let in_repository = task.common_dir.is_some();
        // Title, enabled, and what it does; `None` is a separator.
        let mut items: Vec<(String, bool, Option<MenuAction>)> = Vec::new();
        let mut push = |title: &str, enabled: bool, run: Option<MenuAction>| {
            items.push((title.to_string(), enabled, run));
        };
        if task.merged {
            push(
                "Archive Task…",
                true,
                action(|w, i, window, cx| w.archive_task(i, window, cx)),
            );
            push("", false, None);
        } else if task.is_task {
            push(
                "Finish Task…",
                true,
                action(|w, i, window, cx| w.open_finish_task(Some(i), window, cx)),
            );
            push("", false, None);
        }
        if in_repository {
            push(
                "New Task…",
                true,
                action(|w, i, window, cx| w.present_new_task(Some(i), false, window, cx)),
            );
            let changed = crate::repository_state::RepositoryStore::summary(&workspace.root, cx)
                .is_some_and(|r| r.changed > 0);
            if !task.is_task && changed {
                push(
                    "Move Changes to New Task…",
                    true,
                    action(|w, i, window, cx| w.present_new_task(Some(i), true, window, cx)),
                );
            }
            push(
                "Project Setup…",
                true,
                action(|w, i, window, cx| w.open_project_setup(Some(i), None, window, cx)),
            );
        }
        push(
            "Open Folder as Workspace…",
            true,
            action(|w, _, window, cx| w.run_command("open_workspace", window, cx)),
        );
        push("", false, None);
        push(
            "Rename…",
            true,
            action(|w, i, window, cx| w.rename_workspace(i, window, cx)),
        );
        if workspace.kind == Kind::Folder {
            push(
                crate::file_tree_actions::REVEAL,
                true,
                action(|w, i, _, cx| {
                    crate::file_tree_actions::reveal(&w.workspaces[i].root.clone(), cx)
                }),
            );
            push(
                "Copy Path",
                true,
                action(|w, i, _, cx| {
                    let path = w.workspaces[i].root.to_string_lossy().into_owned();
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path));
                }),
            );
        }
        push("", false, None);
        // The active workspace's tabs stay listed while the sidebar shows
        // tabs: nothing to hide or show.
        if !(self.tabs_in_sidebar(cx) && index == self.active) {
            push(
                if workspace.expanded {
                    "Hide Tabs"
                } else {
                    "Show Tabs"
                },
                true,
                action(|w, i, _, cx| {
                    w.workspaces[i].expanded = !w.workspaces[i].expanded;
                    cx.notify();
                }),
            );
        }
        push(
            "Move Up",
            self.moved_order(index, -1).is_some(),
            action(|w, i, _, cx| w.move_workspace(i, -1, cx)),
        );
        push(
            "Move Down",
            self.moved_order(index, 1).is_some(),
            action(|w, i, _, cx| w.move_workspace(i, 1, cx)),
        );
        push("", false, None);
        if task.is_task && !task.merged {
            push(
                "Archive Task…",
                true,
                action(|w, i, window, cx| w.archive_task(i, window, cx)),
            );
        }
        if task.repository_has_merged_tasks {
            push(
                "Archive Merged Tasks…",
                true,
                action(|w, i, window, cx| w.present_archive_merged_tasks(i, window, cx)),
            );
        }
        if self.can_close_workspace(index) {
            push(
                "Close Workspace",
                true,
                action(|w, i, window, cx| w.close_workspace(i, window, cx)),
            );
        }
        let (id, workbench) = (workspace.id, cx.entity().downgrade());
        move |_, _| {
            let mut menu = MenuSpec::new();
            for (title, enabled, run) in &items {
                let Some(run) = run.clone() else {
                    menu = menu.separator();
                    continue;
                };
                let workbench = workbench.clone();
                menu = menu.item_enabled(title.clone(), *enabled, move |window, cx| {
                    let _ = workbench.update(cx, |w, cx| {
                        if let Some(index) = w.workspaces.iter().position(|ws| ws.id == id) {
                            run(w, index, window, cx)
                        }
                    });
                });
            }
            menu
        }
    }

    /// The overlap chip's list: each workspace this one shares files with,
    /// the files (opened in this workspace on click), and a conflict check.
    fn present_overlaps(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.iter().find(|w| w.id == id) else {
            return;
        };
        let path = task_registry::canonical(&workspace.root.to_string_lossy());
        let main = tasks::main_checkout_root(&path);
        let workbench = cx.entity().downgrade();
        let view = cx.new(|_| OverlapList {
            palette: self.palette.clone(),
            workbench,
            workspace_id: id,
            path,
            main: task_registry::canonical(&main),
            pairs: workspace.task.overlaps.clone(),
            conflicts: HashMap::new(),
            scroll: gpui_kit::ScrollHandle::new(),
        });
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .w(px(420.))
                .title("Changed in more than one workspace")
                .child(view.clone())
        });
    }
}

/// What the overlap chip opens.
struct OverlapList {
    palette: ChromePalette,
    workbench: gpui_kit::WeakEntity<Workbench>,
    workspace_id: u64,
    path: String,
    /// The repository's main checkout, whose changes can move into a task.
    main: String,
    pairs: Vec<Pair>,
    /// Conflict checks by pair: `None` while running, the files once done.
    conflicts: HashMap<String, Option<Vec<String>>>,
    /// The list's scroll, for its scrollbar.
    scroll: gpui_kit::ScrollHandle,
}

impl OverlapList {
    fn check(&mut self, pair: Pair, window: &mut Window, cx: &mut Context<Self>) {
        let key = pair.key();
        self.conflicts.insert(key.clone(), None);
        let work = cx
            .background_executor()
            .spawn(async move { tasks::conflicts(&pair).unwrap_or_default() });
        cx.spawn_in(window, async move |list, cx| {
            let found = work.await;
            let _ = list.update(cx, |list, cx| {
                list.conflicts.insert(key, Some(found));
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for OverlapList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let mut column = div()
            .id("overlaps")
            .flex()
            .flex_col()
            .gap(px(14.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        for (n, pair) in self.pairs.clone().into_iter().enumerate() {
            let other = pair.other(&self.path).clone();
            let found = self.conflicts.get(&pair.key()).cloned();
            let mut files = div().flex().flex_col().gap(px(2.));
            for (f, file) in pair.files.iter().take(20).enumerate() {
                let conflicting = found
                    .as_ref()
                    .and_then(|f| f.as_ref())
                    .is_some_and(|c| c.contains(file));
                let target = Path::new(&self.path).join(file);
                let (workbench, id) = (self.workbench.clone(), self.workspace_id);
                files = files.child(
                    div()
                        .id(("overlap-file", n * 100 + f))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let target = target.clone();
                            let _ = workbench.update(cx, |w, cx| {
                                if let Some(index) = w.workspaces.iter().position(|ws| ws.id == id)
                                {
                                    w.activate_workspace(index, window, cx);
                                }
                                w.open_file(&target, window, cx)
                            });
                        })
                        .when(conflicting, |d| {
                            d.child(icon("triangle-alert", 10., p.danger))
                        })
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(11.))
                                .text_color(hsla(if conflicting {
                                    p.danger
                                } else {
                                    p.text_secondary
                                }))
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(file.clone()),
                        ),
                );
            }
            if pair.files.len() > 20 {
                files = files.child(
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.text_tertiary))
                        .child(format!("…and {} more", pair.files.len() - 20)),
                );
            }
            let status: AnyElement = match found {
                None => chrome_button(
                    ("overlap-check", n),
                    "Check for Conflicts",
                    None,
                    ButtonKind::Secondary,
                    false,
                    &p,
                )
                .on_click(
                    cx.listener(move |list, _, window, cx| list.check(pair.clone(), window, cx)),
                )
                .into_any_element(),
                Some(None) => div()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child("Checking…")
                    .into_any_element(),
                Some(Some(files)) => div()
                    .text_size(px(11.))
                    .text_color(hsla(if files.is_empty() {
                        p.success
                    } else {
                        p.danger
                    }))
                    .child(if files.is_empty() {
                        "These would merge without conflicts.".to_string()
                    } else {
                        format!("{} would conflict.", files.len())
                    })
                    .into_any_element(),
            };
            let offers_move = other.path == self.main || self.path == self.main;
            let (workbench, id) = (self.workbench.clone(), self.workspace_id);
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Also changed in {}", other.name)),
                    )
                    .child(files)
                    .child(div().flex().items_center().gap(px(8.)).child(status).when(
                        offers_move,
                        |d| {
                            d.child(
                                chrome_button(
                                    ("overlap-move", n),
                                    "Move to New Task…",
                                    None,
                                    ButtonKind::Ghost,
                                    false,
                                    &p,
                                )
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = workbench.update(cx, |w, cx| {
                                        let index = w.workspaces.iter().position(|ws| ws.id == id);
                                        w.present_new_task(index, true, window, cx)
                                    });
                                }),
                            )
                        },
                    )),
            );
        }
        div()
            .relative()
            .text_color(hsla(p.text))
            .rounded(px(metrics::RADIUS))
            .child(column)
            .vertical_scrollbar(&self.scroll)
    }
}

/// A sidebar workspace row being dragged to a new place.
#[derive(Clone)]
pub(crate) struct DraggedWorkspace {
    pub id: u64,
    pub name: String,
}
