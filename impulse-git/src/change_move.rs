//! Moving uncommitted work from one checkout into another on the same
//! commit (the main checkout's changes into a new task; the Swift
//! `ChangeMove`). The work is saved in a safety snapshot first; the target
//! gets the files as they were, unstaged (deleted files deleted); then the
//! source puts those files back to HEAD. Untracked files that didn't move,
//! and ignored files, stay put, and so do folders holding a repository of
//! their own, and files edited while the move ran.

use std::collections::HashSet;
use std::path::Path;

use crate::operations::{self, environment, git, git_in, literal, GitResult, OperationError};
use crate::raw::unique_id;
use crate::safety::{self, SafetySnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// The source's working tree and index before the move, for Undo.
    pub snapshot: SafetySnapshot,
    /// Files that were changed or added, and files that were deleted: the
    /// target has all of them.
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    /// Of those, files edited while the move ran: the target has them as
    /// they were before, and the source kept the edits.
    pub edited_during: Vec<String>,
    /// Folders with a git repository of their own (a clone, a submodule). A
    /// snapshot records one only as a commit id, so they can't move: they
    /// stayed in the source.
    pub nested_repositories: Vec<String>,
}

impl Moved {
    pub fn count(&self) -> usize {
        self.changed.len() + self.deleted.len()
    }
}

/// One file's difference between two commits.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    /// A, M, D or T.
    status: String,
    path: String,
    /// A gitlink (mode 160000) on either side: a folder holding its own
    /// repository.
    is_repository: bool,
}

/// Move `paths` (`None`: every uncommitted file) from the checkout at
/// `source` to the one at `target`, which has to be on the same commit. A
/// renamed file needs both its old and new path.
pub fn move_changes(paths: Option<&[&str]>, source: &str, target: &str) -> GitResult<Moved> {
    let head = match head(source) {
        Some(head) if Some(&head) == self::head(target).as_ref() => head,
        _ => {
            return Err(OperationError::Invalid(
                "The new task isn't on the same commit as the files' checkout.".into(),
            ))
        }
    };
    let snapshot = safety::create("move changes to a new task", source, safety::OPLOG_PREFIX)?;
    let mut changes = differences(&head, &snapshot.commit, source);
    if let Some(paths) = paths {
        let chosen: HashSet<&str> = paths.iter().copied().collect();
        changes.retain(|c| chosen.contains(c.path.as_str()));
    }
    // Moving one would leave an empty folder in the target, and removing it
    // from the source would delete its files and history for good.
    let nested: Vec<String> = changes
        .iter()
        .filter(|c| c.is_repository)
        .map(|c| c.path.clone())
        .collect();
    changes.retain(|c| !c.is_repository);
    if changes.is_empty() {
        if !nested.is_empty() {
            return Err(OperationError::Invalid(format!(
                "Only folders with a git repository of their own changed ({}), and those can't move.",
                naming(&nested)
            )));
        }
        return Err(OperationError::Invalid(
            "There are no uncommitted changes to move.".into(),
        ));
    }
    let changed: Vec<String> = changes
        .iter()
        .filter(|c| c.status != "D")
        .map(|c| c.path.clone())
        .collect();
    let deleted: Vec<String> = changes
        .iter()
        .filter(|c| c.status == "D")
        .map(|c| c.path.clone())
        .collect();

    // Into the task, unstaged.
    if !changed.is_empty() {
        let source_arg = format!("--source={}", snapshot.commit);
        let mut args = vec![
            "restore",
            "--overlay",
            source_arg.as_str(),
            "--worktree",
            "--",
        ];
        args.extend(changed.iter().map(String::as_str));
        git(&literal(&args), target)?;
    }
    for path in &deleted {
        remove_file(&Path::new(target).join(path));
    }

    // Out of the source: tracked files back to HEAD, new files removed. Only
    // files still as the snapshot has them: an edit made since (by an agent,
    // or the editor saving) stays.
    let edited: HashSet<String> = edited_since(&snapshot, &changed, &deleted, source)
        .into_iter()
        .collect();
    let added: Vec<&str> = changes
        .iter()
        .filter(|c| c.status == "A" && !edited.contains(&c.path))
        .map(|c| c.path.as_str())
        .collect();
    let modified: Vec<&str> = changes
        .iter()
        .filter(|c| c.status != "A" && !edited.contains(&c.path))
        .map(|c| c.path.as_str())
        .collect();
    if !modified.is_empty() {
        operations::discard_all(&modified, source)?;
    }
    if !added.is_empty() {
        let mut args = vec!["rm", "-q", "--cached", "--ignore-unmatch", "--"];
        args.extend(added.iter().copied());
        let _ = git(&literal(&args), source);
        for path in &added {
            remove(path, source);
        }
    }
    let edited_during = changes
        .iter()
        .map(|c| c.path.clone())
        .filter(|p| edited.contains(p))
        .collect();
    Ok(Moved {
        snapshot,
        changed,
        deleted,
        edited_during,
        nested_repositories: nested,
    })
}

