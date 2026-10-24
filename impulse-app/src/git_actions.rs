//! Git actions with the experience around them (the Swift `GitActions`,
//! `GitPrompts` and `MainWindowController+GitHost`): safety snapshots and
//! Undo for anything that throws work away, a confirmation before
//! discarding files, force pushing or aborting, plain-English errors with
//! git's own output under them, and what happened afterwards in a
//! notification. Also the git commands `run_command` sends here
//! (`git_fetch`, `git_push`, `git_undo_commit`, …).
//!
//! Every action runs on the repository's queue (`RepositoryState::run`), so
//! the main thread never waits for git.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    div, prelude::*, px, App, ClipboardItem, Context, ElementId, Entity, SharedString, Window,
};
use impulse_git::cli::ErrorKind;
use impulse_git::operations::{
    self, CommitOptions, GitResult, OperationAction, OperationError, PullMode, PushOptions,
    ResetMode, StashOptions,
};
use impulse_git::safety::{self, SafetySnapshot};
use impulse_git::{operation_abort, ChangeStatus, FileChange, RepoOperation};

use crate::repository_state::RepositoryState;
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;

/// The commands `run_command` hands to `run_git_command`.
pub(crate) const GIT_COMMANDS: &[&str] = &[
    "switch_branch",
    "git_fetch",
    "git_fetch_all",
    "git_pull",
    "git_pull_rebase",
    "git_push",
    "git_force_push",
    "git_stash",
    "git_pop_stash",
    "git_undo_commit",
    "git_copy_remote_url",
    "git_open_remote",
    "git_create_tag",
    "git_push_tags",
];

type Then = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;
/// Creates the tag a prompt asks for.
type Create = Rc<dyn Fn(&mut Window, &mut App)>;
/// What happens after a commit: `true` when it was made.
type Done = Rc<dyn Fn(&mut Workbench, bool, &mut Window, &mut Context<Workbench>)>;

/// Notifications with a button (Undo, …), told apart for removal.
struct ActionToast;

/// How long a notification with a button stays up.
const ACTION_TOAST_SECONDS: u64 = 15;

/// Both paths of a rename.
fn paths_of(changes: &[FileChange]) -> Vec<String> {
    changes
        .iter()
        .flat_map(|c| std::iter::once(c.path.clone()).chain(c.old_path.clone()))
        .collect()
}

