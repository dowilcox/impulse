//! Manage Branches (the Swift `BranchManagerModel`/`BranchManagerView`):
//! every local branch with its upstream, how far it's ahead or behind,
//! when it last changed and whether it's merged; switch, merge, rebase,
//! compare, rename, publish or delete from here.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use crate::toasts::Toast;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    div, prelude::*, px, Context, Entity, FontWeight, Subscription, WeakEntity, Window,
};
use impulse_git::operations::{self, BranchInfo, OperationError, PushOptions};
use impulse_git::DiffScope;
use impulse_kit::chrome::{ChromePalette, Rgba};

use crate::context_menu::{DropdownMenuButton as _, MenuSpec};
use crate::repository_state::RepositoryState;
use crate::workbench::Workbench;
use crate::workbench_chrome::{chrome_button, hsla, icon, ButtonKind};

/// After this long without a commit a branch is stale.
const STALE: Duration = Duration::from_secs(30 * 24 * 3600);

type Then = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

pub struct BranchManager {
    workbench: WeakEntity<Workbench>,
    repo: Entity<RepositoryState>,
    branches: Vec<BranchInfo>,
    base: Option<String>,
    /// Where each unpublished branch would be published (none without a
    /// remote).
    publish_remotes: HashMap<String, String>,
    loading: bool,
    filter: Entity<InputState>,
    palette: ChromePalette,
    _subscriptions: Vec<Subscription>,
}