/// Put moved files back in `source` as they were before the move (the
/// target is left alone). Files edited during the move were never taken
/// out, so they're left as they are. So is a file changed in `source` since
/// the move (it no longer matches HEAD, as the move left it): those are
/// returned.
pub fn undo(moved: &Moved, source: &str) -> GitResult<Vec<String>> {
    let edited: HashSet<&String> = moved.edited_during.iter().collect();
    let candidates: Vec<&str> = moved
        .changed
        .iter()
        .chain(&moved.deleted)
        .filter(|p| !edited.contains(p))
        .map(String::as_str)
        .collect();
    let Some(since) = changed_from_head(&candidates, source) else {
        return Err(OperationError::Invalid(
            "Couldn't tell which files changed since the move, so nothing was put back.".into(),
        ));
    };
    let left: HashSet<&str> = edited
        .iter()
        .map(|p| p.as_str())
        .chain(since.iter().map(String::as_str))
        .collect();
    let changed: Vec<&str> = moved
        .changed
        .iter()
        .map(String::as_str)
        .filter(|p| !left.contains(p))
        .collect();
    let deleted: Vec<&str> = moved
        .deleted
        .iter()
        .map(String::as_str)
        .filter(|p| !left.contains(p))
        .collect();
    if !changed.is_empty() {
        safety::restore(&moved.snapshot, &changed, source)?;
    }
    if deleted.is_empty() {
        return Ok(since);
    }
    // A deletion that was staged is staged again.
    let staged: Vec<&str> = deleted
        .iter()
        .copied()
        .filter(|path| {
            let Some(tree) = moved.snapshot.index_tree.as_deref() else {
                return false;
            };
            git(
                &literal(&["ls-tree", "--name-only", tree, "--", path]),
                source,
            )
            .is_ok_and(|r| r.stdout.is_empty())
        })
        .collect();
    if !staged.is_empty() {
        let mut args = vec!["rm", "-q", "--cached", "--ignore-unmatch", "--"];
        args.extend(staged.iter().copied());
        let _ = git(&literal(&args), source);
    }
    for path in &deleted {
        remove_file(&Path::new(source).join(path));
    }
    Ok(since)
}

/// Of `paths`, those that differ from HEAD in `root`: staged, edited,
/// deleted or created. `None` when git can't tell.
fn changed_from_head(paths: &[&str], root: &str) -> Option<Vec<String>> {
    if paths.is_empty() {
        return Some(Vec::new());
    }
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--no-renames",
        "--untracked-files=all",
        "--ignore-submodules=all",
        "--",
    ];
    args.extend(paths.iter().copied());
    let result = git(&literal(&args), root).ok()?;
    // "XY path" each.
    Some(
        result
            .stdout
            .split('\0')
            .filter(|entry| entry.len() > 3)
            .map(|entry| entry[3..].to_string())
            .collect(),
    )
}

