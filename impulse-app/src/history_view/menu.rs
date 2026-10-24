//! History's menus (the Swift `HistoryRowView.menu` and `RefMenuItems`): a
//! commit's, and one branch or tag's, both on its chip and as a submenu of
//! its commit's.

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{App, Context, Entity, Window};
use impulse_git::log::LogEntry;
use impulse_git::operations::ResetMode;
use impulse_git::types::DiffScope;
use impulse_kit::git::refs::RefDecoration;

use super::{HistoryAction, HistoryContext, HistoryEvent, HistoryView};

/// An item that asks the workbench for `action` on `entry`.
fn ask(
    view: &Entity<HistoryView>,
    label: impl Into<gpui_kit::SharedString>,
    action: HistoryAction,
    entry: &LogEntry,
) -> PopupMenuItem {
    let (view, entry) = (view.clone(), entry.clone());
    PopupMenuItem::new(label).on_click(move |_, _, cx| {
        let (action, entry) = (action.clone(), entry.clone());
        view.update(cx, |_, cx| cx.emit(HistoryEvent { action, entry }))
    })
}

/// An item that runs `act` on the view.
fn local(
    view: &Entity<HistoryView>,
    label: impl Into<gpui_kit::SharedString>,
    act: impl Fn(&mut HistoryView, &mut Context<HistoryView>) + 'static,
) -> PopupMenuItem {
    let view = view.clone();
    PopupMenuItem::new(label).on_click(move |_, _, cx| view.update(cx, |view, cx| act(view, cx)))
}

