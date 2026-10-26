//! History's menus (the Swift `HistoryRowView.menu` and `RefMenuItems`): a
//! commit's, and one branch or tag's, both on its chip and as a submenu of
//! its commit's.

use gpui_kit::{App, Context, Entity, Window};
use impulse_git::log::LogEntry;
use impulse_git::operations::ResetMode;
use impulse_git::types::DiffScope;
use impulse_kit::git::refs::RefDecoration;

use super::{HistoryAction, HistoryContext, HistoryEvent, HistoryView};
use crate::context_menu::MenuSpec;

/// What an item that asks the workbench for `action` on `entry` runs.
fn ask(
    view: &Entity<HistoryView>,
    action: HistoryAction,
    entry: &LogEntry,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let (view, entry) = (view.clone(), entry.clone());
    move |_, cx| {
        let (action, entry) = (action.clone(), entry.clone());
        view.update(cx, |_, cx| cx.emit(HistoryEvent { action, entry }))
    }
}

/// What an item that runs `act` on the view runs.
fn local(
    view: &Entity<HistoryView>,
    act: impl Fn(&mut HistoryView, &mut Context<HistoryView>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let view = view.clone();
    move |_, cx| view.update(cx, |view, cx| act(view, cx))
}

fn copy(text: String) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_, cx| HistoryView::copy(text.clone(), cx)
}

/// "Tag v1.0", "Remote Branch origin/main", "Branch main".
fn menu_title(decoration: &RefDecoration) -> String {
    match decoration {
        RefDecoration::Tag(name) => format!("Tag {name}"),
        RefDecoration::RemoteBranch { .. } => format!("Remote Branch {}", decoration.label()),
        _ => format!("Branch {}", decoration.label()),
    }
}

/// Merging or rebasing onto a commit uses the branch on it, when there is
/// one (for the merge message), else the commit.
fn merge_target(decorations: &[RefDecoration], entry: &LogEntry) -> String {
    for decoration in decorations {
        match decoration {
            RefDecoration::LocalBranch(name) | RefDecoration::Tag(name) => return name.clone(),
            RefDecoration::RemoteBranch { remote, branch } if branch != "HEAD" => {
                return format!("{remote}/{branch}")
            }
            _ => {}
        }
    }
    entry.sha.clone()
}

/// A commit's menu (`HistoryRowView.menu`).
pub(super) fn commit_items(view: &Entity<HistoryView>, entry: &LogEntry, cx: &App) -> MenuSpec {
    let (context, compare_base, decorations, entries_order) = {
        let history = view.read(cx);
        let position = |sha: &str| history.entries.iter().position(|e| e.sha == sha);
        let order = history
            .compare_base
            .as_ref()
            .map(|base| (position(&base.sha), position(&entry.sha)));
        (
            history.context.clone(),
            history.compare_base.clone(),
            history.refs(entry),
            order,
        )
    };
    let current = context.branch.clone().unwrap_or_else(|| "HEAD".into());
    let is_head = decorations
        .iter()
        .any(|d| matches!(d, RefDecoration::Head { .. } | RefDecoration::DetachedHead));
    let target = merge_target(&decorations, entry);
    let mut menu = MenuSpec::new()
        .item(
            "Check Out (Detached)",
            ask(view, HistoryAction::Checkout, entry),
        )
        .item(
            "Create Branch Here…",
            ask(view, HistoryAction::BranchHere, entry),
        )
        .item("Create Tag Here…", ask(view, HistoryAction::TagHere, entry))
        .separator()
        .item_enabled(
            format!("Merge into {current}"),
            !is_head,
            ask(view, HistoryAction::Merge(target.clone()), entry),
        )
        .item_enabled(
            format!("Rebase {current} onto Here"),
            !(is_head || context.branch.is_none()),
            ask(view, HistoryAction::RebaseOnto(target), entry),
        )
        .item("Cherry-Pick", ask(view, HistoryAction::CherryPick, entry))
        .item("Revert", ask(view, HistoryAction::Revert, entry))
        .submenu(
            format!("Reset {current} Here"),
            MenuSpec::new()
                .item(
                    "Soft (keep changes staged)",
                    ask(view, HistoryAction::Reset(ResetMode::Soft), entry),
                )
                .item(
                    "Mixed (keep changes)",
                    ask(view, HistoryAction::Reset(ResetMode::Mixed), entry),
                )
                .item(
                    "Hard (discard changes)…",
                    ask(view, HistoryAction::Reset(ResetMode::Hard), entry),
                ),
        );
    let named: Vec<RefDecoration> = decorations
        .into_iter()
        .filter(|d| *d != RefDecoration::DetachedHead)
        .collect();
    if !named.is_empty() {
        menu = menu.separator();
        for decoration in named {
            let items = ref_items(view, &decoration, entry, &context);
            menu = menu.submenu(menu_title(&decoration), items);
        }
    }
    menu = menu.separator().item(
        "Compare with Working Tree",
        ask(
            view,
            HistoryAction::Compare(DiffScope::Snapshot {
                from: entry.sha.clone(),
                to: None,
            }),
            entry,
        ),
    );
    let is_base = compare_base.as_ref().is_some_and(|b| b.sha == entry.sha);
    if let (Some(base), Some((base_at, entry_at))) = (&compare_base, entries_order) {
        if !is_base {
            // Older → newer, by position in the (newest-first) list.
            let (from, to) = if base_at.unwrap_or(0) > entry_at.unwrap_or(0) {
                (base.sha.clone(), entry.sha.clone())
            } else {
                (entry.sha.clone(), base.sha.clone())
            };
            menu = menu.item(
                format!("Compare with {} · {}", base.short_sha(), base.subject),
                ask(
                    view,
                    HistoryAction::Compare(DiffScope::Range { from, to }),
                    entry,
                ),
            );
        }
    }
    let selected = entry.clone();
    menu.item(
        if is_base {
            "Clear Compare Selection"
        } else {
            "Select for Compare"
        },
        local(view, move |view, cx| {
            view.compare_base = if is_base {
                None
            } else {
                Some(selected.clone())
            };
            cx.notify();
        }),
    )
    .separator()
    .item("Copy SHA", copy(entry.sha.clone()))
    .item("Copy Subject", copy(entry.subject.clone()))
}