/// Files that differ between two commits, renames as a deletion and an
/// addition.
fn differences(a: &str, b: &str, root: &str) -> Vec<Change> {
    // ":100644 100644 <id> <id> M\0path\0" per file.
    let Ok(result) = git(&["diff", "--raw", "-z", "--no-renames", a, b], root) else {
        return Vec::new();
    };
    let fields: Vec<&str> = result
        .stdout
        .split('\0')
        .filter(|f| !f.is_empty())
        .collect();
    let mut list = Vec::new();
    for pair in fields.chunks_exact(2) {
        let meta: Vec<&str> = pair[0].split(' ').filter(|m| !m.is_empty()).collect();
        if meta.len() >= 5 {
            list.push(Change {
                status: meta[4].chars().take(1).collect(),
                path: pair[1].to_string(),
                is_repository: meta[0] == ":160000" || meta[1] == "160000",
            });
        }
    }
    list
}

/// Of the files a move took (`changed` and `deleted`), those that no longer
/// match `snapshot` in `root`: edited, deleted or created again since it was
/// taken. All of them when git can't tell.
pub fn edited_since(
    snapshot: &SafetySnapshot,
    changed: &[String],
    deleted: &[String],
    root: &str,
) -> Vec<String> {
    let mut edited: Vec<String> = deleted
        .iter()
        .filter(|p| std::fs::symlink_metadata(Path::new(root).join(p)).is_ok())
        .cloned()
        .collect();
    if changed.is_empty() {
        return edited;
    }
    let everything = || {
        changed
            .iter()
            .cloned()
            .chain(edited.iter().cloned())
            .collect::<Vec<_>>()
    };
    // The files as they are now, recorded the way the snapshot recorded them
    // (in a temporary index, so the real one is untouched), compared with it.
    let Ok(dir) = git(&["rev-parse", "--absolute-git-dir"], root) else {
        return everything();
    };
    let temp_index = Path::new(dir.stdout.trim())
        .join(format!("impulse-move-{}.index", unique_id()))
        .to_string_lossy()
        .into_owned();
    let mut env = environment();
    env.insert("GIT_INDEX_FILE".into(), temp_index.clone());
    let run = |args: Vec<String>| git_in(&args, root, &env, 120);
    let mut add = vec!["add", "--all", "--"];
    add.extend(changed.iter().map(String::as_str));
    let mut compare = vec![
        "diff-index",
        "--cached",
        "--name-only",
        "-z",
        "--no-renames",
        snapshot.commit.as_str(),
        "--",
    ];
    compare.extend(changed.iter().map(String::as_str));
    let diff = run(vec!["read-tree".into(), snapshot.commit.clone()])
        .and_then(|_| run(literal(&add)))
        .and_then(|_| run(literal(&compare)));
    let _ = std::fs::remove_file(&temp_index);
    match diff {
        Ok(diff) => {
            edited.extend(
                diff.stdout
                    .split('\0')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string),
            );
            edited
        }
        Err(_) => everything(),
    }
}

fn head(root: &str) -> Option<String> {
    let result = git(&["rev-parse", "--verify", "-q", "HEAD"], root).ok()?;
    Some(result.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

/// Delete a file, then any folders it leaves empty (not `root` itself).
fn remove(path: &str, root: &str) {
    let mut target = Path::new(root).join(path);
    if !remove_file(&target) {
        return;
    }
    let root_length = root.len();
    while let Some(parent) = target.parent().map(Path::to_path_buf) {
        // remove_dir only takes an empty folder: nothing that appears
        // meanwhile goes.
        if parent.as_os_str().len() <= root_length || std::fs::remove_dir(&parent).is_err() {
            return;
        }
        target = parent;
    }
}

/// Delete a file or symbolic link, never a folder (one where a file was holds
/// what no snapshot recorded). False when nothing was deleted.
pub fn remove_file(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_dir() => std::fs::remove_file(path).is_ok(),
        _ => false,
    }
}

/// "vendor/lib", or "vendor/lib and 2 more".
fn naming(paths: &[String]) -> String {
    match paths {
        [] => String::new(),
        [only] => only.clone(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}
