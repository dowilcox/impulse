//! What History's menus ask of the workbench (the Swift
//! `HistorySurface.perform`): check out a commit, branch or tag there,
//! merge, rebase, cherry-pick, revert and reset with Undo, switch to,
//! delete or push the refs on it, and open compares in Review.

use std::rc::Rc;

use crate::toasts::Toast;
use gpui_kit::{Context, Entity, Window};
use impulse_git::operations::{self, ResetMode};

use crate::history_view::{HistoryAction, HistoryEvent};
use crate::repository_state::{RepositoryState, RepositoryStore};
use crate::workbench::Workbench;

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn mode_name(mode: ResetMode) -> &'static str {
    match mode {
        ResetMode::Soft => "soft",
        ResetMode::Mixed => "mixed",
        ResetMode::Hard => "hard",
        ResetMode::Keep => "keep",
    }
}

impl Workbench {
    pub(crate) fn history_action(
        &mut self,
        root: String,
        event: &HistoryEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = RepositoryStore::state(&root, cx);
        let entry = &event.entry;
        let sha = entry.sha.clone();
        match event.action.clone() {
            HistoryAction::Checkout => {
                let label = format!("Checking out {}…", short(&sha));
                let failure = format!("Couldn't check out {}", short(&sha));
                self.git_run(
                    &repo,
                    Some(label),
                    None,
                    false,
                    move |root, _| operations::checkout_detached(&sha, root),
                    window,
                    cx,
                    move |this, result, _, window, cx| this.report(result, &failure, window, cx),
                );
            }
            HistoryAction::BranchHere => {
                let title = format!("New branch at {}", entry.short_sha());
                self.text_prompt(
                    &title,
                    &entry.subject.clone(),
                    "",
                    "Create Branch",
                    window,
                    cx,
                    move |this, name, window, cx| {
                        let name = name.trim().to_string();
                        if !impulse_git::types::is_valid_ref_name(&name) {
                            return this.toast(
                                Toast::warning(format!(
                                    "\u{201c}{name}\u{201d} isn't a valid branch name."
                                )),
                                window,
                                cx,
                            );
                        }
                        this.git_create_branch_at(&repo, name, sha.clone(), window, cx)
                    },
                );
            }
            HistoryAction::TagHere => self.git_ask_for_tag_at(&repo, sha, window, cx),
            HistoryAction::CherryPick => self.git_head_moving(
                &repo,
                format!("Cherry-picking {}…", short(&sha)),
                format!("cherry-pick {}", short(&sha)),
                false,
                {
                    let sha = sha.clone();
                    move |root| operations::cherry_pick(&sha, root)
                },
                format!("Cherry-picked {}", short(&sha)),
                "Cherry-pick stopped",
                "Couldn't undo the cherry-pick",
                window,
                cx,
            ),
            HistoryAction::Revert => self.git_head_moving(
                &repo,
                format!("Reverting {}…", short(&sha)),
                format!("revert {}", short(&sha)),
                false,
                {
                    let sha = sha.clone();
                    move |root| operations::revert(&sha, root)
                },
                format!("Reverted {}", short(&sha)),
                "Revert stopped",
                "Couldn't undo the revert",
                window,
                cx,
            ),
            HistoryAction::Reset(mode) => {
                let run = move |this: &mut Workbench,
                                window: &mut Window,
                                cx: &mut Context<Workbench>| {
                    let sha = sha.clone();
                    this.git_head_moving(
                        &repo,
                        "Resetting…".into(),
                        format!("reset --{} {}", mode_name(mode), short(&sha)),
                        mode == ResetMode::Hard,
                        {
                            let sha = sha.clone();
                            move |root| operations::reset(mode, &sha, root)
                        },
                        format!("Reset to {} ({})", short(&sha), mode_name(mode)),
                        "Couldn't reset",
                        "Couldn't undo the reset",
                        window,
                        cx,
                    )
                };
                if mode != ResetMode::Hard {
                    return run(self, window, cx);
                }
                self.git_confirm(
                    format!("Reset hard to {}?", entry.short_sha()),
                    "The branch moves to this commit and uncommitted changes are thrown away. \
                     A snapshot is kept so Undo can bring everything back.",
                    "Reset",
                    true,
                    window,
                    cx,
                    run,
                );
            }
            HistoryAction::Merge(revision) => self.git_merge(&repo, revision, window, cx),
            HistoryAction::RebaseOnto(revision) => self.git_rebase(&repo, revision, window, cx),
            HistoryAction::Compare(scope) => self.open_review_of(root, Some(scope), window, cx),
            HistoryAction::SwitchToBranch(name) => self.git_switch_branch(&repo, name, window, cx),
            HistoryAction::DeleteBranch(name) => {
                self.git_delete_branch(&repo, name, None, false, Rc::new(|_, _, _| {}), window, cx)
            }
            HistoryAction::PushTag(name) => {
                self.git_push_tag(&repo, name, false, false, window, cx)
            }
            HistoryAction::DeleteTag(name) => self.git_delete_tag(&repo, name, window, cx),
            HistoryAction::DeleteRemoteTag(name) => {
                self.git_delete_remote_tag(&repo, name, window, cx)
            }
        }
    }

