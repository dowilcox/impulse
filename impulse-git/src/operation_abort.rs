//! Aborting a merge, cherry-pick or revert so it always works and can be
//! undone (the Swift `OperationAbort`). `git merge --abort` (and the others)
//! refuse once a conflicted file has been edited ("Entry ... not uptodate.
//! Cannot merge."), which leaves finishing the operation or `git reset
//! --hard` as the only ways out. Here a snapshot is taken first; when git
//! refuses, Impulse resets to HEAD itself and puts back the uncommitted work
//! from before the operation.
//!
//! Telling that work apart from edits made while resolving needs a record
//! taken when the operation started (`record_start_if_needed`, kept in
//! `<git dir>/impulse-operation.json`): the files that were dirty then and
//! that the incoming side doesn't change. Git won't start an operation whose
//! changes would overwrite uncommitted work, so those files are exactly the
//! work from before, untouched by the operation.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::operations::{self, git, literal, GitResult, OperationAction, OperationError};
use crate::safety::{self, SafetySnapshot};
use crate::types::RepoOperation;

/// The work from before an operation, recorded when it started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRecord {
    /// HEAD and the incoming commit (MERGE_HEAD, CHERRY_PICK_HEAD or
    /// REVERT_HEAD) when it started; the record only applies to that
    /// operation.
    pub head: String,
    pub incoming: String,
    /// The files with uncommitted work from before.
    pub paths: Vec<String>,
    /// A snapshot holding them; `None` when there were none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// git's own `--abort` worked.
    Aborted,
    /// git refused; Impulse reset to HEAD and put back `kept`. `exact` when a
    /// start record told the work from before apart from later edits.
    Reset { kept: Vec<String>, exact: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aborted {
    pub outcome: Outcome,
    /// Everything as it was before aborting, for Undo.
    pub undo: Option<SafetySnapshot>,
    /// The incoming commit, for reopening a merge on Undo.
    pub incoming: Option<String>,
    /// What went wrong once the reset had begun (the work from before
    /// couldn't all be put back). Undo still brings everything back.
    pub problem: Option<OperationError>,
}

/// The operations this applies to (a rebase aborts reliably already).
pub fn applies(operation: &RepoOperation) -> bool {
    matches!(
        operation,
        RepoOperation::Merge | RepoOperation::CherryPick | RepoOperation::Revert
    )
}

/// How new an operation's head file has to be for its start to be recorded.
pub const DEFAULT_RECORD_AGE: Duration = Duration::from_secs(60);

/// Record the work from before the operation now in progress at `root`,
/// unless it's recorded already. Only while the operation is new (its head
/// file is under `max_age` old): later, edits made while resolving could
/// pass for earlier work.
pub fn record_start_if_needed(root: &str, max_age: Duration) {
    let (Some(git_dir), Some((name, incoming)), Some(head)) = (
        git_directory(root),
        incoming_head(root),
        rev_parse("HEAD", root),
    ) else {
        return;
    };
    if load_record(&git_dir).is_some_and(|r| r.head == head && r.incoming == incoming) {
        return;
    }
    // A head file from the future (a planted one) is as stale as an old one;
    // two seconds allow for clock granularity.
    let Some(modified) = std::fs::metadata(Path::new(&git_dir).join(&name))
        .and_then(|m| m.modified())
        .ok()
    else {
        return;
    };
    let now = SystemTime::now();
    let fresh = match now.duration_since(modified) {
        Ok(age) => age <= max_age,
        Err(ahead) => ahead.duration() <= Duration::from_secs(2),
    };
    if !fresh {
        return;
    }
    let paths = work_from_before(root, &name, &incoming);
    let mut snapshot = None;
    if !paths.is_empty() {
        let Ok(created) = safety::create(&format!("before {name}"), root, safety::OPLOG_PREFIX)
        else {
            return;
        };
        snapshot = Some(created.commit);
    }
    save_record(
        &StartRecord {
            head,
            incoming,
            paths,
            snapshot,
        },
        &git_dir,
    );
}

/// Abort `operation` at `root`: a snapshot of everything first (for Undo),
/// then git's `--abort`, then, if git refuses, a hard reset to HEAD with the
/// work from before the operation put back.
pub fn abort(operation: &RepoOperation, root: &str) -> GitResult<Aborted> {
    let git_dir = git_directory(root);
    let incoming = incoming_head(root);
    let undo = safety::create(
        &format!("abort {}", operation.title().to_lowercase()),
        root,
        safety::OPLOG_PREFIX,
    )
    .ok();
    let plain = operations::perform(OperationAction::Abort, operation, root);
    if plain.is_ok() {
        if let Some(git_dir) = &git_dir {
            remove_record(git_dir);
        }
        return Ok(Aborted {
            outcome: Outcome::Aborted,
            undo,
            incoming: incoming.map(|(_, commit)| commit),
            problem: None,
        });
    }
    // Without a snapshot to undo with, don't go further than git would.
    let (Some(undo), Some((name, incoming)), Some(head)) =
        (undo, incoming, rev_parse("HEAD", root))
    else {
        return Err(plain
            .err()
            .unwrap_or_else(|| OperationError::Invalid("Couldn't abort.".into())));
    };

    let record = git_dir
        .as_deref()
        .and_then(load_record)
        .filter(|r| r.head == head && r.incoming == incoming);
    let (kept, source, exact) = match record {
        Some(record) => (
            record.paths,
            record.snapshot.unwrap_or_else(|| undo.commit.clone()),
            true,
        ),
        None => (
            work_from_before(root, &name, &incoming),
            undo.commit.clone(),
            false,
        ),
    };

    // From here on the files have changed: whatever fails, the result
    // carries the undo snapshot, so nothing is lost.
    let mut aborted = Aborted {
        outcome: Outcome::Reset {
            kept: kept.clone(),
            exact,
        },
        undo: Some(undo),
        incoming: Some(incoming),
        problem: None,
    };
    if let Err(error) = git(&["reset", "--hard", "-q", "HEAD"], root) {
        aborted.problem = Some(error);
        return Ok(aborted);
    }
    // A cherry-pick or revert of several commits keeps a sequencer.
    match operation {
        RepoOperation::CherryPick => {
            let _ = git(&["cherry-pick", "--quit"], root);
        }
        RepoOperation::Revert => {
            let _ = git(&["revert", "--quit"], root);
        }
        _ => {}
    }
    if !kept.is_empty() {
        let source = format!("--source={source}");
        let mut args = vec!["restore", "--overlay", source.as_str(), "--worktree", "--"];
        args.extend(kept.iter().map(String::as_str));
        if let Err(error) = git(&literal(&args), root) {
            aborted.problem = Some(error);
        }
    }
    if let Some(git_dir) = &git_dir {
        remove_record(git_dir);
    }
    Ok(aborted)
}

/// Undo an abort: reopen a merge (its conflicts come back) and put every
/// file back as it was when Abort was chosen. A cherry-pick or revert isn't
/// reopened; its files are.
pub fn undo(result: &Aborted, operation: &RepoOperation, root: &str) -> GitResult {
    let Some(undo) = &result.undo else {
        return Err(OperationError::Invalid(
            "There's nothing to undo with.".into(),
        ));
    };
    if let (RepoOperation::Merge, Some(incoming)) = (operation, &result.incoming) {
        // Conflicts are expected: they're what was being resolved.
        let _ = git(&["merge", "--no-ff", "--no-commit", incoming], root);
    }
    safety::restore(undo, &[] as &[&str], root)
}

// MARK: Helpers

/// The files with uncommitted work that the incoming side doesn't change.
fn work_from_before(root: &str, name: &str, incoming: &str) -> Vec<String> {
    // -z: otherwise git quotes names with non-ASCII characters
    // ("caf\303\251.txt"), which then match no file.
    let Ok(dirty) = git(&["diff", "--name-only", "-z", "--no-renames", "HEAD"], root) else {
        return Vec::new();
    };
    let changed: HashSet<String> = if name == "MERGE_HEAD" {
        let Some(base) = merge_base("HEAD", incoming, root) else {
            return Vec::new();
        };
        let Ok(incoming_changes) = git(
            &["diff", "--name-only", "-z", "--no-renames", &base, incoming],
            root,
        ) else {
            return Vec::new();
        };
        paths(&incoming_changes.stdout).into_iter().collect()
    } else {
        let Ok(commit_changes) = git(
            &[
                "diff-tree",
                "-r",
                "-z",
                "--root",
                "--no-commit-id",
                "--name-only",
                "--no-renames",
                incoming,
            ],
            root,
        ) else {
            return Vec::new();
        };
        paths(&commit_changes.stdout).into_iter().collect()
    };
    paths(&dirty.stdout)
        .into_iter()
        .filter(|p| !changed.contains(p))
        .collect()
}

/// The operation's head file and the commit in it.
pub fn incoming_head(root: &str) -> Option<(String, String)> {
    ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"]
        .into_iter()
        .find_map(|name| rev_parse(name, root).map(|commit| (name.to_string(), commit)))
}

pub fn record_path(git_dir: &str) -> String {
    Path::new(git_dir)
        .join("impulse-operation.json")
        .to_string_lossy()
        .into_owned()
}

pub fn load_record(git_dir: &str) -> Option<StartRecord> {
    serde_json::from_slice(&std::fs::read(record_path(git_dir)).ok()?).ok()
}

fn save_record(record: &StartRecord, git_dir: &str) {
    let Ok(data) = serde_json::to_vec(record) else {
        return;
    };
    // Atomically: written beside it, then moved into place.
    let path = record_path(git_dir);
    let temp = format!("{path}.{}", crate::raw::unique_id());
    if std::fs::write(&temp, data).is_ok() && std::fs::rename(&temp, &path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

fn remove_record(git_dir: &str) {
    let _ = std::fs::remove_file(record_path(git_dir));
}

/// The worktree's own git folder (where MERGE_HEAD is), absolute.
fn git_directory(root: &str) -> Option<String> {
    let result = git(&["rev-parse", "--absolute-git-dir"], root).ok()?;
    Some(result.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

fn rev_parse(name: &str, root: &str) -> Option<String> {
    let result = git(&["rev-parse", "-q", "--verify", name], root).ok()?;
    Some(result.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

fn merge_base(a: &str, b: &str, root: &str) -> Option<String> {
    let result = git(&["merge-base", a, b], root).ok()?;
    result
        .stdout
        .split('\n')
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// A NUL-separated path list (`-z`).
fn paths(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}
