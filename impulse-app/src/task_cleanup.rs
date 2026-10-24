//! Removing tasks' folders and bringing them back (the Swift `TaskCleanup`
//! and the archiving half of `TaskWorkflows`): Archive Task and Archive
//! Merged Tasks. Uncommitted work goes into a safety snapshot first, the
//! archive script runs, Impulse's own files for the task go too, the folder
//! loses its trust, and Undo brings it all back. A task holding a git
//! repository of its own isn't archived: removing it would delete that
//! repository for good.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, WindowExt as _};
use gpui_kit::{div, prelude::*, px, Context, FontWeight, Window};
use impulse_git::safety::{self, SafetySnapshot};
use impulse_git::{branches, ignored, landing, operations, snapshot};
use impulse_kit::chrome::ChromePalette;
use impulse_kit::git::task_registry::{self, TaskRecord};
use impulse_kit::trust::WorkspaceTrust;

use crate::tasks::{self, confirm, toast, Run, Then, ToastAction, ToastKind};
use crate::workbench::Workbench;
use crate::workbench_chrome::{abbreviate_home, hsla};

/// A task whose folder was removed, with what Undo needs.
#[derive(Clone)]
pub(crate) struct ArchivedTask {
    pub root: String,
    pub branch: String,
    pub main_root: String,
    pub snapshot: Option<SafetySnapshot>,
    pub record: Option<TaskRecord>,
    /// The folder was trusted in its own right (Undo trusts it again).
    pub was_trusted: bool,
    /// Set when its branch was deleted too: the commit to recreate it at.
    pub deleted_branch_commit: Option<String>,
}

/// What `remove_task` did.
pub(crate) struct Removed {
    pub result: Result<ArchivedTask, String>,
    pub script_failed: bool,
}

fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Remove a task's folder (off the main thread). Uncommitted work is
/// snapshotted first, and a task whose snapshot can't be made isn't
/// removed; then `archive_script` runs, and a failure there doesn't stop it.
/// A task holding a repository of its own isn't removed either. The
/// folder's trust goes with it, so a task at the same path later starts out
/// restricted.
pub(crate) fn remove_task(
    root: &str,
    branch: &str,
    dirty: bool,
    archive_script: Option<&str>,
    trust: &WorkspaceTrust,
) -> Removed {
    let failed = |message: String| Removed {
        result: Err(message),
        script_failed: false,
    };
    // Run git from the main checkout: the worktree is about to vanish.
    let main_root = branches::common_git_directory(root)
        .and_then(|c| {
            Path::new(&c)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| root.to_string());
    let trusted = trust.is_trusted(root);
    // A snapshot records a repository inside the task only by its commit
    // id: removing the folder would lose its files and history. Not knowing
    // counts as having one.
    match ignored::nested_repositories(root, trusted) {
        Some(nested) if nested.is_empty() => {}
        nested => {
            return failed(format!(
                "The task wasn't archived. {}",
                nested_repository_message(nested.as_deref())
            ))
        }
    }
    let changed = |root: &str| {
        snapshot::snapshot(root, false)
            .map(|s| s.changed_file_count())
            .unwrap_or(0)
    };
    // Look again: closing the workspace may have just saved files.
    let dirty = dirty || changed(root) > 0;
    let mut saved = None;
    if dirty {
        match safety::create(&format!("archive {branch}"), root, safety::OPLOG_PREFIX) {
            Ok(snapshot) => saved = Some(snapshot),
            Err(error) => {
                return failed(format!(
                    "The task wasn't archived: its uncommitted files couldn't be saved first. {}",
                    error.message()
                ))
            }
        }
    }
    // After the snapshot, so Undo brings back what the script changes too.
    let mut script_failed = false;
    if let Some(script) = archive_script {
        script_failed = !tasks::run_script(script, root, &tasks::identity(root));
    }
    // What the script left behind (a log, say) mustn't stop the removal.
    let force = dirty || changed(root) > 0;
    let folder = impulse_kit::trust::normalize(root);
    let was_trusted = trust.trusting_folder(&folder).as_deref() == Some(folder.as_str());
    if let Err(error) = operations::remove_worktree(root, force, &main_root) {
        return Removed {
            result: Err(error.message()),
            script_failed,
        };
    }
    let record = tasks::remove_record(root, &main_root);
    // Impulse's own files for it (a Compose override) go with the folder.
    if let Some(common) = branches::common_git_directory(&main_root) {
        let _ = std::fs::remove_dir_all(tasks::task_folder(&name_of(root), &common));
    }
    if was_trusted {
        trust.revoke(&folder);
    }
    Removed {
        result: Ok(ArchivedTask {
            root: root.to_string(),
            branch: branch.to_string(),
            main_root,
            snapshot: saved,
            record,
            was_trusted,
            deleted_branch_commit: None,
        }),
        script_failed,
    }
}

/// Why a task holding repositories of its own (`None`: one that couldn't
/// be checked) isn't archived.
pub(crate) fn nested_repository_message(nested: Option<&[String]>) -> String {
    let Some(nested) = nested else {
        return "Impulse couldn't check whether its folder holds a git repository of its own (a clone \
                or a submodule), which removing it would delete for good."
            .into();
    };
    let shown = nested
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
        + if nested.len() > 3 { ", …" } else { "" };
    let one = nested.len() == 1;
    format!(
        "It holds {} of its own ({shown}), which removing the folder would delete for good, with any \
         commits not pushed. Move {} out of the task first (a submodule: push its commits, then `git \
         submodule deinit` it).",
        if one { "a git repository" } else { "git repositories" },
        if one { "it" } else { "them" },
    )
}

/// "Ignored files in it (.env, node_modules/, …) are deleted, and Undo
/// can't bring them back." `None` when there are none; `ignored` `None`
/// when they couldn't be listed.
pub(crate) fn ignored_files_warning(ignored: Option<&[String]>, undo: &str) -> Option<String> {
    let Some(ignored) = ignored else {
        return Some(format!(
            "Impulse couldn't list the ignored files in it; any there are deleted, and {undo} can't bring them back."
        ));
    };
    if ignored.is_empty() {
        return None;
    }
    let shown = ignored
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
        + if ignored.len() > 3 { ", …" } else { "" };
    Some(format!(
        "Ignored files in it ({shown}) are deleted, and {undo} can't bring them back."
    ))
}

/// Bring an archived task back (off the main thread): its branch if it was
/// deleted, its folder, its uncommitted files and its record. The error
/// says whether the folder is back (only its files aren't).
pub(crate) fn restore_archived(task: &ArchivedTask) -> Result<(), (String, bool)> {
    if let Some(commit) = &task.deleted_branch_commit {
        operations::create_branch(&task.branch, Some(commit), false, &task.main_root)
            .map_err(|e| (e.message(), false))?;
    }
    operations::add_worktree(
        &task.root,
        &task.branch,
        false,
        None,
        false,
        &task.main_root,
    )
    .map_err(|e| (e.message(), false))?;
    if let Some(record) = &task.record {
        tasks::restore_record(record, &task.main_root);
    }
    if let Some(snapshot) = &task.snapshot {
        if let Err(error) = safety::restore(snapshot, &[] as &[&str], &task.root) {
            return Err((
                format!(
                    "{} is back, but its uncommitted files couldn't be put back. They're kept in the snapshot {}. {}",
                    task.branch,
                    snapshot.ref_name,
                    error.message()
                ),
                true,
            ));
        }
    }
    Ok(())
}

thread_local! {
    /// Workspaces an Archive Task is under way for: a second request waits.
    static ARCHIVING: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

fn set_archiving(id: u64, on: bool) -> bool {
    ARCHIVING.with(|a| {
        let mut a = a.borrow_mut();
        if on {
            a.insert(id)
        } else {
            a.remove(&id)
        }
    })
}

/// What Archive Task looks at before asking.
struct Look {
    nested: Option<Vec<String>>,
    ignored: Option<Vec<String>>,
    branch: Option<String>,
    dirty: usize,
    unpushed: bool,
}

impl Workbench {
    /// "Archive Task...": close the workspace, snapshot any uncommitted
    /// work, remove the worktree folder, keep the branch. Undo brings it
    /// back.
    pub(crate) fn archive_task(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if !workspace.task.is_task {
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Only task worktrees can be archived.".into(),
                None,
                None,
            );
        }
        let id = workspace.id;
        if !set_archiving(id, true) {
            return;
        }
        let root = workspace.root.to_string_lossy().into_owned();
        let name = workspace.name();
        let trusted = self.trust.is_trusted(&root);
        let work = {
            let root = root.clone();
            cx.background_executor().spawn(async move {
                let nested = ignored::nested_repositories(&root, trusted);
                let ignored = if nested.as_ref().is_some_and(|n| n.is_empty()) {
                    ignored::ignored_entry_list(&root, trusted)
                } else {
                    Some(Vec::new())
                };
                let snapshot = snapshot::snapshot(&root, false);
                Look {
                    nested,
                    ignored,
                    branch: snapshot.as_ref().and_then(|s| s.branch.clone()),
                    dirty: snapshot
                        .as_ref()
                        .map(|s| s.changed_file_count())
                        .unwrap_or(0),
                    unpushed: snapshot
                        .as_ref()
                        .is_some_and(|s| s.ahead > 0 || s.upstream.is_none()),
                }
            })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let look = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                if !workbench.workspaces.iter().any(|w| w.id == id) {
                    set_archiving(id, false);
                    return;
                }
                // One that couldn't be checked counts as there.
                if !look.nested.as_ref().is_some_and(|n| n.is_empty()) {
                    set_archiving(id, false);
                    return toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        format!("{name} wasn't archived."),
                        Some(nested_repository_message(look.nested.as_deref())),
                        None,
                    );
                }
                workbench.confirm_archive_task(id, root, name, look, window, cx);
            });
        })
        .detach();
    }

    fn confirm_archive_task(
        &mut self,
        id: u64,
        root: String,
        name: String,
        look: Look,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let branch = look.branch.clone().unwrap_or_else(|| name_of(&root));
        let dirty = look.dirty;
        let mut message = format!(
            "The folder {} is removed; branch {branch} is kept.",
            abbreviate_home(&root)
        );
        if dirty > 0 {
            message += &format!(
                " {dirty} uncommitted file{} will be saved in a snapshot that Undo restores.",
                if dirty == 1 { "" } else { "s" }
            );
        }
        if let Some(warning) = ignored_files_warning(look.ignored.as_deref(), "Undo") {
            message += &format!(" {warning}");
        }
        if look.unpushed {
            message += " The branch has commits that aren't pushed.";
        }
        let workbench = cx.entity().downgrade();
        confirm(
            window,
            cx,
            format!("Archive {name}?"),
            message,
            "Archive",
            "Cancel",
            true,
            move |yes, window, cx| {
                let _ = workbench.update(cx, |workbench, cx| {
                    if !yes || !workbench.still_archivable(id, &root, &name, window, cx) {
                        set_archiving(id, false);
                        return;
                    }
                    // The project's archive script (once trusted) runs before
                    // removal; in a folder that isn't trusted, only when the
                    // user says so.
                    let root2 = root.clone();
                    workbench.trust_project_config(
                        &root,
                        window,
                        cx,
                        Box::new(move |workbench, config, window, cx| {
                            let script = config.and_then(|c| c.archive_script);
                            let root3 = root2.clone();
                            let shown = branch.clone();
                            workbench.confirm_archive_script(
                                script,
                                &root2,
                                &shown,
                                window,
                                cx,
                                Box::new(move |workbench, script, window, cx| {
                                    if !workbench.still_archivable(id, &root3, &name, window, cx) {
                                        set_archiving(id, false);
                                        return;
                                    }
                                    // Close the workspace first (unsaved files
                                    // keep it open), then remove the folder.
                                    let closed = workbench.close_workspace_for_task(id, window, cx);
                                    set_archiving(id, false);
                                    if closed {
                                        workbench.remove_task_worktree(
                                            root3,
                                            branch,
                                            dirty > 0,
                                            script,
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                            );
                        }),
                    );
                });
            },
        );
    }

    /// Whether the task workspace `id` is still open on `root` with its
    /// folder there, saying so when the folder went away.
    fn still_archivable(
        &mut self,
        id: u64,
        root: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let open = self
            .workspaces
            .iter()
            .any(|w| w.id == id && w.task.is_task && w.root.to_string_lossy() == root);
        if open && Path::new(root).is_dir() {
            return true;
        }
        if self.workspaces.iter().any(|w| w.id == id) {
            toast(
                window,
                cx,
                ToastKind::Info,
                format!("{name} wasn't archived: its folder is gone."),
                None,
                None,
            );
        }
        false
    }

    /// The archive script to run in the task at `root`: as it is in a
    /// trusted folder; in one that isn't (a task made from someone else's
    /// branch), only once the user says so, every time.
    pub(crate) fn confirm_archive_script(
        &mut self,
        script: Option<String>,
        root: &str,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Then<Option<String>>,
    ) {
        match script {
            Some(_) if !self.trust.is_trusted(root) => {
                self.confirm_branch_script(branch, root, "archive", window, cx, then)
            }
            script => then(self, script, window, cx),
        }
    }

    fn remove_task_worktree(
        &mut self,
        root: String,
        branch: String,
        dirty: bool,
        script: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trust = self.trust.clone();
        let work = {
            let (root, branch) = (root.clone(), branch.clone());
            cx.background_executor()
                .spawn(async move { remove_task(&root, &branch, dirty, script.as_deref(), &trust) })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let removed = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                if removed.script_failed {
                    toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        "The archive script failed; archiving anyway.".into(),
                        None,
                        None,
                    );
                }
                match removed.result {
                    Err(error) => toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        "Couldn't archive the task".into(),
                        Some(error),
                        None,
                    ),
                    Ok(archived) => {
                        workbench.refresh_task_states(cx);
                        let undo = workbench.restore_action(vec![archived], cx);
                        toast(
                            window,
                            cx,
                            ToastKind::Success,
                            format!("Archived {branch}. The branch is kept."),
                            None,
                            Some(("Undo", undo)),
                        );
                    }
                }
            });
        })
        .detach();
    }

    /// The toast action that brings `tasks` back.
    pub(crate) fn restore_action(&self, tasks: Vec<ArchivedTask>, cx: &mut Context<Self>) -> Run {
        let workbench = cx.entity().downgrade();
        Rc::new(move |window, cx| {
            let tasks = tasks.clone();
            let _ = workbench.update(cx, |workbench, cx| {
                workbench.restore_tasks(tasks, window, cx)
            });
        })
    }

    /// Bring archived tasks back and reopen their workspaces.
    pub(crate) fn restore_tasks(
        &mut self,
        tasks: Vec<ArchivedTask>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let work = cx.background_executor().spawn(async move {
            tasks
                .into_iter()
                .map(|task| {
                    let result = restore_archived(&task);
                    (task, result)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let results = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                for (task, result) in results {
                    if let Err((error, folder_restored)) = result {
                        toast(
                            window,
                            cx,
                            ToastKind::Warning,
                            format!("Couldn't restore {}", task.branch),
                            Some(error),
                            None,
                        );
                        // Only a folder Impulse made again.
                        if !folder_restored {
                            continue;
                        }
                    }
                    // Trusted again only if it was before; someone else's
                    // branch stays restricted.
                    if task.was_trusted {
                        workbench.trust.trust(&task.root);
                    } else if task.record.as_ref().and_then(|r| r.from_remote) == Some(true) {
                        workbench.trust.restrict(&task.root);
                    }
                    workbench.open_task_workspace(Path::new(&task.root), None, window, cx);
                }
            });
        })
        .detach();
    }

    // MARK: Archive Merged Tasks

    /// "Archive Merged Tasks...": the repository's tasks whose branch is
    /// merged into their base, archived together with one Undo.
    pub(crate) fn present_archive_merged_tasks(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(main) = self.task_repository(Some(index)) else {
            return;
        };
        let sheet = cx.new(|_| MergedTasksSheet {
            palette: self.palette.clone(),
            rows: Vec::new(),
            is_loading: true,
            delete_branches: false,
            delete_remote_branches: false,
        });
        let trust = self.trust.clone();
        let work = {
            let main = main.clone();
            cx.background_executor()
                .spawn(async move { merged_rows(&main, &trust) })
        };
        let loading = sheet.clone();
        cx.spawn_in(window, async move |_, cx| {
            let rows = work.await;
            let _ = cx.update(|window, cx| {
                loading.update(cx, |sheet, cx| {
                    sheet.rows = rows;
                    sheet.is_loading = false;
                    cx.notify();
                });
                window.refresh();
            });
        })
        .detach();
        let workbench = cx.entity().downgrade();
        // Archive the chosen tasks (the button, or Return).
        let archive: Run = Rc::new({
            let chosen = sheet.clone();
            move |window, cx| {
                window.close_dialog(cx);
                let (rows, delete, remote) = {
                    let s = chosen.read(cx);
                    (
                        s.rows
                            .iter()
                            .filter(|r| r.is_on && r.can_archive())
                            .cloned()
                            .collect::<Vec<_>>(),
                        s.delete_branches,
                        s.delete_branches && s.delete_remote_branches,
                    )
                };
                if rows.is_empty() {
                    return;
                }
                let main = main.clone();
                let _ = workbench.update(cx, |workbench, cx| {
                    workbench.archive_merged_next(
                        rows,
                        Vec::new(),
                        (delete, remote, main),
                        window,
                        cx,
                    )
                });
            }
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            let count = sheet
                .read(cx)
                .rows
                .iter()
                .filter(|r| r.is_on && r.can_archive())
                .count();
            let (on_click, on_enter) = (archive.clone(), archive.clone());
            dialog
                .w(px(460.))
                .close_button(false)
                .on_ok(move |_, window, cx| {
                    on_enter(window, cx);
                    false
                })
                .child(sheet.clone())
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("merged-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("merged-archive")
                                .primary()
                                .icon(gpui_kit::component::Icon::empty().path("icons/archive.svg"))
                                .label(if count == 1 {
                                    "Archive 1 Task".to_string()
                                } else {
                                    format!("Archive {count} Tasks")
                                })
                                .disabled(count == 0)
                                .on_click(move |_, window, cx| on_click(window, cx)),
                        ),
                )
        });
    }

    /// Close each task's workspace (one with unsaved files is skipped) and
    /// ask about its archive script where its folder isn't trusted, then
    /// archive them all.
    fn archive_merged_next(
        &mut self,
        mut remaining: Vec<MergedRow>,
        mut ready: Vec<(MergedRow, Option<String>)>,
        options: (bool, bool, String),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if remaining.is_empty() {
            return self.archive_merged(ready, options, window, cx);
        }
        let row = remaining.remove(0);
        if let Some(index) = self.workspace_at(&row.path) {
            let id = self.workspaces[index].id;
            if !self.close_workspace_for_task(id, window, cx) {
                return self.archive_merged_next(remaining, ready, options, window, cx);
            }
        }
        let script =
            tasks::already_trusted_project_config(&row.path).and_then(|c| c.archive_script);
        let (path, branch) = (row.path.clone(), row.branch.clone());
        self.confirm_archive_script(
            script,
            &path,
            &branch,
            window,
            cx,
            Box::new(move |workbench, script, window, cx| {
                ready.push((row, script));
                workbench.archive_merged_next(remaining, ready, options, window, cx)
            }),
        );
    }

    fn archive_merged(
        &mut self,
        ready: Vec<(MergedRow, Option<String>)>,
        (delete_branches, delete_remote, main): (bool, bool, String),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trust: Arc<WorkspaceTrust> = self.trust.clone();
        let work = cx.background_executor().spawn(async move {
            let mut archived = Vec::new();
            let mut failures = Vec::new();
            for (row, script) in ready {
                let removed = remove_task(
                    &row.path,
                    &row.branch,
                    row.uncommitted > 0,
                    script.as_deref(),
                    &trust,
                );
                if removed.script_failed {
                    failures.push(format!(
                        "{}: the archive script failed; archived anyway.",
                        row.branch
                    ));
                }
                match removed.result {
                    Err(error) => failures.push(format!("{}: {error}", row.branch)),
                    Ok(mut task) => {
                        if delete_branches {
                            // Merged when the sheet looked isn't enough:
                            // checked again now.
                            let targets: Vec<&str> = row.base.iter().map(String::as_str).collect();
                            let cleanup = landing::delete_landed_branch(
                                &row.branch,
                                &targets,
                                if delete_remote {
                                    row.remote.as_deref()
                                } else {
                                    None
                                },
                                &main,
                                trust.is_trusted(&main),
                            );
                            task.deleted_branch_commit = cleanup.deleted_commit;
                            failures.extend(cleanup.kept);
                        }
                        archived.push(task);
                    }
                }
            }
            (archived, failures)
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let (archived, failures) = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                for failure in failures {
                    toast(window, cx, ToastKind::Warning, failure, None, None);
                }
                if archived.is_empty() {
                    return;
                }
                workbench.refresh_task_states(cx);
                let count = archived.len();
                let deleted = archived
                    .iter()
                    .filter(|t| t.deleted_branch_commit.is_some())
                    .count();
                let branches = match deleted {
                    0 => String::new(),
                    d if d == count => format!(
                        " and deleted {}",
                        if count == 1 {
                            "its branch"
                        } else {
                            "their branches"
                        }
                    ),
                    d => format!(" and deleted {d} of their branches"),
                };
                let what = if count == 1 {
                    archived[0].branch.clone()
                } else {
                    format!("{count} tasks")
                };
                let undo: ToastAction = ("Undo", workbench.restore_action(archived, cx));
                toast(
                    window,
                    cx,
                    ToastKind::Success,
                    format!("Archived {what}{branches}"),
                    None,
                    Some(undo),
                );
            });
        })
        .detach();
    }
}