fn short(sha: &str) -> String {
    sha[..sha.len().min(7)].to_string()
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

impl Workbench {
    // MARK: Presenting

    /// A git failure, as a dialog: the plain-English message, with git's
    /// own output under it.
    pub(crate) fn present_git_error(
        &mut self,
        error: &OperationError,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = error.output().unwrap_or("").trim().to_string();
        let generic = matches!(error, OperationError::Cli(cli) if cli.kind == ErrorKind::Other);
        let message = error.message();
        let detail = (!detail.is_empty() && !generic).then_some(detail);
        let title = title.to_string();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(460.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(message.clone())
                        .children(detail.clone().map(|detail| {
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(11.5))
                                .max_h(px(220.))
                                .overflow_hidden()
                                .child(detail)
                        })),
                )
                .footer(
                    div().flex().justify_end().child(
                        Button::new("git-error-ok")
                            .primary()
                            .label("OK")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
        });
    }

    pub(crate) fn report(
        &mut self,
        result: GitResult<()>,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = result {
            self.present_git_error(&error, title, window, cx);
        }
    }

    /// Ask before something destructive; `then` runs on the confirm button.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn git_confirm(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        confirm: &'static str,
        destructive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        let (title, message) = (title.into(), message.into());
        let then: Then = Rc::new(then);
        let workbench = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (then, workbench) = (then.clone(), workbench.clone());
            // Return confirms, as the Swift app's alerts' default button.
            let (on_ok, ok_workbench) = (then.clone(), workbench.clone());
            let dialog = dialog.on_ok(move |_, window, cx| {
                window.close_dialog(cx);
                let then = on_ok.clone();
                let _ = ok_workbench.update(cx, |workbench, cx| then(workbench, window, cx));
                false
            });
            let button = Button::new("git-confirm").label(confirm);
            let button = if destructive {
                button.danger()
            } else {
                button.primary()
            };
            dialog
                .title(title.clone())
                .w(px(440.))
                .child(div().child(message.clone()))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("git-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(button.on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let then = then.clone();
                            let _ =
                                workbench.update(cx, |workbench, cx| then(workbench, window, cx));
                        })),
                )
        });
    }

    /// A notification (the Swift app's toast).
    pub(crate) fn toast(
        &mut self,
        note: Notification,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(note, cx);
    }

    /// A success notification with a button (Undo, Pop Stash, …); `detail`
    /// goes under the message.
    pub(crate) fn toast_with_action(
        &mut self,
        message: String,
        detail: Option<String>,
        action_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        let action: Then = Rc::new(action);
        let workbench = cx.entity().downgrade();
        #[cfg(debug_assertions)]
        {
            let (action, workbench) = (action.clone(), workbench.clone());
            let press: crate::debug_snapshot::Press = Rc::new(move |window, cx| {
                let action = action.clone();
                let _ = workbench.update(cx, |workbench, cx| action(workbench, window, cx));
            });
            crate::debug_snapshot::note_toast(
                "success",
                &message,
                detail.as_deref(),
                Some((action_title, press)),
            );
        }
        let note = match detail {
            Some(detail) => Notification::success(detail).title(message),
            None => Notification::success(message),
        };
        let note = note.action(move |_, _, cx| {
            let (action, workbench) = (action.clone(), workbench.clone());
            let note = cx.entity();
            Button::new("toast-action")
                .label(action_title)
                .on_click(move |_, window, cx| {
                    note.update(cx, |note, cx| note.dismiss(window, cx));
                    let action = action.clone();
                    let _ = workbench.update(cx, |workbench, cx| action(workbench, window, cx));
                })
        });
        // Up long enough to reach the button (GPUI Kit's own autohide is
        // 5 seconds), as the Swift app's toasts with an action are.
        let key = ElementId::from(SharedString::from(format!("git-toast-{}", self.next_id())));
        let note = note.id1::<ActionToast>(key.clone());
        window.push_notification(note, cx);
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(ACTION_TOAST_SECONDS))
                .await;
            let _ = cx.update(|window, cx| window.remove_notification1::<ActionToast>(key, cx));
        })
        .detach();
    }

    // MARK: Running

    /// Run `operation` on `repo`'s queue; `done` gets its result on the
    /// main thread once the repository is refreshed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn git_run<T: Send + 'static>(
        &mut self,
        repo: &Entity<RepositoryState>,
        label: Option<String>,
        snapshot_reason: Option<String>,
        require_snapshot: bool,
        operation: impl FnOnce(&str, &mut dyn FnMut(&str)) -> GitResult<T> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(
                &mut Workbench,
                GitResult<T>,
                Option<SafetySnapshot>,
                &mut Window,
                &mut Context<Workbench>,
            ) + 'static,
    ) {
        let task = repo.update(cx, |repo, cx| {
            repo.run(label, snapshot_reason, require_snapshot, operation, cx)
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let (result, snapshot) = task.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                done(workbench, result, snapshot, window, cx)
            });
        })
        .detach();
    }

    /// The active workspace's repository, or a note that there isn't one.
    pub(crate) fn repository_or_note(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<RepositoryState>> {
        let repo = self.git.active.clone();
        if repo.is_none() {
            self.toast(Notification::info("Not in a git repository."), window, cx);
        }
        repo
    }

    /// The git commands of `GIT_COMMANDS`.
    pub(crate) fn run_git_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repository_or_note(window, cx) else {
            return;
        };
        match id {
            "switch_branch" => self.show_branch_switcher(repo, window, cx),
            "git_fetch" => self.git_fetch(&repo, false, window, cx),
            "git_fetch_all" => self.git_fetch(&repo, true, window, cx),
            "git_pull" => self.git_pull(&repo, None, window, cx),
            "git_pull_rebase" => self.git_pull(&repo, Some(PullMode::Rebase), window, cx),
            "git_push" => self.git_push(&repo, false, window, cx),
            "git_force_push" => self.git_force_push(&repo, window, cx),
            "git_stash" => self.git_stash_all(&repo, window, cx),
            "git_pop_stash" => self.git_pop_latest_stash(&repo, window, cx),
            "git_undo_commit" => self.git_undo_last_commit(&repo, window, cx),
            "git_copy_remote_url" => self.git_with_remote_url(&repo, false, window, cx),
            "git_open_remote" => self.git_with_remote_url(&repo, true, window, cx),
            "git_create_tag" => self.git_ask_for_tag(&repo, window, cx),
            "git_push_tags" => self.git_push_all_tags(&repo, window, cx),
            _ => {}
        }
    }

    // MARK: Staging

    pub(crate) fn git_stage(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: &[FileChange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = paths_of(changes);
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| operations::stage(&paths, root),
            window,
            cx,
            |this, result, _, window, cx| this.report(result, "Couldn't stage", window, cx),
        );
    }

    pub(crate) fn git_unstage(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: &[FileChange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = paths_of(changes);
        let unborn = repo.read(cx).snapshot.as_ref().is_some_and(|s| s.is_unborn);
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| operations::unstage(&paths, root, unborn),
            window,
            cx,
            |this, result, _, window, cx| this.report(result, "Couldn't unstage", window, cx),
        );
    }

    pub(crate) fn git_stage_all(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_run(
            repo,
            None,
            None,
            false,
            |root, _| operations::stage_all(root),
            window,
            cx,
            |this, result, _, window, cx| {
                this.report(result, "Couldn't stage all changes", window, cx)
            },
        );
    }

    pub(crate) fn git_unstage_all(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let unborn = repo.read(cx).snapshot.as_ref().is_some_and(|s| s.is_unborn);
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| operations::unstage_all(root, unborn),
            window,
            cx,
            |this, result, _, window, cx| {
                this.report(result, "Couldn't unstage all changes", window, cx)
            },
        );
    }

    // MARK: Discarding files

    /// Discard working-tree changes (tracked) or move untracked files to the
    /// Trash, after confirming. Undoable.
    pub(crate) fn git_discard(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: Vec<FileChange>,
        include_staged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if changes.is_empty() {
            return;
        }
        let names = if changes.len() == 1 {
            format!("\u{201c}{}\u{201d}", file_name(&changes[0].path))
        } else {
            format!("{} files", changes.len())
        };
        let message = if include_staged {
            "Staged and unstaged changes will be reverted to the last commit. You can undo this right after."
        } else {
            "Your changes will be reverted. Untracked files move to the Trash. You can undo this right after."
        };
        let repo = repo.clone();
        self.git_confirm(
            format!("Discard changes to {names}?"),
            message,
            "Discard",
            true,
            window,
            cx,
            move |this, window, cx| {
                this.perform_discard(&repo, changes.clone(), include_staged, window, cx)
            },
        );
    }

    fn perform_discard(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: Vec<FileChange>,
        include_staged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let untracked: Vec<String> = changes
            .iter()
            .filter(|c| c.status == ChangeStatus::Untracked)
            .map(|c| c.path.clone())
            .collect();
        let tracked: Vec<FileChange> = changes
            .iter()
            .filter(|c| c.status != ChangeStatus::Untracked)
            .cloned()
            .collect();
        let tracked_paths = paths_of(&tracked);
        let label = if changes.len() == 1 {
            file_name(&changes[0].path)
        } else {
            format!("{} files", changes.len())
        };
        let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        let repo_for_undo = repo.clone();
        self.git_run(
            repo,
            None,
            Some(format!("Discard {label}")),
            true,
            move |root, _| {
                if !tracked_paths.is_empty() {
                    if include_staged {
                        operations::discard_all(&tracked_paths, root)?;
                    } else {
                        operations::discard_working_tree(&tracked_paths, root)?;
                    }
                }
                for path in &untracked {
                    let file = std::path::Path::new(root).join(path);
                    trash::delete(&file).map_err(|error| {
                        OperationError::Invalid(format!(
                            "Couldn't move {path} to the Trash: {error}"
                        ))
                    })?;
                }
                Ok(())
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| match result {
                Ok(()) => {
                    if let Some(snapshot) = snapshot {
                        this.offer_restore(
                            &repo_for_undo,
                            format!("Discarded {label}"),
                            snapshot,
                            paths,
                            window,
                            cx,
                        );
                    }
                }
                Err(error) => {
                    this.present_git_error(&error, &format!("Couldn't discard {label}"), window, cx)
                }
            },
        );
    }

    /// A notification whose Undo puts `paths` back from `snapshot`.
    fn offer_restore(
        &mut self,
        repo: &Entity<RepositoryState>,
        message: String,
        snapshot: SafetySnapshot,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = repo.clone();
        self.toast_with_action(
            message,
            None,
            "Undo",
            window,
            cx,
            move |this, window, cx| {
                let (snapshot, paths) = (snapshot.clone(), paths.clone());
                this.git_run(
                    &repo,
                    None,
                    None,
                    false,
                    move |root, _| safety::restore(&snapshot, &paths, root),
                    window,
                    cx,
                    |this, result, _, window, cx| this.report(result, "Couldn't undo", window, cx),
                );
            },
        );
    }

    // MARK: Conflicts

    pub(crate) fn git_mark_resolved(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: &[FileChange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| operations::mark_resolved(&paths, root),
            window,
            cx,
            |this, result, _, window, cx| {
                this.report(result, "Couldn't mark as resolved", window, cx)
            },
        );
    }

    /// Take one side of every conflict in these files (snapshot first).
    /// Undo puts the files back in conflict, with any edits made before.
    pub(crate) fn git_resolve(
        &mut self,
        repo: &Entity<RepositoryState>,
        changes: &[FileChange],
        take_ours: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        let names = if paths.len() == 1 {
            file_name(&paths[0])
        } else {
            format!("{} files", paths.len())
        };
        let repo_for_undo = repo.clone();
        let undo_paths = paths.clone();
        self.git_run(
            repo,
            Some(
                if take_ours {
                    "Keeping current…"
                } else {
                    "Taking incoming…"
                }
                .into(),
            ),
            Some(
                if take_ours {
                    "keep current side"
                } else {
                    "take incoming side"
                }
                .into(),
            ),
            true,
            move |root, _| {
                let point = operations::conflict_point(root);
                operations::resolve_conflicts(&paths, take_ours, root).map(|_| point)
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| {
                let point = match result {
                    Ok(point) => point,
                    Err(error) => {
                        return this.present_git_error(
                            &error,
                            "Couldn't resolve the conflict",
                            window,
                            cx,
                        )
                    }
                };
                let Some(snapshot) = snapshot else {
                    return;
                };
                let message = if take_ours {
                    format!("Kept current in {names}")
                } else {
                    format!("Took incoming in {names}")
                };
                this.toast_with_action(
                    message,
                    None,
                    "Undo",
                    window,
                    cx,
                    move |this, window, cx| {
                        let (paths, point, snapshot) =
                            (undo_paths.clone(), point.clone(), snapshot.clone());
                        this.git_run(
                            &repo_for_undo,
                            None,
                            Some("before undo".into()),
                            false,
                            move |root, _| {
                                operations::reopen_conflicts(&paths, &point, root)?;
                                // The conflict as it was, edits included.
                                safety::restore(&snapshot, &paths, root)
                            },
                            window,
                            cx,
                            |this, result, _, window, cx| {
                                this.report(result, "Couldn't undo", window, cx)
                            },
                        );
                    },
                );
            },
        );
    }

    /// Continue, skip or abort the operation in progress. Aborting a merge,
    /// cherry-pick or revert always works (`operation_abort`).
    pub(crate) fn git_perform(
        &mut self,
        repo: &Entity<RepositoryState>,
        action: OperationAction,
        operation: RepoOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = operation.title().to_lowercase();
        if action == OperationAction::Abort && operation_abort::applies(&operation) {
            let repo = repo.clone();
            return self.git_confirm(
                format!("Abort {name}?"),
                "Your files go back to how they were before it started. Everything as it is now \
                 is saved in a snapshot that Undo restores.",
                "Abort",
                true,
                window,
                cx,
                move |this, window, cx| this.git_abort(&repo, operation.clone(), window, cx),
            );
        }
        let run_name = name.clone();
        let run = move |this: &mut Workbench,
                        repo: &Entity<RepositoryState>,
                        window: &mut Window,
                        cx: &mut Context<Workbench>| {
            let (operation, name) = (operation.clone(), run_name.clone());
            let label = format!("{}…", operation.title());
            this.git_run(
                repo,
                Some(label),
                None,
                false,
                move |root, _| operations::perform(action, &operation, root),
                window,
                cx,
                move |this, result, _, window, cx| {
                    let title = format!("Couldn't {} the {name}", action.raw_value());
                    this.report(result, &title, window, cx)
                },
            );
        };
        if action == OperationAction::Abort {
            let repo = repo.clone();
            self.git_confirm(
                format!("Abort {name}?"),
                "Changes made during the operation will be lost.",
                "Abort",
                true,
                window,
                cx,
                move |this, window, cx| run(this, &repo, window, cx),
            );
        } else {
            run(self, repo, window, cx);
        }
    }

    fn git_abort(
        &mut self,
        repo: &Entity<RepositoryState>,
        operation: RepoOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = operation.title().to_lowercase();
        let repo_for_undo = repo.clone();
        let for_abort = operation.clone();
        self.git_run(
            repo,
            Some("Aborting…".into()),
            None,
            false,
            move |root, _| operation_abort::abort(&for_abort, root),
            window,
            cx,
            move |this, result, _, window, cx| {
                let aborted = match result {
                    Ok(aborted) => aborted,
                    Err(error) => {
                        let title = format!("Couldn't abort the {name}");
                        return this.present_git_error(&error, &title, window, cx);
                    }
                };
                let message = match &aborted.problem {
                    Some(_) => format!("The {name} was only partly aborted"),
                    None => format!("Aborted {name}"),
                };
                let detail = match (&aborted.problem, &aborted.outcome) {
                    (Some(problem), _) => Some(format!(
                        "{} Undo puts every file back as it was before aborting.",
                        problem.message()
                    )),
                    (None, operation_abort::Outcome::Reset { kept, exact }) => {
                        let work = if kept.is_empty() {
                            String::new()
                        } else if kept.len() == 1 {
                            " and put back your file from before it".into()
                        } else {
                            format!(" and put back your {} files from before it", kept.len())
                        };
                        let mut detail =
                            format!("git couldn't abort, so Impulse reset to HEAD{work}.");
                        if !exact && !kept.is_empty() {
                            detail
                                .push_str(" Edits made during it outside its files were kept too.");
                        }
                        Some(detail)
                    }
                    _ => None,
                };
                if aborted.undo.is_none() {
                    let note = match detail {
                        Some(detail) => Notification::success(detail).title(message),
                        None => Notification::success(message),
                    };
                    return this.toast(note, window, cx);
                }
                let repo = repo_for_undo.clone();
                this.toast_with_action(
                    message,
                    detail,
                    "Undo",
                    window,
                    cx,
                    move |this, window, cx| {
                        let (aborted, operation) = (aborted.clone(), operation.clone());
                        this.git_run(
                            &repo,
                            Some("Undoing…".into()),
                            None,
                            false,
                            move |root, _| operation_abort::undo(&aborted, &operation, root),
                            window,
                            cx,
                            |this, result, _, window, cx| {
                                this.report(result, "Couldn't undo the abort", window, cx)
                            },
                        );
                    },
                );
            },
        );
    }

    // MARK: Commit

    /// Commit what's staged. If nothing is staged but tracked files changed,
    /// offer to commit those (never adds untracked files silently).
    /// `done(true)` once committed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn git_commit(
        &mut self,
        repo: &Entity<RepositoryState>,
        message: String,
        options: CommitOptions,
        then_push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Workbench, bool, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        let Some(snapshot) = repo.read(cx).snapshot.clone() else {
            return;
        };
        let done: Done = Rc::new(done);
        if snapshot.staged.is_empty() && !options.amend {
            if snapshot.unstaged.is_empty() {
                self.toast(
                    Notification::warning("Stage the files you want to commit first."),
                    window,
                    cx,
                );
                return done(self, false, window, cx);
            }
            let repo = repo.clone();
            return self.git_confirm(
                "Nothing is staged",
                format!(
                    "Commit all {} changed tracked files? Untracked files aren't included.",
                    snapshot.unstaged.len()
                ),
                "Commit All",
                false,
                window,
                cx,
                move |this, window, cx| {
                    let options = CommitOptions {
                        include_unstaged_tracked: true,
                        ..options
                    };
                    let done = done.clone();
                    this.perform_commit(
                        &repo,
                        message.clone(),
                        options,
                        then_push,
                        window,
                        cx,
                        move |this, ok, window, cx| done(this, ok, window, cx),
                    );
                },
            );
        }
        self.perform_commit(
            repo,
            message,
            options,
            then_push,
            window,
            cx,
            move |this, ok, window, cx| done(this, ok, window, cx),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn perform_commit(
        &mut self,
        repo: &Entity<RepositoryState>,
        message: String,
        options: CommitOptions,
        then_push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Workbench, bool, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        self.git.committing = true;
        cx.notify();
        let repo_after = repo.clone();
        let recorded = message.clone();
        self.git_run(
            repo,
            Some("Committing…".into()),
            None,
            false,
            move |root, _| operations::commit(&message, &options, root),
            window,
            cx,
            move |this, result, _, window, cx| {
                this.git.committing = false;
                match result {
                    Ok(sha) => {
                        let root = repo_after.read(cx).root.clone();
                        this.git.record_message(&root, &recorded);
                        if options.amend {
                            this.toast(
                                Notification::success(format!("Amended {}", short(&sha))),
                                window,
                                cx,
                            );
                        } else {
                            let repo = repo_after.clone();
                            this.toast_with_action(
                                format!("Committed {}", short(&sha)),
                                None,
                                "Uncommit",
                                window,
                                cx,
                                move |this, window, cx| {
                                    // Only this commit: not one made on top of it since.
                                    let sha = sha.clone();
                                    this.git_run(
                                        &repo,
                                        None,
                                        None,
                                        false,
                                        move |root, _| operations::uncommit(Some(&sha), root),
                                        window,
                                        cx,
                                        |this, result, _, window, cx| {
                                            this.report(result, "Couldn't uncommit", window, cx)
                                        },
                                    );
                                },
                            );
                        }
                        done(this, true, window, cx);
                        if then_push {
                            this.git_push(&repo_after, false, window, cx);
                        }
                    }
                    Err(error) => {
                        this.present_git_error(&error, "Couldn't commit", window, cx);
                        done(this, false, window, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Undo the last commit, keeping its changes staged. Asks first when
    /// the commit is already on the upstream (pushing again would need a
    /// force).
    pub(crate) fn git_undo_last_commit(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = repo.read(cx).snapshot.clone() else {
            return;
        };
        let (Some(head), false) = (snapshot.head_oid.clone(), snapshot.is_unborn) else {
            return;
        };
        let perform = {
            let repo = repo.clone();
            move |this: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>| {
                let (head, redo_head, repo_redo) = (head.clone(), head.clone(), repo.clone());
                this.git_run(
                    &repo,
                    None,
                    None,
                    false,
                    // The commit the user saw, not whatever HEAD is by then.
                    move |root, _| operations::uncommit(Some(&head), root),
                    window,
                    cx,
                    move |this, result, _, window, cx| {
                        if let Err(error) = result {
                            return this.present_git_error(
                                &error,
                                "Couldn't undo the commit",
                                window,
                                cx,
                            );
                        }
                        this.toast_with_action(
                            format!("Undid {}; its changes are staged", short(&redo_head)),
                            None,
                            "Redo",
                            window,
                            cx,
                            move |this, window, cx| {
                                let head = redo_head.clone();
                                this.git_run(
                                    &repo_redo,
                                    None,
                                    None,
                                    false,
                                    move |root, _| operations::reset(ResetMode::Soft, &head, root),
                                    window,
                                    cx,
                                    |this, result, _, window, cx| {
                                        this.report(result, "Couldn't redo the commit", window, cx)
                                    },
                                );
                            },
                        );
                    },
                );
            }
        };
        match (&snapshot.upstream, snapshot.ahead) {
            (Some(upstream), 0) => self.git_confirm(
                "Undo a pushed commit?",
                format!(
                    "{} is already on {upstream}. Undoing it here means the next push has to force.",
                    short(snapshot.head_oid.as_deref().unwrap_or(""))
                ),
                "Undo Commit",
                true,
                window,
                cx,
                perform,
            ),
            _ => perform(self, window, cx),
        }
    }

    // MARK: Remote

    /// Fetch the branch's remote (or every remote) and say what came in.
    pub(crate) fn git_fetch(
        &mut self,
        repo: &Entity<RepositoryState>,
        all_remotes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = repo
            .read(cx)
            .snapshot
            .as_ref()
            .map(|s| s.behind)
            .unwrap_or(0);
        let label = if all_remotes {
            "Fetching all remotes…"
        } else {
            "Fetching…"
        };
        let repo_after = repo.clone();
        self.git_run(
            repo,
            Some(label.into()),
            None,
            false,
            move |root, progress| {
                operations::fetch(
                    all_remotes,
                    None,
                    root,
                    std::time::Duration::from_secs(600),
                    Some(progress),
                )
            },
            window,
            cx,
            move |this, result, _, window, cx| {
                if let Err(error) = result {
                    return this.present_git_error(&error, "Couldn't fetch", window, cx);
                }
                let behind = repo_after
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .map(|s| s.behind)
                    .unwrap_or(0);
                let message = if behind > before {
                    format!("Fetched: {} to pull", plural(behind, "commit"))
                } else {
                    "Fetched: nothing new for this branch".into()
                };
                this.toast(Notification::success(message), window, cx);
            },
        );
    }

    /// Pull with the strategy from settings (or the one given).
    pub(crate) fn git_pull(
        &mut self,
        repo: &Entity<RepositoryState>,
        mode: Option<PullMode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = mode.unwrap_or_else(|| match SettingsStore::get(cx).git_pull_mode.as_str() {
            "rebase" => PullMode::Rebase,
            "merge" => PullMode::Merge,
            _ => PullMode::FastForwardOnly,
        });
        let reason =
            (mode != PullMode::FastForwardOnly).then(|| format!("pull --{}", mode.raw_value()));
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            Some("Pulling…".into()),
            reason,
            false,
            move |root, progress| {
                // Where HEAD is when the pull runs (not when it was asked for).
                let before = operations::resolve_ref("HEAD", root);
                operations::pull(mode, root, Some(progress))?;
                let after = operations::resolve_ref("HEAD", root);
                let count = match (&before, &after) {
                    (Some(b), Some(a)) => operations::commit_count(b, a, root),
                    _ => None,
                };
                Ok((before, after, count))
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| {
                let (before, after, count) = match result {
                    Ok(heads) => heads,
                    Err(error) => {
                        return this.present_git_error(&error, "Couldn't pull", window, cx)
                    }
                };
                if before == after {
                    return this.toast(Notification::success("Already up to date"), window, cx);
                }
                let message = count
                    .map(|n| format!("Pulled {}", plural(n, "commit")))
                    .unwrap_or_else(|| "Pulled".into());
                match (before, mode) {
                    (Some(before), PullMode::Rebase | PullMode::Merge) => {
                        // Rebasing or merging rewrote or added commits: offer Undo.
                        this.offer_head_undo(
                            &repo_undo,
                            message,
                            before,
                            snapshot,
                            "Couldn't undo the pull",
                            window,
                            cx,
                        )
                    }
                    _ => this.toast(Notification::success(message), window, cx),
                }
            },
        );
    }

    /// A notification whose Undo moves the branch back to `previous_head`
    /// and restores the working tree from the safety snapshot (without
    /// one, `reset --keep`, which keeps uncommitted work).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn offer_head_undo(
        &mut self,
        repo: &Entity<RepositoryState>,
        message: String,
        previous_head: String,
        snapshot: Option<SafetySnapshot>,
        failure: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = repo.clone();
        self.toast_with_action(
            message,
            None,
            "Undo",
            window,
            cx,
            move |this, window, cx| {
                let (previous_head, snapshot) = (previous_head.clone(), snapshot.clone());
                let require = snapshot.is_some();
                this.git_run(
                    &repo,
                    None,
                    Some("before undo".into()),
                    require,
                    move |root, _| match &snapshot {
                        None => operations::reset(ResetMode::Keep, &previous_head, root),
                        Some(snapshot) => {
                            operations::reset(ResetMode::Hard, &previous_head, root)?;
                            safety::restore::<&str>(snapshot, &[], root)
                        }
                    },
                    window,
                    cx,
                    move |this, result, _, window, cx| this.report(result, failure, window, cx),
                );
            },
        );
    }

    /// Push; publishes the branch when it has no upstream (to its remote,
    /// else origin, else the first).
    pub(crate) fn git_push(
        &mut self,
        repo: &Entity<RepositoryState>,
        force_with_lease: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = repo.read(cx).snapshot.clone();
        let needs_upstream = snapshot.as_ref().is_none_or(|s| s.upstream.is_none());
        let branch = snapshot.and_then(|s| s.branch);
        let follow_tags = SettingsStore::get(cx).git_push_follow_tags;
        let label = if needs_upstream {
            "Publishing…"
        } else if force_with_lease {
            "Force pushing…"
        } else {
            "Pushing…"
        };
        let pushed = branch.clone();
        self.git_run(
            repo,
            Some(label.into()),
            None,
            false,
            move |root, progress| {
                let remote = operations::default_remote(root, branch.as_deref())
                    .unwrap_or_else(|| "origin".into());
                let mut output = Vec::new();
                let mut collect = |line: &str| {
                    output.push(line.to_string());
                    progress(line);
                };
                operations::push(
                    &PushOptions {
                        set_upstream: needs_upstream,
                        remote: remote.clone(),
                        branch,
                        force_with_lease,
                        follow_tags,
                    },
                    root,
                    Some(&mut collect),
                )?;
                Ok((remote, output))
            },
            window,
            cx,
            move |this, result, _, window, cx| {
                let (remote, output) = match result {
                    Ok(pushed) => pushed,
                    Err(error) => {
                        let title = if needs_upstream {
                            "Couldn't publish"
                        } else {
                            "Couldn't push"
                        };
                        return this.present_git_error(&error, title, window, cx);
                    }
                };
                let message = if needs_upstream {
                    format!(
                        "Published {} to {remote}",
                        pushed.as_deref().unwrap_or("branch")
                    )
                } else if force_with_lease {
                    "Force pushed".into()
                } else {
                    "Pushed".into()
                };
                // What the server said back, such as a link for a merge request.
                let reply = impulse_kit::git::server_message::GitServerMessage::parse(&output);
                let detail = reply.as_ref().map(|reply| {
                    format!(
                        "{remote}: {}",
                        reply
                            .link_caption
                            .clone()
                            .unwrap_or_else(|| reply.lines.join(" "))
                    )
                });
                match reply.and_then(|r| r.link) {
                    Some(link) => this.toast_with_action(
                        message,
                        detail,
                        "Open Link",
                        window,
                        cx,
                        move |_, _, cx| cx.open_url(&link),
                    ),
                    None => {
                        let note = match detail {
                            Some(detail) => Notification::success(detail).title(message),
                            None => Notification::success(message),
                        };
                        this.toast(note, window, cx)
                    }
                }
            },
        );
    }

    /// Overwrite the upstream with this branch, after asking. With a
    /// lease: git refuses if the remote moved since the last fetch.
    pub(crate) fn git_force_push(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = repo.read(cx).snapshot.clone();
        let Some((branch, upstream)) = snapshot.and_then(|s| {
            s.branch
                .map(|b| (b, s.upstream.unwrap_or_else(|| "the remote".into())))
        }) else {
            return self.toast(
                Notification::info("Check out a branch to push."),
                window,
                cx,
            );
        };
        let repo = repo.clone();
        self.git_confirm(
            format!("Force push {branch}?"),
            format!(
                "{upstream} is replaced with your {branch}, dropping any commits only it has. It \
                 stops if someone else pushed since your last fetch (--force-with-lease)."
            ),
            "Force Push",
            true,
            window,
            cx,
            move |this, window, cx| this.git_push(&repo, true, window, cx),
        );
    }

    /// Copy the default remote's URL, or open it as a web page
    /// (`git@host:o/r.git` is `https://host/o/r`; Impulse knows nothing
    /// about the host itself).
    fn git_with_remote_url(
        &mut self,
        repo: &Entity<RepositoryState>,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = repo.read(cx).root.clone();
        let lookup = cx.background_spawn(async move {
            operations::default_remote(&root, None)
                .and_then(|remote| operations::remote_url(&remote, &root))
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let url = lookup.await;
            let _ = workbench.update_in(cx, |this, window, cx| {
                let Some(url) = url else {
                    return this.toast(
                        Notification::info("This repository has no remote."),
                        window,
                        cx,
                    );
                };
                if !open {
                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    return this.toast(Notification::success(format!("Copied {url}")), window, cx);
                }
                match impulse_kit::git::refs::RemoteWebUrl::parse(&url) {
                    Some(web) => cx.open_url(&web.base),
                    None => this.toast(
                        Notification::info(format!("{url} isn't a web address.")),
                        window,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    // MARK: Tags

    /// Ask for a tag on the current commit: a name (the next version is
    /// suggested), an optional message (an annotated tag), and whether to
    /// push it, which starts from the "Push new tags" setting.
    fn git_ask_for_tag(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_ask_for_tag_at(repo, "HEAD".into(), window, cx)
    }

    /// The tag prompt for `at` (a commit, or "HEAD"): History's Create Tag
    /// Here.
    pub(crate) fn git_ask_for_tag_at(
        &mut self,
        repo: &Entity<RepositoryState>,
        at: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = at.clone();
        let (root, trusted) = {
            let repo = repo.read(cx);
            (repo.root.clone(), repo.trusted)
        };
        let lookup = cx.background_spawn(async move {
            let remote = operations::default_remote(&root, None);
            let tags: Vec<String> =
                RepositoryState::read_git(&root, trusted, &["tag", "--sort=-creatordate"])
                    .map(|out| out.lines().map(str::to_string).collect())
                    .unwrap_or_default();
            let suggestion = impulse_kit::git::refs::next_tag_name(&tags).unwrap_or_default();
            let subject = RepositoryState::read_git(
                &root,
                trusted,
                &[
                    "log",
                    "-1",
                    "--format=%s",
                    "--end-of-options",
                    &revision,
                    "--",
                ],
            )
            .map(|s| s.trim().to_string());
            (remote, suggestion, subject)
        });
        let repo = repo.clone();
        cx.spawn_in(window, async move |workbench, cx| {
            let (remote, suggestion, subject) = lookup.await;
            let _ = workbench.update_in(cx, |this, window, cx| {
                this.present_tag_prompt(repo, at, remote, suggestion, subject, window, cx)
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn present_tag_prompt(
        &mut self,
        repo: Entity<RepositoryState>,
        at: String,
        remote: Option<String>,
        suggestion: String,
        subject: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("v1.0.0")
                .default_value(suggestion)
        });
        let message = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Message (optional: makes an annotated tag)")
        });
        let push = Rc::new(std::cell::Cell::new(
            SettingsStore::get(cx).git_push_tags_on_create && remote.is_some(),
        ));
        let workbench = cx.entity().downgrade();
        // The Create Tag button, and Enter in either field.
        let title = if at == "HEAD" {
            "New tag on the current commit".to_string()
        } else {
            format!("New tag at {}", &at[..at.len().min(7)])
        };
        let create: Create = {
            let (name, message, push) = (name.clone(), message.clone(), push.clone());
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                let tag = name.read(cx).value().trim().to_string();
                let text = message.read(cx).value().trim().to_string();
                let push = push.get();
                let repo = repo.clone();
                let _ = workbench.update(cx, |this, cx| {
                    if !impulse_git::types::is_valid_ref_name(&tag) {
                        return this.toast(
                            Notification::warning(format!(
                                "\u{201c}{tag}\u{201d} isn't a valid tag name."
                            )),
                            window,
                            cx,
                        );
                    }
                    let text = (!text.is_empty()).then_some(text);
                    this.git_create_tag(&repo, tag, at.clone(), text, push, false, window, cx)
                });
            })
        };
        let focused = std::cell::Cell::new(false);
        window.open_dialog(cx, move |dialog, window, cx| {
            if !focused.replace(true) {
                name.update(cx, |input, cx| input.focus(window, cx));
            }
            let (on_ok, on_click) = (create.clone(), create.clone());
            let checkbox = push.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .on_ok(move |_, window, cx| {
                    on_ok(window, cx);
                    false
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(subject.clone().map(|s| div().child(s)))
                        .child(Input::new(&name))
                        .child(Input::new(&message))
                        .children(remote.clone().map(|remote| {
                            Checkbox::new("push-tag")
                                .label(format!("Push to {remote}"))
                                .checked(checkbox.get())
                                .on_change(move |checked, window, _| {
                                    checkbox.set(*checked);
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
                            Button::new("tag-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("tag-create")
                                .primary()
                                .label("Create Tag")
                                .on_click(move |_, window, cx| on_click(window, cx)),
                        ),
                )
        });
    }

    /// Tag `at` (a commit, or "HEAD"); an empty message makes a
    /// lightweight tag.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn git_create_tag(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        at: String,
        message: Option<String>,
        push: bool,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (tag, text, revision) = (name.clone(), message.clone(), at.clone());
        let repo_after = repo.clone();
        self.git_run(
            repo,
            Some(format!("Tagging {name}…")),
            None,
            false,
            move |root, _| {
                // A moved tag's old target, so Undo can put it back.
                let previous = force
                    .then(|| operations::resolve_ref(&format!("refs/tags/{tag}"), root))
                    .flatten();
                operations::create_tag(&tag, &revision, text.as_deref(), force, root)?;
                Ok(previous)
            },
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(previous) => {
                    if push {
                        return this.git_push_tag(&repo_after, name, true, force, window, cx);
                    }
                    let repo = repo_after.clone();
                    let verb = if force { "Moved" } else { "Tagged" };
                    this.toast_with_action(
                        format!("{verb} {name}"),
                        None,
                        "Undo",
                        window,
                        cx,
                        move |this, window, cx| {
                            let (name, previous) = (name.clone(), previous.clone());
                            let title = format!(
                                "Couldn't undo {} {name}",
                                if force { "moving" } else { "creating" }
                            );
                            this.git_run(
                                &repo,
                                None,
                                None,
                                false,
                                move |root, _| match &previous {
                                    Some(previous) => operations::restore_ref(
                                        &format!("refs/tags/{name}"),
                                        previous,
                                        root,
                                    ),
                                    None => operations::delete_tag(&name, root),
                                },
                                window,
                                cx,
                                move |this, result, _, window, cx| {
                                    this.report(result, &title, window, cx)
                                },
                            );
                        },
                    );
                }
                Err(OperationError::Cli(error))
                    if error.kind == ErrorKind::TagAlreadyExists && !force =>
                {
                    let repo = repo_after.clone();
                    let target = if at == "HEAD" {
                        "the current commit".to_string()
                    } else {
                        at[..at.len().min(7)].to_string()
                    };
                    this.git_confirm(
                        format!("{name} already exists"),
                        format!(
                            "Move it to {target}? Anyone who already fetched it keeps the old one."
                        ),
                        "Move Tag",
                        true,
                        window,
                        cx,
                        move |this, window, cx| {
                            this.git_create_tag(
                                &repo,
                                name.clone(),
                                at.clone(),
                                message.clone(),
                                push,
                                true,
                                window,
                                cx,
                            )
                        },
                    );
                }
                Err(error) => {
                    this.present_git_error(&error, &format!("Couldn't create {name}"), window, cx)
                }
            },
        );
    }

    /// Push tag `name`; `created` when it was just made.
    pub(crate) fn git_push_tag(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        created: bool,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = name.clone();
        self.git_run(
            repo,
            Some(format!("Pushing {name}…")),
            None,
            false,
            move |root, progress| {
                let remote =
                    operations::default_remote(root, None).unwrap_or_else(|| "origin".into());
                operations::push_tag(&tag, &remote, force, root, Some(progress))?;
                Ok(remote)
            },
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(remote) => {
                    let verb = if created {
                        "Tagged and pushed"
                    } else {
                        "Pushed"
                    };
                    this.toast(
                        Notification::success(format!("{verb} {name} to {remote}")),
                        window,
                        cx,
                    )
                }
                Err(error) => {
                    let title = if created {
                        format!("Tagged {name}, but couldn't push it")
                    } else {
                        format!("Couldn't push {name}")
                    };
                    this.present_git_error(&error, &title, window, cx)
                }
            },
        );
    }

    pub(crate) fn git_push_all_tags(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_run(
            repo,
            Some("Pushing tags…".into()),
            None,
            false,
            |root, progress| {
                let remote =
                    operations::default_remote(root, None).unwrap_or_else(|| "origin".into());
                operations::push_all_tags(&remote, root, Some(progress))?;
                Ok(remote)
            },
            window,
            cx,
            |this, result, _, window, cx| match result {
                Ok(remote) => this.toast(
                    Notification::success(format!("Pushed tags to {remote}")),
                    window,
                    cx,
                ),
                Err(error) => this.present_git_error(&error, "Couldn't push tags", window, cx),
            },
        );
    }

    // MARK: Branches and stash

    /// Switch branches. If local changes would be overwritten, offer to
    /// stash them first (and to pop them afterwards).
    pub(crate) fn git_switch_branch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = name.clone();
        let repo_after = repo.clone();
        self.git_run(
            repo,
            Some(format!("Switching to {name}…")),
            None,
            false,
            move |root, _| operations::switch_branch(&target, root),
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(()) => this.reload_tree(cx),
                Err(OperationError::Cli(error))
                    if error.kind == ErrorKind::LocalChangesWouldBeOverwritten =>
                {
                    let repo = repo_after.clone();
                    this.git_confirm(
                        "Your changes would be overwritten",
                        format!(
                            "Stash your uncommitted changes, then switch to {name}? You can pop \
                             the stash afterwards."
                        ),
                        "Stash & Switch",
                        false,
                        window,
                        cx,
                        move |this, window, cx| {
                            this.git_stash_and_switch(&repo, name.clone(), window, cx)
                        },
                    );
                }
                Err(error) => this.present_git_error(
                    &error,
                    &format!("Couldn't switch to {name}"),
                    window,
                    cx,
                ),
            },
        );
    }

    fn git_stash_and_switch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = name.clone();
        let repo_after = repo.clone();
        self.git_run(
            repo,
            Some(format!("Switching to {name}…")),
            None,
            false,
            move |root, _| {
                operations::stash(
                    &StashOptions {
                        message: Some(format!("Impulse: before switching to {target}")),
                        include_untracked: true,
                        ..Default::default()
                    },
                    root,
                )?;
                let stash = operations::stash_commit(0, root);
                operations::switch_branch(&target, root)?;
                Ok(stash)
            },
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(stash) => {
                    this.reload_tree(cx);
                    let repo = repo_after.clone();
                    this.toast_with_action(
                        format!("Stashed your changes and switched to {name}"),
                        None,
                        "Pop Stash",
                        window,
                        cx,
                        // This stash, even if another was made on top of it since.
                        move |this, window, cx| {
                            this.git_apply_stash(&repo, 0, stash.clone(), true, window, cx)
                        },
                    );
                }
                Err(error) => this.present_git_error(
                    &error,
                    &format!("Couldn't switch to {name}"),
                    window,
                    cx,
                ),
            },
        );
    }

    pub(crate) fn git_create_branch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = name.clone();
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| operations::create_branch(&target, None, true, root),
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(()) => this.toast(
                    Notification::success(format!("Created and switched to {name}")),
                    window,
                    cx,
                ),
                Err(error) => {
                    this.present_git_error(&error, &format!("Couldn't create {name}"), window, cx)
                }
            },
        );
    }

    pub(crate) fn git_stash_all(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_run(
            repo,
            Some("Stashing…".into()),
            None,
            false,
            |root, _| {
                operations::stash(
                    &StashOptions {
                        include_untracked: true,
                        ..Default::default()
                    },
                    root,
                )
            },
            window,
            cx,
            |this, result, _, window, cx| match result {
                Ok(()) => {
                    this.reload_tree(cx);
                    this.toast(Notification::success("Stashed your changes"), window, cx)
                }
                Err(error) => this.present_git_error(&error, "Couldn't stash", window, cx),
            },
        );
    }

    pub(crate) fn git_pop_latest_stash(
        &mut self,
        repo: &Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = repo.read(cx).root.clone();
        let lookup =
            cx.background_spawn(async move { operations::stash_list(&root).into_iter().next() });
        let repo = repo.clone();
        cx.spawn_in(window, async move |workbench, cx| {
            let latest = lookup.await;
            let _ = workbench.update_in(cx, |this, window, cx| match latest {
                Some(latest) => {
                    this.git_apply_stash(&repo, 0, Some(latest.commit), true, window, cx)
                }
                None => this.toast(Notification::info("There are no stashes."), window, cx),
            });
        })
        .detach();
    }

    /// Apply or pop a stash: the one at `commit` wherever it has moved to
    /// in the list, or without one, whatever is at `index` when it runs.
    pub(crate) fn git_apply_stash(
        &mut self,
        repo: &Entity<RepositoryState>,
        index: usize,
        commit: Option<String>,
        pop: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            None,
            // Popping drops the entry: snapshot the working tree so Undo can
            // put both back.
            pop.then(|| "pop stash".to_string()),
            false,
            move |root, _| {
                let entry =
                    operations::stash_entry(index, commit.as_deref(), root).ok_or_else(|| {
                        OperationError::Stale("That stash isn't in the list anymore.".into())
                    })?;
                operations::stash_apply(entry.index, pop, root)?;
                Ok(entry)
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(error) => {
                        let title = if pop {
                            "Couldn't pop the stash"
                        } else {
                            "Couldn't apply the stash"
                        };
                        return this.present_git_error(&error, title, window, cx);
                    }
                };
                this.reload_tree(cx);
                let (true, Some(snapshot)) = (pop, snapshot) else {
                    return;
                };
                let repo = repo_undo.clone();
                this.toast_with_action(
                    format!("Popped \u{201c}{}\u{201d}", entry.message),
                    None,
                    "Undo",
                    window,
                    cx,
                    move |this, window, cx| {
                        let (entry, snapshot) = (entry.clone(), snapshot.clone());
                        // Snapshot first: edits made since the pop are kept for Undo.
                        this.git_run(
                            &repo,
                            None,
                            Some("before undo".into()),
                            true,
                            move |root, _| {
                                operations::undo_stash_pop(
                                    &entry.commit,
                                    &entry.message,
                                    &snapshot,
                                    root,
                                )
                            },
                            window,
                            cx,
                            |this, result, _, window, cx| {
                                this.reload_tree(cx);
                                this.report(result, "Couldn't undo the pop", window, cx)
                            },
                        );
                    },
                );
            },
        );
    }

    /// Drop a stash; the notification's Undo stores it again.
    pub(crate) fn git_drop_stash(
        &mut self,
        repo: &Entity<RepositoryState>,
        index: usize,
        commit: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| {
                let entry =
                    operations::stash_entry(index, commit.as_deref(), root).ok_or_else(|| {
                        OperationError::Stale("That stash isn't in the list anymore.".into())
                    })?;
                operations::stash_drop(entry.index, root)?;
                Ok(entry)
            },
            window,
            cx,
            move |this, result, _, window, cx| {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(error) => {
                        return this.present_git_error(
                            &error,
                            "Couldn't drop the stash",
                            window,
                            cx,
                        )
                    }
                };
                let repo = repo_undo.clone();
                this.toast_with_action(
                    format!("Dropped \u{201c}{}\u{201d}", entry.message),
                    None,
                    "Undo",
                    window,
                    cx,
                    move |this, window, cx| {
                        let entry = entry.clone();
                        this.git_run(
                            &repo,
                            None,
                            None,
                            false,
                            move |root, _| {
                                operations::stash_store(&entry.commit, &entry.message, root)
                            },
                            window,
                            cx,
                            |this, result, _, window, cx| {
                                this.report(result, "Couldn't restore the stash", window, cx)
                            },
                        );
                    },
                );
            },
        );
    }
}