/// The actions for one branch or tag on a commit (`RefMenuItems`).
pub(super) fn ref_items(
    view: &Entity<HistoryView>,
    decoration: &RefDecoration,
    entry: &LogEntry,
    context: &HistoryContext,
) -> MenuSpec {
    let current = context.branch.clone().unwrap_or_else(|| "HEAD".into());
    let detached = context.branch.is_none();
    let menu = MenuSpec::new();
    match decoration {
        RefDecoration::Tag(name) => {
            let mut menu = menu;
            if let Some(remote) = &context.tag_remote {
                menu = menu.item(
                    format!("Push to {remote}"),
                    ask(view, HistoryAction::PushTag(name.clone()), entry),
                );
            }
            menu = menu
                .item(
                    format!("Merge into {current}"),
                    ask(view, HistoryAction::Merge(name.clone()), entry),
                )
                .item("Copy Name", copy(name.clone()))
                .separator()
                .item(
                    "Delete Tag",
                    ask(view, HistoryAction::DeleteTag(name.clone()), entry),
                );
            if let Some(remote) = &context.tag_remote {
                menu = menu.item(
                    format!("Delete from {remote}…"),
                    ask(view, HistoryAction::DeleteRemoteTag(name.clone()), entry),
                );
            }
            menu
        }
        RefDecoration::LocalBranch(name) => menu
            .item(
                format!("Switch to {name}"),
                ask(view, HistoryAction::SwitchToBranch(name.clone()), entry),
            )
            .item(
                format!("Merge into {current}"),
                ask(view, HistoryAction::Merge(name.clone()), entry),
            )
            .item_enabled(
                format!("Rebase {current} onto {name}"),
                !detached,
                ask(view, HistoryAction::RebaseOnto(name.clone()), entry),
            )
            .item("Copy Name", copy(name.clone()))
            .separator()
            .item(
                "Delete Branch…",
                ask(view, HistoryAction::DeleteBranch(name.clone()), entry),
            ),
        RefDecoration::Head { branch } => menu.item("Copy Name", copy(branch.clone())),
        RefDecoration::RemoteBranch { branch, .. } => {
            let full = decoration.label();
            let mut menu = menu;
            if branch != "HEAD" {
                menu = menu
                    .item(
                        format!("Check Out {branch}"),
                        ask(view, HistoryAction::SwitchToBranch(branch.clone()), entry),
                    )
                    .item(
                        format!("Merge into {current}"),
                        ask(view, HistoryAction::Merge(full.clone()), entry),
                    )
                    .item_enabled(
                        format!("Rebase {current} onto {full}"),
                        !detached,
                        ask(view, HistoryAction::RebaseOnto(full.clone()), entry),
                    );
            }
            menu.item("Copy Name", copy(full))
        }
        RefDecoration::DetachedHead => menu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> LogEntry {
        LogEntry {
            sha: "abc1234def".into(),
            parents: vec![],
            author: "A".into(),
            email: "a@example.com".into(),
            date: std::time::SystemTime::UNIX_EPOCH,
            subject: "Subject".into(),
            refs: vec![],
        }
    }

    #[test]
    fn merges_take_the_branch_on_the_commit() {
        let remotes = vec!["origin".to_string()];
        let parse = |refs: &[&str]| -> Vec<RefDecoration> {
            refs.iter()
                .map(|r| RefDecoration::parse(r, &remotes))
                .collect()
        };
        assert_eq!(
            merge_target(&parse(&["HEAD -> main", "feature"]), &entry()),
            "feature"
        );
        assert_eq!(
            merge_target(&parse(&["origin/HEAD", "origin/main"]), &entry()),
            "origin/main"
        );
        assert_eq!(merge_target(&parse(&["tag: v1.0"]), &entry()), "v1.0");
        assert_eq!(
            merge_target(&parse(&["HEAD -> main"]), &entry()),
            "abc1234def"
        );
        assert_eq!(menu_title(&RefDecoration::Tag("v1".into())), "Tag v1");
        assert_eq!(
            menu_title(&RefDecoration::RemoteBranch {
                remote: "origin".into(),
                branch: "main".into()
            }),
            "Remote Branch origin/main"
        );
    }
}