    /// Create branch `name` at `at` and switch to it.
    fn git_create_branch_at(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        at: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = name.clone();
        self.git_run(
            repo,
            Some(format!("Creating {name}…")),
            None,
            false,
            move |root, _| operations::create_branch(&target, Some(&at), true, root),
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Ok(()) => this.toast(
                    Toast::success(format!("Created and switched to {name}")),
                    window,
                    cx,
                ),
                Err(error) => {
                    this.present_git_error(&error, &format!("Couldn't create {name}"), window, cx)
                }
            },
        );
    }

    /// Run `operation`, which moves HEAD, with a safety snapshot; then offer
    /// Undo when HEAD moved (cherry-pick, revert, reset).
    #[allow(clippy::too_many_arguments)]
    fn git_head_moving(
        &mut self,
        repo: &Entity<RepositoryState>,
        progress: String,
        reason: String,
        require_snapshot: bool,
        operation: impl FnOnce(&str) -> operations::GitResult + Send + 'static,
        done: String,
        stopped: &'static str,
        undo_failure: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            Some(progress),
            Some(reason),
            require_snapshot,
            move |root, _| {
                let before = operations::resolve_ref("HEAD", root);
                operation(root)?;
                Ok((before, operations::resolve_ref("HEAD", root)))
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| match result {
                Err(error) => this.present_git_error(&error, stopped, window, cx),
                Ok((Some(before), after)) if after.as_ref() != Some(&before) => this
                    .offer_head_undo(&repo_undo, done, before, snapshot, undo_failure, window, cx),
                Ok(_) => {}
            },
        );
    }

    /// Delete a local tag. Undo puts it back, annotation included.
    fn git_delete_tag(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = name.clone();
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| {
                let object = operations::resolve_ref(&format!("refs/tags/{tag}"), root);
                operations::delete_tag(&tag, root)?;
                Ok(object)
            },
            window,
            cx,
            move |this, result, _, window, cx| match result {
                Err(error) => {
                    this.present_git_error(&error, &format!("Couldn't delete {name}"), window, cx)
                }
                Ok(None) => this.toast(Toast::success(format!("Deleted tag {name}")), window, cx),
                Ok(Some(object)) => {
                    let repo = repo_undo.clone();
                    this.toast_with_action(
                        format!("Deleted tag {name}"),
                        None,
                        "Undo",
                        window,
                        cx,
                        move |this, window, cx| {
                            let (name, object) = (name.clone(), object.clone());
                            let failure = format!("Couldn't restore {name}");
                            this.git_run(
                                &repo,
                                None,
                                None,
                                false,
                                move |root, _| {
                                    operations::restore_ref(
                                        &format!("refs/tags/{name}"),
                                        &object,
                                        root,
                                    )
                                },
                                window,
                                cx,
                                move |this, result, _, window, cx| {
                                    this.report(result, &failure, window, cx)
                                },
                            );
                        },
                    );
                }
            },
        );
    }

    /// Delete a tag on the remote, after asking (it can't be undone there).
    fn git_delete_remote_tag(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = repo.read(cx).root.clone();
        let remote = operations::default_remote(&root, None).unwrap_or_else(|| "origin".into());
        let repo = repo.clone();
        self.git_confirm(
            format!("Delete {name} from {remote}?"),
            "The tag is removed from the remote; your local tag stays. Anyone who already \
             fetched it keeps their copy.",
            "Delete",
            true,
            window,
            cx,
            move |this, window, cx| {
                let (tag, remote_name, name, remote) =
                    (name.clone(), remote.clone(), name.clone(), remote.clone());
                this.git_run(
                    &repo,
                    Some(format!("Deleting {name} from {remote}…")),
                    None,
                    false,
                    move |root, _| operations::delete_remote_tag(&tag, &remote_name, root),
                    window,
                    cx,
                    move |this, result, _, window, cx| match result {
                        Ok(()) => this.toast(
                            Toast::success(format!("Deleted {name} from {remote}")),
                            window,
                            cx,
                        ),
                        Err(error) => this.present_git_error(
                            &error,
                            &format!("Couldn't delete {name} from {remote}"),
                            window,
                            cx,
                        ),
                    },
                );
            },
        );
    }
}
