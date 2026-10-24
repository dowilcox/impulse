//! `impulse review`, `impulse comments` and `impulse problems` (and the MCP
//! tools of the same names), as the Swift `ControlCommands` and
//! `AgentWorkspaceCommands` answer them: Review opened for the caller's
//! repository; the user's review comments, each saying whether its lines
//! still read the same, or one marked addressed; and the language
//! servers' problems in the caller's workspace. They read the caller's
//! workspace, never start a server or run a project command, and answer
//! with text plus the same as JSON data.

use std::collections::HashMap;
use std::path::Path;

use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use impulse_git::types::DiffScope;
use impulse_kit::review_comments::{CommentStore, Resolution};
use impulse_protocol::agent_tools::{self, CommentEntry, CommentList, ProblemList};
use impulse_protocol::command::{Command, Comments, ReviewScope};
use impulse_protocol::control::{ControlRequest, ControlResponse};

use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::{Kind, Surface};

type Reply = async_channel::Sender<ControlResponse>;

fn inside_terminal() -> ControlResponse {
    ControlResponse::failure("Run this inside an Impulse terminal.")
}

fn data_dir() -> std::path::PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions))
}

/// `path` under `root` without it; anything else as given.
fn relative(path: &str, root: &str) -> String {
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// The repository's comments as agents see them (`path` keeps one file's,
/// `current_only` drops outdated ones, `only` keeps one comment), each
/// checked against its file's text now.
fn comment_list(
    store: &CommentStore,
    root: &str,
    path: Option<&str>,
    current_only: bool,
    only: Option<&str>,
) -> CommentList {
    let path = path.map(|p| relative(p, root));
    let mut comments: Vec<agent_tools::ReviewComment> = store
        .comments
        .iter()
        .filter(|c| only.is_none_or(|id| c.id == id))
        .filter(|c| path.as_deref().is_none_or(|p| c.path == p))
        // The same JSON on both sides.
        .filter_map(|c| serde_json::to_value(c).ok())
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    comments.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    let mut texts: HashMap<String, Option<String>> = HashMap::new();
    let entries = comments
        .iter()
        .map(|comment| {
            let text = texts.entry(comment.path.clone()).or_insert_with(|| {
                impulse_kit::text_file::read(&Path::new(root).join(&comment.path)).map(|c| c.text)
            });
            CommentEntry::new(comment, comment.freshness(text.as_deref()))
        })
        .filter(|entry| !current_only || entry.outdated != Some(true))
        .collect();
    CommentList {
        repository: Path::new(root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        comments: entries,
    }
}

/// Every open Review (and History's) of `root`, in every window, shows its
/// comments as saved now.
fn reload_reviews(root: &str, cx: &mut App) {
    for (workbench, _) in crate::session::workbenches(cx) {
        let reviews: Vec<_> = workbench
            .read(cx)
            .workspaces
            .iter()
            .flat_map(|w| w.all_surfaces())
            .filter_map(|s| match s {
                Surface::Review(r) => Some(r.clone()),
                Surface::History(h) => Some(h.read(cx).review().clone()),
                _ => None,
            })
            .collect();
        for review in reviews {
            if review.read(cx).root == root {
                review.update(cx, |review, cx| review.reload_comments(cx));
            }
        }
    }
}

impl Workbench {
    /// Answer `review`, `comments` or `problems` for the pane `terminal`
    /// (`from_shell`: the caller is its shell or started from it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_review_control(
        &mut self,
        request: &ControlRequest,
        terminal: Option<Entity<TerminalView>>,
        cwd: Option<String>,
        from_shell: bool,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = move |response: ControlResponse| {
            let _ = reply.try_send(response);
        };
        let json = request.arguments.contains_key("json");
        match request.parsed() {
            Command::Review { scope, background } => {
                if scope == ReviewScope::LastTurn {
                    // `agent_hooks` answers this one first.
                    return answer(ControlResponse::failure(
                        "No agent turns recorded yet in this repository.",
                    ));
                }
                let Some(cwd) = cwd else {
                    return answer(inside_terminal());
                };
                let Some(root) = impulse_git::branches::repo_root(&cwd)
                    .map(|r| r.trim_end_matches('/').to_string())
                else {
                    return answer(ControlResponse::failure("Not in a git repository."));
                };
                let scope = match scope {
                    ReviewScope::Staged => DiffScope::Staged,
                    ReviewScope::Unstaged => DiffScope::Unstaged,
                    _ => DiffScope::Uncommitted,
                };
                if !background {
                    window.activate_window();
                }
                if let Some(terminal) = terminal {
                    self.reveal_surface(&Surface::Terminal(terminal), cx);
                }
                self.open_review_of(root, Some(scope), window, cx);
                answer(ControlResponse::ok());
            }
            Command::Comments(comments) => {
                // They read the caller's workspace: without a pane, there's
                // none.
                let (Some(_), Some(cwd)) = (terminal.as_ref(), cwd) else {
                    return answer(inside_terminal());
                };
                // Marking one addressed changes what's saved, so only the
                // pane's own shell, or a program it started, may.
                if matches!(comments, Comments::Resolve { .. }) && !from_shell {
                    return answer(ControlResponse::failure(
                        "impulse comments only works from the terminal's own shell, not inside tmux or screen.",
                    ));
                }
                // The agent's name, once the Rust app tells agents apart.
                let by = "An agent".to_string();
                let task = cx.background_spawn(async move {
                    let Some(root) = impulse_git::branches::repo_root(&cwd)
                        .map(|r| r.trim_end_matches('/').to_string())
                    else {
                        return (ControlResponse::failure("Not in a git repository."), None);
                    };
                    let mut store = CommentStore::open(&data_dir(), &root);
                    let list = match comments {
                        Comments::Resolve { id, note } => {
                            if note.is_empty() {
                                return (
                                    ControlResponse::failure("Say what you did in the note."),
                                    None,
                                );
                            }
                            if !store.resolve(&id, Some(Resolution::now(by, note))) {
                                return (
                                    ControlResponse::failure(format!(
                                        "No review comment {id} in this repository."
                                    )),
                                    None,
                                );
                            }
                            let list = comment_list(&store, &root, None, false, Some(&id));
                            return (
                                ControlResponse::answer(&list, list.text(), json),
                                Some(root),
                            );
                        }
                        Comments::List { path, current_only } => {
                            comment_list(&store, &root, path.as_deref(), current_only, None)
                        }
                    };
                    (ControlResponse::answer(&list, list.text(), json), None)
                });
                cx.spawn(async move |_, cx| {
                    let (response, changed) = task.await;
                    if let Some(root) = changed {
                        cx.update(|cx| reload_reviews(&root, cx));
                    }
                    answer(response);
                })
                .detach();
            }
            Command::Problems { path, minimum } => {
                let Some(terminal) = terminal else {
                    return answer(inside_terminal());
                };
                let root = self.workspace_root_of(&terminal, cwd, cx);
                let all: Vec<agent_tools::Problem> = self
                    .problems
                    .read(cx)
                    .all()
                    .into_iter()
                    .map(|p| agent_tools::Problem {
                        path: p.path,
                        line: p.line as i64,
                        column: p.column as i64,
                        severity: agent_tools::Severity::from_label(p.severity.label())
                            .unwrap_or(agent_tools::Severity::Info),
                        message: p.message,
                        source: p.source,
                        code: p.code,
                    })
                    .collect();
                let inside = format!("{root}/");
                let checked: Vec<String> = self
                    .checked_files(cx)
                    .into_iter()
                    .filter(|f| *f == root || f.starts_with(&inside))
                    .collect();
                let name = Path::new(&root)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let note = if !self.trust.is_trusted(&root) {
                    Some(format!(
                        "Language servers don't run in {name} because the user hasn't trusted it, so there are no diagnostics."
                    ))
                } else if checked.is_empty() {
                    Some(
                        "No language server has checked a file in this workspace yet: servers check the files open in Impulse's editor (and some, the whole project)."
                            .to_string(),
                    )
                } else {
                    None
                };
                let list = ProblemList::new(&all, &root, path.as_deref(), minimum, &checked, note);
                answer(ControlResponse::answer(&list, list.text(), json));
            }
            _ => answer(ControlResponse::failure(format!(
                "impulse: unknown command '{}'",
                request.command
            ))),
        }
    }

    /// The caller's workspace folder (a folder workspace's root), else its
    /// repository's, else its own folder.
    fn workspace_root_of(
        &self,
        terminal: &Entity<TerminalView>,
        cwd: Option<String>,
        cx: &App,
    ) -> String {
        let workspace = self.workspaces.iter().find(|w| {
            w.all_surfaces()
                .filter_map(Surface::terminal)
                .any(|t| t == terminal)
        });
        if let Some(workspace) = workspace.filter(|w| w.kind == Kind::Folder) {
            return workspace.root.to_string_lossy().into_owned();
        }
        let folder = cwd.unwrap_or_else(|| terminal.read(cx).cwd.clone());
        impulse_git::branches::repo_root(&folder)
            .map(|r| r.trim_end_matches('/').to_string())
            .unwrap_or(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use impulse_kit::review_comments::{ReviewComment, Side};

    /// A fresh folder under the system's temporary one.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "impulse-control-review-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn comments_list_with_their_freshness() {
        let data = temp_dir("data");
        let repo = temp_dir("repo");
        let root = repo.to_string_lossy().into_owned();
        std::fs::write(repo.join("a.rs"), "fn a() {}\nlet x = 2;\n").unwrap();
        let mut store = CommentStore::open(&data, &root);
        let current = ReviewComment::new("a.rs", Side::New, 1, 1, "fn a() {}", "Doc it.");
        let stale = ReviewComment::new("a.rs", Side::New, 2, 2, "let x = 1;", "Why 1?");
        let removed = ReviewComment::new("b.rs", Side::Old, 3, 3, "gone", "Keep it.");
        let (current_id, stale_id) = (current.id.clone(), stale.id.clone());
        store.add(stale);
        store.add(removed);
        store.add(current);

        let list = comment_list(&store, &root, None, false, None);
        assert_eq!(list.repository, repo.file_name().unwrap().to_string_lossy());
        let ids: Vec<&str> = list.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids[..2], [current_id.as_str(), stale_id.as_str()]);
        assert_eq!(list.comments[0].outdated, Some(false));
        assert_eq!(list.comments[1].outdated, Some(true));
        assert_eq!(list.comments[2].outdated, None);

        let current_only = comment_list(&store, &root, None, true, None);
        assert_eq!(current_only.comments.len(), 2);
        let absolute = format!("{root}/a.rs");
        assert_eq!(
            comment_list(&store, &root, Some(&absolute), false, None)
                .comments
                .len(),
            2
        );
        let one = comment_list(&store, &root, None, false, Some(&stale_id));
        assert_eq!(one.comments.len(), 1);
        let _ = std::fs::remove_dir_all(data);
        let _ = std::fs::remove_dir_all(repo);
    }
}