impl BranchManager {
    fn new(
        workbench: WeakEntity<Workbench>,
        repo: Entity<RepositoryState>,
        palette: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let subscriptions = vec![cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        let mut manager = BranchManager {
            workbench,
            repo,
            branches: Vec::new(),
            base: None,
            publish_remotes: HashMap::new(),
            loading: true,
            filter,
            palette,
            _subscriptions: subscriptions,
        };
        manager.reload(cx);
        manager
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        let root = self.repo.read(cx).root.clone();
        let task = cx.background_spawn(async move {
            let base = impulse_git::scoped_diff::default_base_branch(&root)
                .map(|b| b.strip_prefix("origin/").map(str::to_string).unwrap_or(b));
            let list = operations::branch_details(&root, base.as_deref());
            let unpublished: Vec<&str> = list
                .iter()
                .filter(|b| b.upstream.is_none())
                .map(|b| b.name.as_str())
                .collect();
            let remotes = operations::default_remotes(&root, &unpublished);
            (base, list, remotes)
        });
        cx.spawn(async move |this, cx| {
            let (base, list, remotes) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.base = base;
                this.branches = list;
                this.publish_remotes = remotes;
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// The checked-out branch.
    fn current(&self) -> Option<&str> {
        self.branches
            .iter()
            .find(|b| b.is_current)
            .map(|b| b.name.as_str())
    }

    fn is_stale(branch: &BranchInfo) -> bool {
        !branch.is_current
            && SystemTime::now()
                .duration_since(branch.last_commit)
                .is_ok_and(|age| age > STALE)
    }

    /// Run `act` on the window; `leaves` closes the dialog first (the
    /// action shows something else).
    fn act(
        &self,
        leaves: bool,
        act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> impl Fn(&mut Window, &mut gpui_kit::App) + 'static {
        let workbench = self.workbench.clone();
        let act: Then = Rc::new(act);
        move |window, cx| {
            if leaves {
                window.close_dialog(cx);
            }
            let act = act.clone();
            let _ = workbench.update(cx, |workbench, cx| act(workbench, window, cx));
        }
    }

    fn badge(&self, text: &'static str, color: Rgba) -> impl IntoElement {
        div()
            .px(px(5.))
            .h(px(15.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .bg(hsla(color.opacity(0.14)))
            .text_color(hsla(color))
            .text_size(px(9.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text)
    }

    fn row(&self, index: usize, branch: &BranchInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let me = cx.entity();
        let repo = self.repo.clone();
        let name = branch.name.clone();
        let mut detail = vec![branch
            .upstream
            .as_ref()
            .map(|u| format!("→ {u}"))
            .unwrap_or_else(|| "not published".into())];
        if branch.ahead > 0 {
            detail.push(format!("↑{}", branch.ahead));
        }
        if branch.behind > 0 {
            detail.push(format!("↓{}", branch.behind));
        }
        if let Ok(age) = SystemTime::now().duration_since(branch.last_commit) {
            detail.push(impulse_kit::dates::abbreviated_relative(
                age.as_secs() as i64
            ));
        }
        if !branch.subject.is_empty() {
            detail.push(branch.subject.clone());
        }
        let current = self.current().unwrap_or("Current Branch").to_string();
        let publish_remote = branch
            .upstream
            .is_none()
            .then(|| self.publish_remotes.get(&branch.name).cloned())
            .flatten();
        let base = self.base.clone();
        let is_current = branch.is_current;

        let switch = {
            let (repo, name) = (repo.clone(), name.clone());
            self.act(true, move |w, window, cx| {
                w.git_switch_branch(&repo, name.clone(), window, cx)
            })
        };
        let menu_items = {
            let mut items: Vec<(String, bool, MenuAction)> = Vec::new();
            if !is_current {
                let (r, n) = (repo.clone(), name.clone());
                items.push((
                    format!("Merge into {current}"),
                    true,
                    Box::new(self.act(true, move |w, window, cx| {
                        w.git_merge(&r, n.clone(), window, cx)
                    })),
                ));
                let (r, n) = (repo.clone(), name.clone());
                items.push((
                    format!("Rebase {current} onto This"),
                    true,
                    Box::new(self.act(true, move |w, window, cx| {
                        w.git_rebase(&r, n.clone(), window, cx)
                    })),
                ));
                let n = name.clone();
                items.push((
                    "Compare with Current Branch".into(),
                    true,
                    Box::new(self.act(true, move |w, window, cx| {
                        w.open_review(Some(DiffScope::Branch { base: n.clone() }), window, cx)
                    })),
                ));
            }
            let (r, n) = (repo.clone(), name.clone());
            items.push((
                "Show History".into(),
                true,
                Box::new(self.act(true, move |w, window, cx| {
                    let root = r.read(cx).root.clone();
                    let branch = (!is_current).then(|| n.clone());
                    let surface = w.make_history(root, None, branch, window, cx);
                    let active = w.active;
                    w.add_tab(active, surface, window, cx);
                })),
            ));
            let (r, n, m) = (repo.clone(), name.clone(), me.clone());
            items.push((
                "Rename…".into(),
                true,
                Box::new(self.act(false, move |w, window, cx| {
                    w.git_rename_branch(&r, n.clone(), m.clone(), window, cx)
                })),
            ));
            if let Some(remote) = publish_remote {
                let (r, n, m) = (repo.clone(), name.clone(), me.clone());
                items.push((
                    format!("Publish to {remote}"),
                    true,
                    Box::new(self.act(false, move |w, window, cx| {
                        w.git_publish_branch(&r, n.clone(), remote.clone(), m.clone(), window, cx)
                    })),
                ));
            }
            let (r, n, m) = (repo.clone(), name.clone(), me.clone());
            items.push((
                "Delete…".into(),
                !is_current,
                Box::new(self.act(false, move |w, window, cx| {
                    let m = m.clone();
                    w.git_delete_branch(
                        &r,
                        n.clone(),
                        base.clone(),
                        false,
                        Rc::new(move |_, _, cx| m.update(cx, |m, cx| m.reload(cx))),
                        window,
                        cx,
                    )
                })),
            ));
            Rc::new(items)
        };
        div()
            .id(("branch", index))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .h(px(44.))
            .child(icon(
                if is_current {
                    "circle-dot"
                } else {
                    "git-branch"
                },
                12.,
                if is_current {
                    p.accent
                } else {
                    p.text_tertiary
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.5))
                                    .text_color(hsla(p.text))
                                    .when(is_current, |d| d.font_weight(FontWeight::SEMIBOLD))
                                    .child(branch.name.clone()),
                            )
                            .when(branch.is_merged, |d| {
                                d.child(self.badge("merged", p.success))
                            })
                            .when(Self::is_stale(branch), |d| {
                                d.child(self.badge("stale", p.text_tertiary))
                            })
                            .when(branch.upstream_gone, |d| {
                                d.child(self.badge("upstream gone", p.warning))
                            }),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(10.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(detail.join(" · ")),
                    ),
            )
            .when(!is_current, |d| {
                d.child(
                    chrome_button(
                        ("switch", index),
                        "Switch",
                        None,
                        ButtonKind::Secondary,
                        false,
                        p,
                    )
                    .on_click(move |_, window, cx| switch(window, cx)),
                )
            })
            .child(
                Button::new(("more", index))
                    .ghost()
                    .small()
                    .icon(Icon::empty().path("icons/ellipsis.svg"))
                    // Swift's `BranchRow` menu: the merge and rebase apart
                    // from the rest, and Delete at the end.
                    .dropdown(move |_, _| {
                        let mut menu = MenuSpec::new();
                        for (index, (label, enabled, _)) in menu_items.iter().enumerate() {
                            if label == "Delete…" || label == "Compare with Current Branch" {
                                menu = menu.separator();
                            }
                            let items = menu_items.clone();
                            menu = menu.item_enabled(label.clone(), *enabled, move |window, cx| {
                                (items[index].2)(window, cx)
                            });
                        }
                        menu
                    }),
            )
    }
}

impl Render for BranchManager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let query = self.filter.read(cx).value().trim().to_lowercase();
        let visible: Vec<(usize, BranchInfo)> = self
            .branches
            .iter()
            .filter(|b| query.is_empty() || b.name.to_lowercase().contains(&query))
            .cloned()
            .enumerate()
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(20.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(hsla(p.text))
                            .child("Branches"),
                    )
                    .children(self.base.as_ref().map(|base| {
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(format!("merged means merged into {base}"))
                    }))
                    .child(div().flex_1())
                    .child(crate::alert::field(&self.filter, 180., 22.)),
            )
            // The box stays put while the list scrolls inside it.
            .child(
                div().h(px(340.)).rounded(px(6.)).bg(hsla(p.content)).child(
                    div()
                        .id("branch-list")
                        .size_full()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .children(visible.iter().map(|(i, b)| self.row(*i, b, cx))),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(self.loading, |d| {
                        d.child(crate::workbench_chrome::progress_ring(
                            None,
                            p.text_tertiary,
                            12.,
                            1.5,
                            0.,
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        chrome_button(
                            "branches-done",
                            "Done",
                            None,
                            ButtonKind::Primary,
                            false,
                            &p,
                        )
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

impl Workbench {
    pub(crate) fn show_branch_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repository_or_note(window, cx) else {
            return;
        };
        let workbench = cx.entity().downgrade();
        let palette = self.palette.clone();
        let manager = cx.new(|cx| BranchManager::new(workbench, repo, palette, window, cx));
        let background = hsla(self.palette.overlay);
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .w(px(640.))
                .p_0()
                .bg(background)
                .close_button(false)
                .child(manager.clone())
        });
    }

    /// Merge `revision` into the current branch, with Undo. A task's
    /// branch that's still moving (an agent busy there, files not committed)
    /// asks first, and merges the commit it has now (Swift's
    /// `TaskActivity`).
    pub(crate) fn git_merge(
        &mut self,
        repo: &Entity<RepositoryState>,
        revision: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = repo.read(cx).root.clone();
        let wanted = vec![revision.clone()];
        let find = cx.background_spawn(async move {
            crate::agent_hooks::merge_candidates(&wanted, &root)
                .into_iter()
                .next()
        });
        let repo = repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let candidate = find.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(task) = candidate else {
                    return this.merge_now(&repo, revision, window, cx);
                };
                let agents: Vec<_> = crate::agent_hooks::agents_in_folder(&task.path, cx)
                    .into_iter()
                    .filter(|(_, s)| {
                        matches!(
                            s,
                            impulse_kit::agents::state_machine::AgentState::Working
                                | impulse_kit::agents::state_machine::AgentState::NeedsInput
                        )
                    })
                    .collect();
                let Some(question) = crate::agent_hooks::merge_question(
                    &task.branch,
                    &task.path,
                    &task.commit,
                    task.uncommitted,
                    task.last_commit,
                    &agents,
                ) else {
                    return this.merge_now(&repo, revision, window, cx);
                };
                let title = format!("{} is still being worked on", task.branch);
                let message = question
                    .strip_prefix(&format!("{title}: "))
                    .unwrap_or(&question)
                    .to_string();
                let short: String = task.commit.chars().take(7).collect();
                let workbench = cx.entity().downgrade();
                let (commit, label) = (task.commit.clone(), format!("{revision} at {short}"));
                crate::alert::Alert::new(title)
                    .message(message)
                    .button(format!("Merge {short}"), move |window, cx| {
                        window.close_dialog(cx);
                        let _ = workbench.update(cx, |w, cx| {
                            w.merge_now_labelled(&repo, commit.clone(), label.clone(), window, cx)
                        });
                    })
                    .button("Cancel", |window, cx| window.close_dialog(cx))
                    .show(window, cx);
            });
        })
        .detach();
    }

    fn merge_now(
        &mut self,
        repo: &Entity<RepositoryState>,
        revision: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A commit reads by its short SHA, as Swift labels it.
        let is_sha = revision.len() >= 40 && revision.chars().all(|c| c.is_ascii_hexdigit());
        let label = if is_sha {
            revision.chars().take(7).collect()
        } else {
            revision.clone()
        };
        self.merge_now_labelled(repo, revision, label, window, cx)
    }

    fn merge_now_labelled(
        &mut self,
        repo: &Entity<RepositoryState>,
        revision: String,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let into = repo
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|s| s.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        self.git_move_head(
            repo,
            label.clone(),
            format!("Merging {label}…"),
            format!("merge {label}"),
            move |root| operations::merge(&revision, false, root),
            (
                format!("Merged {{}} into {into}"),
                format!("{into} already has {{}}"),
                "Merge of {} stopped",
                "Couldn't undo the merge",
            ),
            window,
            cx,
        );
    }

    /// Rebase the current branch onto `revision`, with Undo.
    pub(crate) fn git_rebase(
        &mut self,
        repo: &Entity<RepositoryState>,
        revision: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let branch = repo
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|s| s.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        self.git_move_head(
            repo,
            revision.clone(),
            format!("Rebasing onto {revision}…"),
            format!("rebase onto {revision}"),
            move |root| operations::rebase(&revision, root),
            (
                format!("Rebased {branch} onto {{}}"),
                format!("{branch} is already on top of {{}}"),
                "Rebase onto {} stopped",
                "Couldn't undo the rebase",
            ),
            window,
            cx,
        );
    }

    /// A merge or rebase: run it with a safety snapshot, then offer Undo,
    /// or say nothing moved. `messages`: done, nothing to do, stopped (each
    /// with `{}` for the label), and the Undo failure.
    #[allow(clippy::too_many_arguments)]
    fn git_move_head(
        &mut self,
        repo: &Entity<RepositoryState>,
        label: String,
        progress: String,
        reason: String,
        operation: impl FnOnce(&str) -> impulse_git::operations::GitResult + Send + 'static,
        messages: (String, String, &'static str, &'static str),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_undo = repo.clone();
        self.git_run(
            repo,
            Some(progress),
            Some(reason),
            false,
            move |root, _| {
                let before = operations::resolve_ref("HEAD", root);
                operation(root)?;
                Ok((before, operations::resolve_ref("HEAD", root)))
            },
            window,
            cx,
            move |this, result, snapshot, window, cx| {
                let (done, nothing, stopped, undo_failure) = messages;
                match result {
                    Err(error) => {
                        this.present_git_error(&error, &stopped.replace("{}", &label), window, cx)
                    }
                    Ok((Some(before), after)) if after.as_ref() != Some(&before) => this
                        .offer_head_undo(
                            &repo_undo,
                            done.replace("{}", &label),
                            before,
                            snapshot,
                            undo_failure,
                            window,
                            cx,
                        ),
                    Ok(_) => this.toast(Toast::info(nothing.replace("{}", &label)), window, cx),
                }
            },
        );
    }

    /// Delete a local branch, with Undo; one that isn't merged asks first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn git_delete_branch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        base: Option<String>,
        force: bool,
        done: Then,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = name.clone();
        let repo_after = repo.clone();
        self.git_run(
            repo,
            None,
            None,
            false,
            move |root, _| {
                // Where it was, for Undo.
                let sha = operations::resolve_ref(&format!("refs/heads/{target}"), root);
                operations::delete_branch(&target, force, root)?;
                Ok(sha)
            },
            window,
            cx,
            move |this, result, _, window, cx| {
                done(this, window, cx);
                match result {
                    Ok(sha) => {
                        let Some(sha) = sha else {
                            return this.toast(
                                Toast::success(format!("Deleted {name}")),
                                window,
                                cx,
                            );
                        };
                        let (repo, done) = (repo_after.clone(), done.clone());
                        this.toast_with_action(
                            format!("Deleted {name}"),
                            None,
                            "Undo",
                            window,
                            cx,
                            move |this, window, cx| {
                                let (name, sha, done) = (name.clone(), sha.clone(), done.clone());
                                let failure = format!("Couldn't restore {name}");
                                this.git_run(
                                    &repo,
                                    None,
                                    None,
                                    false,
                                    move |root, _| {
                                        operations::create_branch(&name, Some(&sha), false, root)
                                    },
                                    window,
                                    cx,
                                    move |this, result, _, window, cx| {
                                        this.report(result, &failure, window, cx);
                                        done(this, window, cx);
                                    },
                                );
                            },
                        );
                    }
                    Err(OperationError::Cli(error))
                        if !force && error.output.contains("not fully merged") =>
                    {
                        // Not merged: say what would be lost, then force.
                        let repo = repo_after.clone();
                        let into = base.clone().unwrap_or_else(|| "the current branch".into());
                        this.git_confirm(
                            format!("{name} isn't merged"),
                            format!(
                                "Its commits aren't in {into} yet. Delete it anyway? Undo stays \
                                 available for a few seconds."
                            ),
                            "Delete",
                            true,
                            window,
                            cx,
                            move |this, window, cx| {
                                this.git_delete_branch(
                                    &repo,
                                    name.clone(),
                                    base.clone(),
                                    true,
                                    done.clone(),
                                    window,
                                    cx,
                                )
                            },
                        );
                    }
                    Err(error) => this.present_git_error(
                        &error,
                        &format!("Couldn't delete {name}"),
                        window,
                        cx,
                    ),
                }
            },
        );
    }

    fn git_rename_branch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        manager: Entity<BranchManager>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = repo.clone();
        let title = format!("Rename {name}");
        self.text_prompt(
            &title,
            "",
            &name.clone(),
            "Rename",
            window,
            cx,
            move |this, new_name, window, cx| {
                let new_name = new_name.trim().to_string();
                if new_name == name {
                    return;
                }
                if !impulse_kit::git::refs::is_valid_ref_name(&new_name) {
                    return this.toast(
                        Toast::warning(format!("“{new_name}” isn't a valid branch name.")),
                        window,
                        cx,
                    );
                }
                let (old, manager) = (name.clone(), manager.clone());
                this.git_run(
                    &repo,
                    None,
                    None,
                    false,
                    move |root, _| operations::rename_branch(&old, &new_name, root),
                    window,
                    cx,
                    move |this, result, _, window, cx| {
                        this.report(result, "Couldn't rename", window, cx);
                        manager.update(cx, |m, cx| m.reload(cx));
                    },
                );
            },
        );
    }

    fn git_publish_branch(
        &mut self,
        repo: &Entity<RepositoryState>,
        name: String,
        remote: String,
        manager: Entity<BranchManager>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (branch, to) = (name.clone(), remote.clone());
        self.git_run(
            repo,
            Some(format!("Publishing {name}…")),
            None,
            false,
            move |root, progress| {
                operations::push(
                    &PushOptions {
                        set_upstream: true,
                        remote: to,
                        branch: Some(branch),
                        ..Default::default()
                    },
                    root,
                    Some(progress),
                )
            },
            window,
            cx,
            move |this, result, _, window, cx| {
                match result {
                    Ok(()) => this.toast(
                        Toast::success(format!("Published {name} to {remote}")),
                        window,
                        cx,
                    ),
                    Err(error) => this.present_git_error(&error, "Couldn't publish", window, cx),
                }
                manager.update(cx, |m, cx| m.reload(cx));
            },
        );
    }
}

/// A menu item's action.
type MenuAction = Box<dyn Fn(&mut Window, &mut gpui_kit::App)>;