/// One merged task in the sheet.
#[derive(Clone)]
pub(crate) struct MergedRow {
    path: String,
    branch: String,
    uncommitted: usize,
    /// The remote its branch is published to, if any.
    remote: Option<String>,
    /// What it's merged into (`origin/main`), checked again before its
    /// branch is deleted.
    base: Option<String>,
    /// Ignored files and folders (`None`: they couldn't be listed).
    ignored: Option<Vec<String>>,
    /// Repositories of its own inside it (`None`: couldn't be checked).
    nested: Option<Vec<String>>,
    is_on: bool,
}

impl MergedRow {
    fn can_archive(&self) -> bool {
        self.nested.as_ref().is_some_and(|n| n.is_empty())
    }
}

fn merged_rows(main: &str, trust: &WorkspaceTrust) -> Vec<MergedRow> {
    let remotes = operations::remotes(main);
    let tasks = tasks::registry(main).map(|r| r.tasks).unwrap_or_default();
    tasks::merged_records(main, &tasks, trust.is_trusted(main))
        .into_iter()
        .map(|task| {
            let snapshot = snapshot::snapshot(&task.path, false);
            let trusted = trust.is_trusted(&task.path);
            let nested = ignored::nested_repositories(&task.path, trusted);
            MergedRow {
                uncommitted: snapshot
                    .as_ref()
                    .map(|s| s.changed_file_count())
                    .unwrap_or(0),
                remote: snapshot.and_then(|s| s.upstream).and_then(|upstream| {
                    remotes
                        .iter()
                        .find(|r| upstream.starts_with(&format!("{r}/")))
                        .cloned()
                }),
                base: task.base_ref(),
                ignored: ignored::ignored_entry_list(&task.path, trusted),
                is_on: nested.as_ref().is_some_and(|n| n.is_empty()),
                nested,
                path: task_registry::canonical(&task.path),
                branch: task.branch,
            }
        })
        .collect()
}