fn copy(label: &'static str, text: String) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, _, cx: &mut App| HistoryView::copy(text.clone(), cx))
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

/// A commit's menu.
pub(super) fn commit_items(
    menu: PopupMenu,
    view: &Entity<HistoryView>,
    entry: &LogEntry,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
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
    let is_head = decorations.iter().any(|d| {
        matches!(
            d,
            RefDecoration::Head { .. } | RefDecoration::DetachedHead
        )
    });
    let target = merge_target(&decorations, entry);
    let mut menu = menu
        .item(ask(view, "Check Out (Detached)", HistoryAction::Checkout, entry))
        .item(ask(view, "Create Branch Here…", HistoryAction::BranchHere, entry))
        .item(ask(view, "Create Tag Here…", HistoryAction::TagHere, entry))
        .separator()
        .item(
            ask(view, format!("Merge into {current}"), HistoryAction::Merge(target.clone()), entry)
                .disabled(is_head),
        )
        .item(
            ask(
                view,
                format!("Rebase {current} onto Here"),
                HistoryAction::RebaseOnto(target),
                entry,
            )
            .disabled(is_head || context.branch.is_none()),
        )
        .item(ask(view, "Cherry-Pick", HistoryAction::CherryPick, entry))
        .item(ask(view, "Revert", HistoryAction::Revert, entry));
    {
        let (view, entry) = (view.clone(), entry.clone());
        menu = menu.submenu(
            format!("Reset {current} Here"),
            window,
            cx,
            move |menu, _, _| {
                menu.item(ask(
                    &view,
                    "Soft (keep changes staged)",
                    HistoryAction::Reset(ResetMode::Soft),
                    &entry,
                ))
                .item(ask(
                    &view,
                    "Mixed (keep changes)",
                    HistoryAction::Reset(ResetMode::Mixed),
                    &entry,
                ))
                .item(ask(
                    &view,
                    "Hard (discard changes)…",
                    HistoryAction::Reset(ResetMode::Hard),
                    &entry,
                ))
            },
        );
    }
    let named: Vec<RefDecoration> = decorations
        .into_iter()
        .filter(|d| *d != RefDecoration::DetachedHead)
        .collect();
    if !named.is_empty() {
        menu = menu.separator();
        for decoration in named {
            let (view, entry, context) = (view.clone(), entry.clone(), context.clone());
            menu = menu.submenu(
                menu_title(&decoration),
                window,
                cx,
                move |menu, _, _| ref_items(menu, &view, &decoration, &entry, &context),
            );
        }
    }
    menu = menu.separator().item(ask(
        view,
        "Compare with Working Tree",
        HistoryAction::Compare(DiffScope::Snapshot {
            from: entry.sha.clone(),
            to: None,
        }),
        entry,
    ));
    let is_base = compare_base.as_ref().is_some_and(|b| b.sha == entry.sha);
    if let (Some(base), Some((base_at, entry_at))) = (&compare_base, entries_order) {
        if !is_base {
            // Older → newer, by position in the (newest-first) list.
            let (from, to) = if base_at.unwrap_or(0) > entry_at.unwrap_or(0) {
                (base.sha.clone(), entry.sha.clone())
            } else {
                (entry.sha.clone(), base.sha.clone())
            };
            menu = menu.item(ask(
                view,
                format!("Compare with {} · {}", base.short_sha(), base.subject),
                HistoryAction::Compare(DiffScope::Range { from, to }),
                entry,
            ));
        }
    }
    let selected = entry.clone();
    menu.item(local(
        view,
        if is_base {
            "Clear Compare Selection"
        } else {
            "Select for Compare"
        },
        move |view, cx| {
            view.compare_base = if is_base { None } else { Some(selected.clone()) };
            cx.notify();
        },
    ))
    .separator()
    .item(copy("Copy SHA", entry.sha.clone()))
    .item(copy("Copy Subject", entry.subject.clone()))
}

/// The actions for one branch or tag on a commit.
pub(super) fn ref_items(
    menu: PopupMenu,
    view: &Entity<HistoryView>,
    decoration: &RefDecoration,
    entry: &LogEntry,
    context: &HistoryContext,
) -> PopupMenu {
    let current = context.branch.clone().unwrap_or_else(|| "HEAD".into());
    let detached = context.branch.is_none();
    match decoration {
        RefDecoration::Tag(name) => {
            let mut menu = menu;
            if let Some(remote) = &context.tag_remote {
                menu = menu.item(ask(
                    view,
                    format!("Push to {remote}"),
                    HistoryAction::PushTag(name.clone()),
                    entry,
                ));
            }
            menu = menu
                .item(ask(
                    view,
                    format!("Merge into {current}"),
                    HistoryAction::Merge(name.clone()),
                    entry,
                ))
                .item(copy("Copy Name", name.clone()))
                .separator()
                .item(ask(
                    view,
                    "Delete Tag",
                    HistoryAction::DeleteTag(name.clone()),
                    entry,
                ));
            if let Some(remote) = &context.tag_remote {
                menu = menu.item(ask(
                    view,
                    format!("Delete from {remote}…"),
                    HistoryAction::DeleteRemoteTag(name.clone()),
                    entry,
                ));
            }
            menu
        }
        RefDecoration::LocalBranch(name) => menu
            .item(ask(
                view,
                format!("Switch to {name}"),
                HistoryAction::SwitchToBranch(name.clone()),
                entry,
            ))
            .item(ask(
                view,
                format!("Merge into {current}"),
                HistoryAction::Merge(name.clone()),
                entry,
            ))
            .item(
                ask(
                    view,
                    format!("Rebase {current} onto {name}"),
                    HistoryAction::RebaseOnto(name.clone()),
                    entry,
                )
                .disabled(detached),
            )
            .item(copy("Copy Name", name.clone()))
            .separator()
            .item(ask(
                view,
                "Delete Branch…",
                HistoryAction::DeleteBranch(name.clone()),
                entry,
            )),
        RefDecoration::Head { branch } => menu.item(copy("Copy Name", branch.clone())),
        RefDecoration::RemoteBranch { branch, .. } => {
            let full = decoration.label();
            let mut menu = menu;
            if branch != "HEAD" {
                menu = menu
                    .item(ask(
                        view,
                        format!("Check Out {branch}"),
                        HistoryAction::SwitchToBranch(branch.clone()),
                        entry,
                    ))
                    .item(ask(
                        view,
                        format!("Merge into {current}"),
                        HistoryAction::Merge(full.clone()),
                        entry,
                    ))
                    .item(
                        ask(
                            view,
                            format!("Rebase {current} onto {full}"),
                            HistoryAction::RebaseOnto(full.clone()),
                            entry,
                        )
                        .disabled(detached),
                    );
            }
            menu.item(copy("Copy Name", full))
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
            refs.iter().map(|r| RefDecoration::parse(r, &remotes)).collect()
        };
        assert_eq!(merge_target(&parse(&["HEAD -> main", "feature"]), &entry()), "feature");
        assert_eq!(merge_target(&parse(&["origin/HEAD", "origin/main"]), &entry()), "origin/main");
        assert_eq!(merge_target(&parse(&["tag: v1.0"]), &entry()), "v1.0");
        assert_eq!(merge_target(&parse(&["HEAD -> main"]), &entry()), "abc1234def");
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
