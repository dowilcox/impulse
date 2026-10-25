//! Safety snapshots: before any destructive action (discard, reset, branch
//! switch over local changes, stash drop, ...) Impulse records the working
//! tree, untracked files that aren't ignored included, and the index as a
//! commit under a private ref, so the action can be undone. The same
//! mechanism records agent-turn checkpoints.
//!
//! The refs live outside refs/heads and refs/tags, so a normal `git push`
//! never sends them (`git push --mirror` would). They are pruned by age and
//! count. The real index is never written: the trees are built in copies of
//! it, so a snapshot can't take `index.lock` from the user's or an agent's
//! `git add` running at the same moment.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cli::{CliError, CliOutput};
use crate::operations::{
    environment, git, git_in, git_with, literal, Call, GitResult, OperationError,
};
use crate::raw::unique_id;

pub const OPLOG_PREFIX: &str = "refs/impulse/oplog/";
pub const CHECKPOINT_PREFIX: &str = "refs/impulse/checkpoints/";
/// "Reviewed up to here" marks (the review's "Since my last review").
pub const REVIEW_PREFIX: &str = "refs/impulse/reviews/";
/// How long `prune` keeps snapshots by default.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);
/// The tree of an empty index.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetySnapshot {
    /// The full ref name, e.g. refs/impulse/oplog/1767225600123-discard.
    pub ref_name: String,
    /// The snapshot commit (its tree is the working tree at the time).
    pub commit: String,
    /// The tree of the index at the time (`None` when it had conflicts).
    pub index_tree: Option<String>,
    pub date: SystemTime,
    /// What was about to happen ("Discard changes to a.txt").
    pub reason: String,
}

/// A fixed identity: snapshots are internal and must work even when the user
/// hasn't configured git's user.name and user.email.
fn identity(mut env: HashMap<String, String>) -> HashMap<String, String> {
    for (key, value) in [
        ("GIT_AUTHOR_NAME", "Impulse"),
        ("GIT_AUTHOR_EMAIL", "snapshots@impulse.invalid"),
        ("GIT_COMMITTER_NAME", "Impulse"),
        ("GIT_COMMITTER_EMAIL", "snapshots@impulse.invalid"),
    ] {
        env.insert(key.into(), value.into());
    }
    env
}

/// `base` with `GIT_INDEX_FILE` set to `index`.
fn with_index(base: &HashMap<String, String>, index: &str) -> HashMap<String, String> {
    let mut env = base.clone();
    env.insert("GIT_INDEX_FILE".into(), index.into());
    env
}

/// The environment for recording `root`'s working tree: in a folder that
/// isn't trusted, git runs none of the repository's own hooks, fsmonitor
/// or filters (`restricted_environment`), as for every other read Impulse
/// makes there on its own.
fn recording_environment(root: &str, trusted: bool) -> GitResult<HashMap<String, String>> {
    if trusted {
        return Ok(environment());
    }
    crate::cli::restricted_environment(root, &environment())
        .ok_or_else(|| OperationError::Invalid("Couldn't read the repository's settings.".into()))
}

/// A copy of the real index at `copy`, removed when dropped.
struct TempIndex(String);