/// The Archive Merged Tasks sheet.
pub(crate) struct MergedTasksSheet {
    palette: ChromePalette,
    rows: Vec<MergedRow>,
    is_loading: bool,
    delete_branches: bool,
    delete_remote_branches: bool,
}

impl MergedTasksSheet {
    fn remote_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.is_on)
            .filter_map(|r| r.remote.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }

    /// Ignored files in the chosen tasks; `None` when one's couldn't be listed.
    fn ignored(&self) -> Option<Vec<String>> {
        let chosen: Vec<&MergedRow> = self.rows.iter().filter(|r| r.is_on).collect();
        if chosen.iter().any(|r| r.ignored.is_none()) {
            return None;
        }
        Some(
            chosen
                .iter()
                .flat_map(|r| r.ignored.clone().unwrap_or_default())
                .collect(),
        )
    }
}

impl Render for MergedTasksSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let small =
            |text: String, color| div().text_size(px(11.)).text_color(hsla(color)).child(text);
        let mut rows = div().flex().flex_col().gap(px(6.));
        for (index, row) in self.rows.iter().enumerate() {
            let name = name_of(&row.path);
            let mut label = div().flex().items_center().gap(px(6.)).child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(name.clone()),
            );
            if row.branch != name {
                label = label.child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(11.))
                        .text_color(hsla(p.text_tertiary))
                        .child(row.branch.clone()),
                );
            }
            label = match &row.nested {
                Some(nested) if !nested.is_empty() => label.child(small(
                    format!(
                        "holds a repository of its own ({}); move it out first",
                        nested[0]
                    ),
                    p.warning,
                )),
                None => label.child(small(
                    "couldn't check it for repositories of its own".into(),
                    p.warning,
                )),
                Some(_) => label
                    .when(row.uncommitted > 0, |d| {
                        d.child(small(format!("{} uncommitted", row.uncommitted), p.warning))
                    })
                    .children(match &row.ignored {
                        Some(ignored) if !ignored.is_empty() => {
                            Some(small(format!("{} ignored", ignored.len()), p.warning))
                        }
                        None => Some(small("ignored files unknown".into(), p.warning)),
                        _ => None,
                    }),
            };
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Checkbox::new(("merged-row", index))
                            .checked(row.is_on)
                            .disabled(!row.can_archive())
                            .on_change(cx.listener(move |sheet, checked: &bool, window, cx| {
                                if let Some(row) = sheet.rows.get_mut(index) {
                                    row.is_on = *checked;
                                }
                                window.refresh();
                                cx.notify();
                            })),
                    )
                    .child(label),
            );
        }
        let remotes = self.remote_names();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_color(hsla(p.text))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Archive Merged Tasks"))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(
                        "These tasks' branches are merged into their base. Archiving removes their folders; \
                         uncommitted files are saved, and Undo brings it all back.",
                    ),
            )
            .when(self.is_loading, |d| d.child(small("Looking for merged tasks…".into(), p.text_tertiary)))
            .when(!self.is_loading && self.rows.is_empty(), |d| {
                d.child(small("No task's branch is merged.".into(), p.text_tertiary))
            })
            .child(rows)
            .children(match self.ignored() {
                Some(ignored) if !ignored.is_empty() => Some(small(
                    format!(
                        "Ignored files in them ({}{}) are deleted, and Undo can't bring them back.",
                        ignored.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
                        if ignored.len() > 3 { ", …" } else { "" }
                    ),
                    p.warning,
                )),
                None => Some(small(
                    "Ignored files in them are deleted, and Undo can't bring them back.".into(),
                    p.warning,
                )),
                _ => None,
            })
            .when(!self.rows.is_empty(), |d| {
                d.child(
                    Checkbox::new("merged-delete-branches")
                        .label("Delete their branches too")
                        .checked(self.delete_branches)
                        .on_change(cx.listener(|sheet, checked: &bool, _, cx| {
                            sheet.delete_branches = *checked;
                            cx.notify();
                        })),
                )
                .when(!remotes.is_empty(), |d| {
                    d.child(
                        div().pl(px(18.)).child(
                            Checkbox::new("merged-delete-remote")
                                .label(format!(
                                    "…and on {} (Undo can't bring those back)",
                                    remotes.join(", ")
                                ))
                                .checked(self.delete_remote_branches)
                                .disabled(!self.delete_branches)
                                .on_change(cx.listener(|sheet, checked: &bool, _, cx| {
                                    sheet.delete_remote_branches = *checked;
                                    cx.notify();
                                })),
                        ),
                    )
                })
            })
    }
}