impl TempIndex {
    fn copy(git_dir: &str, name: &str) -> TempIndex {
        let path = Path::new(git_dir).join(name).to_string_lossy().into_owned();
        let real = Path::new(git_dir).join("index");
        if real.exists() {
            let _ = std::fs::copy(&real, &path);
        }
        TempIndex(path)
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn absolute_git_dir(root: &str) -> Option<String> {
    git(&["rev-parse", "--absolute-git-dir"], root)
        .ok()
        .map(|r| r.stdout.trim().to_string())
}

fn head_commit(root: &str) -> Option<String> {
    let result = git(&["rev-parse", "--verify", "-q", "HEAD"], root).ok()?;
    Some(result.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

fn trimmed(result: Result<CliOutput, CliError>) -> Option<String> {
    result.ok().map(|r| r.stdout.trim().to_string())
}

/// Record the working tree and index under `prefix` (`OPLOG_PREFIX` by
/// default), before something the user asked for.
pub fn create(reason: &str, root: &str, prefix: &str) -> GitResult<SafetySnapshot> {
    create_with(reason, root, prefix, true)
}

/// `create`, for a snapshot Impulse takes on its own (an agent's turn, a
/// checkpoint an agent asked for): restricted when `root` isn't trusted.
pub fn create_with(
    reason: &str,
    root: &str,
    prefix: &str,
    trusted: bool,
) -> GitResult<SafetySnapshot> {
    let Some(git_dir) = absolute_git_dir(root) else {
        return Err(OperationError::Invalid("Not a git repository".into()));
    };
    let base = recording_environment(root, trusted)?;

    // The working tree's tree, built in a temporary index.
    let temp_index = TempIndex::copy(&git_dir, &format!("impulse-snapshot-{}.index", unique_id()));
    let env = with_index(&base, &temp_index.0);
    git_in(&["add", "--all", "--", "."], root, &env, 120)?;
    let Some(worktree_tree) = trimmed(git_in(&["write-tree"], root, &env, 120)) else {
        return Err(OperationError::Invalid(
            "Couldn't record the working tree.".into(),
        ));
    };

    // The real index's tree, from a copy too. (It fails with unresolved
    // conflicts; then there's no index tree.)
    let index_copy = TempIndex::copy(
        &git_dir,
        &format!("impulse-snapshot-{}.index-tree", unique_id()),
    );
    let index_tree = trimmed(git_in(
        &["write-tree"],
        root,
        &with_index(&base, &index_copy.0),
        60,
    ));

    let mut message = format!("impulse snapshot: {reason}\n");
    if let Some(index_tree) = &index_tree {
        message.push_str(&format!("\nindex-tree {index_tree}\n"));
    }
    let mut args = vec![
        "commit-tree".to_string(),
        worktree_tree,
        "--no-gpg-sign".into(),
        "-F".into(),
        "-".into(),
    ];
    if let Some(head) = head_commit(root) {
        args.extend(["-p".into(), head]);
    }
    let created = crate::cli::run(
        &args,
        root,
        crate::cli::RunOptions {
            stdin: Some(message.as_bytes()),
            environment: Some(&identity(environment())),
            timeout: Some(Duration::from_secs(60)),
            on_output_line: None,
        },
    )?;
    let commit = created.stdout.trim().to_string();

    let now = SystemTime::now();
    let millis = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let ref_name = format!("{prefix}{millis}-{}", slug(reason));
    git(&["update-ref", &ref_name, &commit], root)?;
    Ok(SafetySnapshot {
        ref_name,
        commit,
        index_tree,
        date: now,
        reason: reason.into(),
    })
}

/// A commit of the working tree as it is (uncommitted and untracked files
/// that aren't ignored included) on top of HEAD, with no ref pointing at it,
/// for comparing work that isn't committed (git's gc removes it in time).
/// HEAD itself when nothing is uncommitted; `None` when it can't be made.
pub fn working_tree_commit(root: &str, trusted: bool) -> Option<String> {
    let git_dir = absolute_git_dir(root)?;
    let head = head_commit(root)?;
    let base = recording_environment(root, trusted).ok()?;
    let temp_index = TempIndex::copy(&git_dir, &format!("impulse-compare-{}.index", unique_id()));
    let env = with_index(&base, &temp_index.0);
    git_in(&["add", "--all", "--", "."], root, &env, 120).ok()?;
    let tree = trimmed(git_in(&["write-tree"], root, &env, 60))?;
    if git(&["rev-parse", "HEAD^{tree}"], root).is_ok_and(|r| r.stdout.trim() == tree) {
        return Some(head);
    }
    trimmed(git_in(
        &[
            "commit-tree",
            &tree,
            "--no-gpg-sign",
            "-p",
            &head,
            "-m",
            "impulse: working tree",
        ],
        root,
        &identity(environment()),
        60,
    ))
}

/// Snapshots under `prefix` (nested folders included, e.g. one per terminal
/// under the checkpoint prefix), newest first.
pub fn list(root: &str, prefix: &str) -> Vec<SafetySnapshot> {
    let Ok(result) = git_with(
        &[
            "for-each-ref",
            "--sort=-refname",
            "--format=%(refname)%00%(objectname)%00%(contents:subject)%00%(contents:body)%1e",
            prefix,
        ],
        root,
        Call {
            timeout: Duration::from_secs(30),
            ..Default::default()
        },
    ) else {
        return Vec::new();
    };
    let mut snapshots: Vec<SafetySnapshot> = result
        .stdout
        .split('\u{1e}')
        .filter_map(|chunk| {
            let fields: Vec<&str> = chunk.trim_matches(['\n', '\r']).split('\0').collect();
            if fields.len() < 4 {
                return None;
            }
            let subject = fields[2];
            let reason = subject
                .strip_prefix("impulse snapshot: ")
                .unwrap_or(subject);
            let index_tree = fields[3]
                .split('\n')
                .find_map(|line| line.strip_prefix("index-tree "))
                .map(str::to_string);
            Some(SafetySnapshot {
                ref_name: fields[0].into(),
                commit: fields[1].into(),
                index_tree,
                date: date_of_ref(fields[0]),
                reason: reason.into(),
            })
        })
        .collect();
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.date));
    snapshots
}

/// Put `paths` (empty: everything) back as they were in `snapshot`, in the
/// working tree and, when recorded, the index. Files that aren't in the
/// snapshot (created after it) are left alone.
pub fn restore<S: AsRef<str>>(snapshot: &SafetySnapshot, paths: &[S], root: &str) -> GitResult {
    let pathspec: Vec<String> = if paths.is_empty() {
        vec![".".into()]
    } else {
        paths.iter().map(|p| p.as_ref().to_string()).collect()
    };
    // Overlay: files that aren't in the snapshot stay as they are.
    let source = format!("--source={}", snapshot.commit);
    let mut args = vec!["restore", "--overlay", source.as_str(), "--worktree", "--"];
    args.extend(pathspec.iter().map(String::as_str));
    git(&literal(&args), root)?;
    // The empty tree means the index was empty: nothing to put back (and
    // git rejects a pathspec that matches nothing).
    if let Some(index_tree) = snapshot.index_tree.as_deref().filter(|t| *t != EMPTY_TREE) {
        // Only paths either index knows: an untracked file brought back above
        // matches neither, and git would reject the whole command.
        let staged: Vec<String> = if paths.is_empty() {
            pathspec
        } else {
            pathspec
                .into_iter()
                .filter(|p| index_has(p, index_tree, root))
                .collect()
        };
        if staged.is_empty() {
            return Ok(());
        }
        let source = format!("--source={index_tree}");
        let mut args = vec!["restore", "--overlay", source.as_str(), "--staged", "--"];
        args.extend(staged.iter().map(String::as_str));
        git(&literal(&args), root)?;
    }
    Ok(())
}

/// `path` is in the saved index tree or the current index.
fn index_has(path: &str, tree: &str, root: &str) -> bool {
    git(
        &literal(&["ls-tree", "-r", "--name-only", tree, "--", path]),
        root,
    )
    .is_ok_and(|r| !r.stdout.is_empty())
        || git(&literal(&["ls-files", "--", path]), root).is_ok_and(|r| !r.stdout.is_empty())
}

/// Delete snapshots beyond the newest `keep` or older than `max_age`, in one
/// ref transaction (thousands of old refs take one git process, not one
/// each). Returns the refs deleted.
pub fn prune(root: &str, prefix: &str, keep: usize, max_age: Duration) -> Vec<String> {
    let Ok(result) = git_with(
        &["for-each-ref", "--format=%(refname)", prefix],
        root,
        Call {
            timeout: Duration::from_secs(30),
            ..Default::default()
        },
    ) else {
        return Vec::new();
    };
    let cutoff = SystemTime::now().checked_sub(max_age).unwrap_or(UNIX_EPOCH);
    let mut refs: Vec<(String, SystemTime)> = result
        .stdout
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|r| (r.to_string(), date_of_ref(r)))
        .collect();
    refs.sort_by_key(|(_, date)| std::cmp::Reverse(*date));
    let doomed: Vec<String> = refs
        .into_iter()
        .enumerate()
        .filter(|(offset, (_, date))| *offset >= keep || *date < cutoff)
        .map(|(_, (r, _))| r)
        .collect();
    if doomed.is_empty() {
        return Vec::new();
    }
    let commands: String = doomed.iter().map(|r| format!("delete {r}\n")).collect();
    let deleted = git_with(
        &["update-ref", "--stdin"],
        root,
        Call {
            stdin: Some(commands.as_bytes()),
            timeout: Duration::from_secs(60),
            ..Default::default()
        },
    );
    if deleted.is_err() {
        return Vec::new();
    }
    doomed
}

/// A snapshot's time, from the `<millis>-<reason>` last path component.
pub fn date_of_ref(reference: &str) -> SystemTime {
    let leaf = reference.rsplit('/').next().unwrap_or("");
    let millis: u64 = leaf
        .split('-')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// The reason as a ref-safe word: lowercase ASCII letters and digits, a dash
/// for each run of anything else, at most 32 characters.
pub fn slug(reason: &str) -> String {
    let mut out = String::new();
    for c in reason.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "snapshot".into()
    } else {
        trimmed.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_dates() {
        assert_eq!(slug("Discard changes to a.txt"), "discard-changes-to-a-txt");
        assert_eq!(slug("  !!"), "snapshot");
        assert_eq!(slug("Ünïcode reason"), "n-code-reason");
        assert_eq!(slug(&"x".repeat(50)).len(), 32);
        assert_eq!(
            date_of_ref("refs/impulse/oplog/1767225600123-discard"),
            UNIX_EPOCH + Duration::from_millis(1_767_225_600_123)
        );
        assert_eq!(date_of_ref("refs/impulse/oplog/nonsense"), UNIX_EPOCH);
    }
}
