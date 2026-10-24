//! Mutating git operations, and the reads around them that the git CLI
//! answers best (the Swift `GitOperations`). All of them run the user's
//! `git`, so they behave exactly as in a terminal: hooks, signing, filters,
//! credentials.
//!
//! File paths go to git as literal pathspecs (`--literal-pathspecs`): a name
//! like `app/[id].tsx` or `:!x` mustn't act as a glob or pathspec magic and
//! touch other files. Branch names and revisions go after
//! `--end-of-options`, so a ref like `--orphan=x` can't become an option.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, SystemTime};

use crate::cli::{self, CliError, CliOutput, RunOptions};
use crate::patch::{self, PatchSelection};
use crate::raw::unix_time;
use crate::safety::{self, SafetySnapshot};
use crate::scoped_diff::{self, DiffOptions, PatchTextError};
use crate::snapshot::current_operation;
use crate::types::{is_valid_ref_name, DiffScope, RepoOperation};

/// A git operation that failed: a classified CLI failure, or a precondition
/// Impulse checked itself (the hunk changed since it was shown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationError {
    Cli(CliError),
    Stale(String),
    Invalid(String),
}

impl OperationError {
    pub fn message(&self) -> String {
        match self {
            OperationError::Cli(error) => error.message(),
            OperationError::Stale(detail) | OperationError::Invalid(detail) => detail.clone(),
        }
    }

    /// Raw git output, when there is any.
    pub fn output(&self) -> Option<&str> {
        match self {
            OperationError::Cli(error) => Some(&error.output),
            _ => None,
        }
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for OperationError {}

impl From<CliError> for OperationError {
    fn from(error: CliError) -> Self {
        OperationError::Cli(error)
    }
}

pub type GitResult<T = ()> = Result<T, OperationError>;

/// Called with each progress line git prints (fetch, pull, push).
pub type Progress<'a> = Option<&'a mut dyn FnMut(&str)>;

// MARK: Running git

static ENVIRONMENT: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Extra environment for every git command (tests pin identity and config).
pub fn environment() -> HashMap<String, String> {
    ENVIRONMENT.read().unwrap().clone()
}

pub fn set_environment(environment: HashMap<String, String>) {
    *ENVIRONMENT.write().unwrap() = environment;
}

/// How to run one command; the default limit only ends a command that hangs.
/// Local commands can legitimately take long (an LFS smudge during a switch,
/// a slow hook), and stopping one partway leaves a half-updated working tree.
pub(crate) struct Call<'a> {
    pub stdin: Option<&'a [u8]>,
    pub timeout: Duration,
    pub on_line: Progress<'a>,
}

impl Default for Call<'_> {
    fn default() -> Self {
        Call {
            stdin: None,
            timeout: Duration::from_secs(1800),
            on_line: None,
        }
    }
}

pub(crate) fn git<S: AsRef<str>>(arguments: &[S], root: &str) -> GitResult<CliOutput> {
    git_with(arguments, root, Call::default())
}

pub(crate) fn git_with<S: AsRef<str>>(
    arguments: &[S],
    root: &str,
    call: Call,
) -> GitResult<CliOutput> {
    let environment = environment();
    Ok(cli::run(
        arguments,
        root,
        RunOptions {
            stdin: call.stdin,
            environment: Some(&environment),
            timeout: Some(call.timeout),
            on_output_line: call.on_line.map(|f| f as &mut dyn FnMut(&str)),
        },
    )?)
}

/// `git` with `environment` in place of `environment()`.
pub(crate) fn git_in(
    arguments: &[impl AsRef<str>],
    root: &str,
    environment: &HashMap<String, String>,
    timeout: u64,
) -> Result<CliOutput, CliError> {
    cli::run(
        arguments,
        root,
        RunOptions {
            environment: Some(environment),
            timeout: Some(Duration::from_secs(timeout)),
            ..Default::default()
        },
    )
}

fn done(result: GitResult<CliOutput>) -> GitResult {
    result.map(|_| ())
}

fn trimmed(result: &CliOutput) -> String {
    result.stdout.trim().to_string()
}

/// `--literal-pathspecs` and then `arguments`.
pub(crate) fn literal<S: AsRef<str>>(arguments: &[S]) -> Vec<String> {
    std::iter::once("--literal-pathspecs")
        .chain(arguments.iter().map(AsRef::as_ref))
        .map(str::to_string)
        .collect()
}

fn strings<S: AsRef<str>>(head: &[&str], tail: &[S]) -> Vec<String> {
    head.iter()
        .copied()
        .chain(tail.iter().map(AsRef::as_ref))
        .map(str::to_string)
        .collect()
}

fn split_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_nul(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

// MARK: Staging whole files

/// `git add` the paths (new, modified, or deleted).
pub fn stage<S: AsRef<str>>(paths: &[S], root: &str) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    done(git(
        &literal(&strings(&["add", "--all", "--"], paths)),
        root,
    ))
}

/// Stage every change, untracked files included.
pub fn stage_all(root: &str) -> GitResult {
    done(git(&["add", "--all"], root))
}

/// Unstage the paths, keeping their working-tree changes. Works before the
/// first commit too.
pub fn unstage<S: AsRef<str>>(paths: &[S], root: &str, is_unborn: bool) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    if is_unborn {
        return done(git(
            &literal(&strings(&["rm", "--cached", "-r", "--quiet", "--"], paths)),
            root,
        ));
    }
    done(git(
        &literal(&strings(&["restore", "--staged", "--"], paths)),
        root,
    ))
}

pub fn unstage_all(root: &str, is_unborn: bool) -> GitResult {
    if is_unborn {
        return done(git(&["rm", "--cached", "-r", "--quiet", "."], root));
    }
    done(git(&["reset", "--quiet"], root))
}

/// Revert tracked files' working-tree changes to the index version.
/// Untracked files are left alone (the app moves those to the Trash).
pub fn discard_working_tree<S: AsRef<str>>(paths: &[S], root: &str) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    done(git(
        &literal(&strings(&["restore", "--worktree", "--"], paths)),
        root,
    ))
}

/// Revert files to HEAD in both the index and the working tree.
pub fn discard_all<S: AsRef<str>>(paths: &[S], root: &str) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    done(git(
        &literal(&strings(
            &["restore", "--staged", "--worktree", "--source=HEAD", "--"],
            paths,
        )),
        root,
    ))
}

// MARK: Hunk and line selections

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchTarget {
    /// Stage selected changes from the unstaged diff into the index.
    Stage,
    /// Take selected changes out of the index (the staged diff).
    Unstage,
    /// Revert selected changes in the working tree (the unstaged diff).
    Discard,
}

impl PatchTarget {
    fn source_scope(self) -> DiffScope {
        if self == PatchTarget::Unstage {
            DiffScope::Staged
        } else {
            DiffScope::Unstaged
        }
    }

    fn reverse(self) -> bool {
        self != PatchTarget::Stage
    }

    /// `context_lines` is what the patch was built with: without any, `git
    /// apply` needs `--unidiff-zero` or it takes every hunk to sit at the
    /// start or end of the file.
    fn apply_arguments(self, context_lines: i32) -> Vec<&'static str> {
        let mut arguments = match self {
            PatchTarget::Stage => vec!["apply", "--cached"],
            PatchTarget::Unstage => vec!["apply", "--cached", "--reverse"],
            PatchTarget::Discard => vec!["apply", "--reverse"],
        };
        arguments.extend(["--recount", "--whitespace=nowarn"]);
        if context_lines <= 0 {
            arguments.push("--unidiff-zero");
        }
        arguments.push("-");
        arguments
    }
}

/// Why a file whose diff was cut short (`FileDiff::truncated`) can't be
/// acted on by hunk or line.
pub const TRUNCATED_DIFF_MESSAGE: &str = "This file's diff is too long to show in full, so its hunks and lines can't be staged, unstaged or reverted on their own. Use the whole file instead.";

/// Stage, unstage or discard a selection of one file's hunks and lines.
/// `expected_hunk_ids` (hunk index to the id from the `FileDiff` the user
/// saw) guards against acting on a hunk that changed in the meantime.
pub fn apply(
    target: PatchTarget,
    selection: &PatchSelection,
    path: &str,
    old_path: Option<&str>,
    expected_hunk_ids: &HashMap<usize, String>,
    options: &DiffOptions,
    root: &str,
) -> GitResult {
    let scope = target.source_scope();
    if !expected_hunk_ids.is_empty() {
        let Ok(current) = scoped_diff::file_diff(root, path, old_path, &scope, options) else {
            return Err(OperationError::Stale(
                "The file's diff couldn't be read. Refresh and try again.".into(),
            ));
        };
        // A diff cut short ends partway through a hunk (with the whole file
        // as context, its only one): the hunk would carry changes nobody saw.
        if current.truncated {
            return Err(OperationError::Invalid(TRUNCATED_DIFF_MESSAGE.into()));
        }
        for (index, id) in expected_hunk_ids {
            if current.hunk_ids.get(*index) != Some(id) {
                return Err(OperationError::Stale(
                    "The file changed since it was shown. Refresh and try again.".into(),
                ));
            }
        }
    }
    let text = match scoped_diff::patch_text(root, path, old_path, &scope, options) {
        Ok(Some(text)) => text,
        Ok(None) => {
            return Err(OperationError::Stale(format!(
                "There are no changes left in {path}."
            )))
        }
        Err(PatchTextError::Build(error)) => {
            return Err(OperationError::Invalid(error.to_string()))
        }
        Err(PatchTextError::Git(error)) => return Err(OperationError::Invalid(error.to_string())),
    };
    let patch = patch::build(&text, selection, target.reverse())
        .map_err(|e| OperationError::Invalid(e.to_string()))?;
    done(git_with(
        &target.apply_arguments(options.context_lines),
        root,
        Call {
            stdin: Some(patch.as_bytes()),
            ..Default::default()
        },
    ))
}

// MARK: Commit

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub amend: bool,
    pub sign_off: bool,
    pub no_verify: bool,
    pub allow_empty: bool,
    /// Commit only what is staged (the default); when true, `--all` stages
    /// tracked changes first.
    pub include_unstaged_tracked: bool,
}

/// `git commit -F -` with the message on stdin. Returns the new HEAD id.
pub fn commit(message: &str, options: &CommitOptions, root: &str) -> GitResult<String> {
    let trimmed_message = message.trim();
    if trimmed_message.is_empty() && !options.amend {
        return Err(OperationError::Invalid(
            "Write a commit message first.".into(),
        ));
    }
    // `whitespace`, git's own default for -F: lines starting with "#"
    // ("#482 was...", Markdown headings) are message, not comments.
    let mut args = vec!["commit", "--quiet", "--cleanup=whitespace"];
    let reads_message = !(trimmed_message.is_empty() && options.amend);
    if reads_message {
        args.extend(["-F", "-"]);
    } else {
        args.push("--no-edit");
    }
    if options.amend {
        args.push("--amend");
    }
    if options.sign_off {
        args.push("--signoff");
    }
    if options.no_verify {
        args.push("--no-verify");
    }
    if options.allow_empty {
        args.push("--allow-empty");
    }
    if options.include_unstaged_tracked {
        args.push("--all");
    }
    let stdin = format!("{trimmed_message}\n");
    git_with(
        &args,
        root,
        Call {
            stdin: reads_message.then_some(stdin.as_bytes()),
            timeout: Duration::from_secs(600),
            ..Default::default()
        },
    )?;
    Ok(trimmed(&git(&["rev-parse", "HEAD"], root)?))
}

/// Undo the last commit, keeping its changes staged. With `expected_head`,
/// only while HEAD is still that commit: one made since (an agent's) isn't
/// the one to take back.
pub fn uncommit(expected_head: Option<&str>, root: &str) -> GitResult {
    let Some(expected_head) = expected_head else {
        return done(git(&["reset", "--soft", "HEAD~1"], root));
    };
    let short = &expected_head[..expected_head.len().min(7)];
    let moved = || {
        OperationError::Stale(format!(
            "The branch has moved on from {short}, so nothing was undone."
        ))
    };
    if resolve_ref("HEAD", root).as_deref() != Some(expected_head) {
        return Err(moved());
    }
    // What `reset --soft HEAD~1` refuses, refused alike.
    let unmerged = git(&["ls-files", "-u"], root).is_ok_and(|r| !r.stdout.is_empty());
    if resolve_ref("MERGE_HEAD", root).is_some() || unmerged {
        return Err(OperationError::Invalid(
            "A commit can't be undone in the middle of a merge.".into(),
        ));
    }
    let Some(parent) = resolve_ref(&format!("{expected_head}^"), root) else {
        return Err(OperationError::Invalid(format!(
            "{short} is the first commit; there's none before it to go back to."
        )));
    };
    // `reset --soft HEAD~1` as one compare-and-swap: HEAD moves only while
    // it's still `expected_head` (the index and files stay as they are), and
    // ORIG_HEAD and the reflog say what reset would.
    let commands =
        format!("update HEAD {parent} {expected_head}\nupdate ORIG_HEAD {expected_head}\n");
    let updated = git_with(
        &["update-ref", "-m", "reset: moving to HEAD~1", "--stdin"],
        root,
        Call {
            stdin: Some(commands.as_bytes()),
            ..Default::default()
        },
    );
    if let Err(error) = updated {
        return Err(
            if resolve_ref("HEAD", root).as_deref() == Some(expected_head) {
                error
            } else {
                moved()
            },
        );
    }
    Ok(())
}

/// The HEAD commit's full message (for amend).
pub fn head_message(root: &str) -> Option<String> {
    git(&["log", "-1", "--format=%B"], root)
        .ok()
        .map(|r| trimmed(&r))
}

// MARK: Branches

pub fn switch_branch(name: &str, root: &str) -> GitResult {
    done(git(&["switch", "--end-of-options", name], root))
}

/// Create a branch at `start_point` (HEAD by default), switching to it when
/// `checkout`.
pub fn create_branch(
    name: &str,
    start_point: Option<&str>,
    checkout: bool,
    root: &str,
) -> GitResult {
    let mut args = if checkout {
        vec!["switch", "-c", name, "--end-of-options"]
    } else {
        vec!["branch", "--end-of-options", name]
    };
    args.extend(start_point);
    done(git(&args, root))
}

pub fn rename_branch(name: &str, new_name: &str, root: &str) -> GitResult {
    done(git(
        &["branch", "-m", "--end-of-options", name, new_name],
        root,
    ))
}

pub fn delete_branch(name: &str, force: bool, root: &str) -> GitResult {
    done(git(
        &[
            "branch",
            if force { "-D" } else { "-d" },
            "--end-of-options",
            name,
        ],
        root,
    ))
}

/// Delete branch `name` only while it still points at `commit`, in one step
/// (a compare-and-swap), so a commit made to it meanwhile isn't lost. Its
/// settings (the upstream) go too, as with `branch -D`.
pub fn delete_branch_at(name: &str, commit: &str, root: &str) -> GitResult {
    if worktrees(root)
        .iter()
        .any(|w| w.branch.as_deref() == Some(name))
    {
        return Err(OperationError::Invalid(format!(
            "{name} is checked out in a worktree."
        )));
    }
    let reference = format!("refs/heads/{name}");
    if let Err(error) = git(&["update-ref", "-d", &reference, commit], root) {
        return Err(
            if resolve_ref(&reference, root).as_deref() == Some(commit) {
                error
            } else {
                OperationError::Stale(format!("{name} has moved."))
            },
        );
    }
    let _ = git(
        &["config", "--remove-section", &format!("branch.{name}")],
        root,
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// The upstream branch was deleted on the remote.
    pub upstream_gone: bool,
    pub last_commit: SystemTime,
    pub subject: String,
    pub is_current: bool,
    /// Fully merged into the base branch (safe to delete).
    pub is_merged: bool,
}

/// Local branches with their tracking state, most recently committed first.
/// `base` decides "merged" (the default branch).
pub fn branch_details(root: &str, base: Option<&str>) -> Vec<BranchInfo> {
    let Ok(result) = git(
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)%1f%(upstream:short)%1f%(upstream:track)%1f%(committerdate:unix)%1f%(subject)%1f%(HEAD)",
            "refs/heads",
        ],
        root,
    ) else {
        return Vec::new();
    };
    let mut merged = HashSet::new();
    if let Some(base) = base {
        if let Ok(list) = git(
            &["branch", "--merged", base, "--format=%(refname:short)"],
            root,
        ) {
            merged = split_lines(&list.stdout).into_iter().collect();
        }
    }
    split_lines(&result.stdout)
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\u{1f}').collect();
            if fields.len() < 6 {
                return None;
            }
            let track = fields[2];
            let count = |label: &str| -> usize {
                let Some(at) = track.find(&format!("{label} ")) else {
                    return 0;
                };
                let digits: String = track[at + label.len() + 1..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                digits.parse().unwrap_or(0)
            };
            Some(BranchInfo {
                name: fields[0].into(),
                upstream: (!fields[1].is_empty()).then(|| fields[1].into()),
                ahead: count("ahead"),
                behind: count("behind"),
                upstream_gone: track.contains("gone"),
                last_commit: unix_time(fields[3].parse().unwrap_or(0.0)),
                subject: fields[4].into(),
                is_current: fields[5] == "*",
                is_merged: merged.contains(fields[0]) && Some(fields[0]) != base,
            })
        })
        .collect()
}

/// Local and remote-tracking branch names, most recently committed first.
pub fn branches(root: &str) -> (Vec<String>, Vec<String>) {
    let Ok(result) = git(
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
        root,
    ) else {
        return (Vec::new(), Vec::new());
    };
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in split_lines(&result.stdout) {
        if let Some(name) = line.strip_prefix("refs/heads/") {
            local.push(name.to_string());
        } else if let Some(name) = line.strip_prefix("refs/remotes/") {
            if !line.ends_with("/HEAD") {
                remote.push(name.to_string());
            }
        }
    }
    (local, remote)
}

// MARK: Stash

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub branch: Option<String>,
    /// The stash's commit: which stash this is, wherever it moves in the
    /// list.
    pub commit: String,
}

pub fn stash_list(root: &str) -> Vec<StashEntry> {
    let Ok(result) = git(&["stash", "list", "--format=%gd%x00%H%x00%gs"], root) else {
        return Vec::new();
    };
    split_lines(&result.stdout)
        .iter()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\0').collect();
            if parts.len() != 3 {
                return None;
            }
            let open = parts[0].find('{')?;
            let close = parts[0].find('}')?;
            let index = parts[0].get(open + 1..close)?.parse().ok()?;
            // "WIP on main: abc123 subject" / "On main: message"
            let branch = parts[2]
                .find(':')
                .and_then(|colon| parts[2][..colon].split(' ').rfind(|w| !w.is_empty()))
                .map(str::to_string);
            Some(StashEntry {
                index,
                message: parts[2].into(),
                branch,
                commit: parts[1].into(),
            })
        })
        .collect()
}

/// The stash the user picked, as the list has it now: the one at `commit`
/// wherever it moved to, or (without one) whatever is at `index`. `None`
/// when it's gone.
pub fn stash_entry(index: usize, commit: Option<&str>, root: &str) -> Option<StashEntry> {
    let list = stash_list(root);
    match commit.filter(|c| !c.is_empty()) {
        Some(commit) => list.into_iter().find(|e| e.commit == commit),
        None => list.into_iter().find(|e| e.index == index),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashOptions {
    pub message: Option<String>,
    pub include_untracked: bool,
    pub keep_index: bool,
    pub paths: Vec<String>,
}

impl Default for StashOptions {
    fn default() -> Self {
        StashOptions {
            message: None,
            include_untracked: true,
            keep_index: false,
            paths: Vec::new(),
        }
    }
}

pub fn stash(options: &StashOptions, root: &str) -> GitResult {
    let mut args = vec!["stash".to_string(), "push".to_string()];
    if let Some(message) = options.message.as_deref().filter(|m| !m.is_empty()) {
        args.extend(["-m".to_string(), message.to_string()]);
    }
    if options.include_untracked {
        args.push("--include-untracked".into());
    }
    if options.keep_index {
        args.push("--keep-index".into());
    }
    if !options.paths.is_empty() {
        args.push("--".into());
        args.extend(options.paths.iter().cloned());
    }
    done(git(&args, root))
}

/// Apply or pop a stash, restoring what was staged as staged. When the
/// staged part no longer applies ("Conflicts in index"), apply it all as
/// working-tree changes instead of failing.
pub fn stash_apply(index: usize, pop: bool, root: &str) -> GitResult {
    let command = if pop { "pop" } else { "apply" };
    let stash = format!("stash@{{{index}}}");
    let result = done(git(&["stash", command, "--index", &stash], root));
    if let Err(error) = &result {
        if error
            .output()
            .is_some_and(|o| o.contains("Try without --index"))
        {
            return done(git(&["stash", command, &stash], root));
        }
    }
    result
}

pub fn stash_drop(index: usize, root: &str) -> GitResult {
    done(git(&["stash", "drop", &format!("stash@{{{index}}}")], root))
}

/// The commit behind `stash@{index}` (kept so a drop or pop can be undone).
pub fn stash_commit(index: usize, root: &str) -> Option<String> {
    let result = git(
        &["rev-parse", "--verify", "-q", &format!("stash@{{{index}}}")],
        root,
    )
    .ok()?;
    Some(trimmed(&result)).filter(|s| !s.is_empty())
}

/// Put a dropped stash commit back on top of the stash list.
pub fn stash_store(commit: &str, message: &str, root: &str) -> GitResult {
    done(git(&["stash", "store", "-m", message, commit], root))
}

/// Undo `git stash pop`: put the stash's paths back the way `snapshot`
/// (taken just before the pop) had them, remove what the pop created, and
/// store the stash entry again. Other files are left alone.
pub fn undo_stash_pop(
    commit: &str,
    message: &str,
    snapshot: &SafetySnapshot,
    root: &str,
) -> GitResult {
    let paths = stash_paths(commit, root);
    let mut existed = HashSet::new();
    if !paths.is_empty() {
        if let Ok(result) = git(
            &strings(
                &["ls-tree", "-r", "-z", "--name-only", &snapshot.commit, "--"],
                &paths,
            ),
            root,
        ) {
            existed = split_nul(&result.stdout).into_iter().collect();
        }
    }
    for path in paths.iter().filter(|p| !existed.contains(*p)) {
        let _ = std::fs::remove_file(Path::new(root).join(path));
        let _ = git(
            &["rm", "--cached", "-q", "--ignore-unmatch", "--", path],
            root,
        );
    }
    if !existed.is_empty() {
        let kept: Vec<String> = paths
            .iter()
            .filter(|p| existed.contains(*p))
            .cloned()
            .collect();
        safety::restore(snapshot, &kept, root)?;
    }
    stash_store(commit, message, root)
}

/// Every path a stash touches: tracked changes plus untracked files.
pub fn stash_paths(commit: &str, root: &str) -> Vec<String> {
    let mut paths = git(
        &[
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-commit-id",
            &format!("{commit}^1"),
            commit,
        ],
        root,
    )
    .map(|r| split_nul(&r.stdout))
    .unwrap_or_default();
    paths.extend(stash_untracked_paths(commit, root));
    paths
}

/// The untracked files a stash carries (its third parent), relative to
/// `root`.
pub fn stash_untracked_paths(commit: &str, root: &str) -> Vec<String> {
    git(
        &["ls-tree", "-r", "-z", "--name-only", &format!("{commit}^3")],
        root,
    )
    .map(|r| split_nul(&r.stdout))
    .unwrap_or_default()
}

// MARK: Remote

/// Fetch the default remote, `remote`, or every remote, pruning deleted
/// remote branches. `timeout` is shorter for quiet background fetches.
pub fn fetch(
    all_remotes: bool,
    remote: Option<&str>,
    root: &str,
    timeout: Duration,
    on_progress: Progress,
) -> GitResult {
    let mut args = vec!["fetch", "--prune", "--progress"];
    if all_remotes {
        args.push("--all");
    } else if let Some(remote) = remote {
        args.extend(["--end-of-options", remote]);
    }
    done(git_with(
        &args,
        root,
        Call {
            timeout,
            on_line: on_progress,
            ..Default::default()
        },
    ))
}

/// How `pull` brings in upstream commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PullMode {
    /// Only when the branch hasn't diverged (never creates a merge commit).
    FastForwardOnly,
    /// Replay local commits on top of the upstream.
    Rebase,
    /// Merge the upstream in (a merge commit when they diverged).
    Merge,
}

impl PullMode {
    pub fn raw_value(self) -> &'static str {
        match self {
            PullMode::FastForwardOnly => "ff-only",
            PullMode::Rebase => "rebase",
            PullMode::Merge => "merge",
        }
    }
}

pub fn pull(mode: PullMode, root: &str, on_progress: Progress) -> GitResult {
    let flag = match mode {
        PullMode::FastForwardOnly => "--ff-only",
        PullMode::Rebase => "--rebase",
        PullMode::Merge => "--no-rebase",
    };
    done(git_with(
        &["pull", flag, "--no-edit", "--progress"],
        root,
        Call {
            timeout: Duration::from_secs(600),
            on_line: on_progress,
            ..Default::default()
        },
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOptions {
    /// Publish the branch to `remote`.
    pub set_upstream: bool,
    pub remote: String,
    pub branch: Option<String>,
    pub force_with_lease: bool,
    /// Also push annotated tags on the pushed commits.
    pub follow_tags: bool,
}

impl Default for PushOptions {
    fn default() -> Self {
        PushOptions {
            set_upstream: false,
            remote: "origin".into(),
            branch: None,
            force_with_lease: false,
            follow_tags: false,
        }
    }
}

/// Push the current branch.
pub fn push(options: &PushOptions, root: &str, on_progress: Progress) -> GitResult {
    let mut args = vec!["push", "--progress"];
    // --force-if-includes: the lease alone checks the remote-tracking ref,
    // which a background fetch keeps current; this also requires the
    // remote's tip to have been integrated locally.
    if options.force_with_lease {
        args.extend(["--force-with-lease", "--force-if-includes"]);
    }
    if options.follow_tags {
        args.push("--follow-tags");
    }
    if options.set_upstream {
        args.extend(["--set-upstream", &options.remote]);
        args.extend(options.branch.as_deref());
    }
    done(git_with(
        &args,
        root,
        Call {
            timeout: Duration::from_secs(600),
            on_line: on_progress,
            ..Default::default()
        },
    ))
}

/// The best common ancestor of two commits; `None` when they share none.
pub fn merge_base(a: &str, b: &str, root: &str) -> Option<String> {
    let result = git(&["merge-base", a, b], root).ok()?;
    split_lines(&result.stdout).into_iter().next()
}

/// The files that would conflict if `a` and `b` were merged, worked out in
/// memory (`git merge-tree`, git 2.38+) without touching any folder. Empty
/// when they merge cleanly; `None` when git can't tell.
pub fn predict_conflicts(a: &str, b: &str, root: &str) -> Option<Vec<String>> {
    match git(
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            a,
            b,
        ],
        root,
    ) {
        Ok(_) => Some(Vec::new()),
        // Exit 1: conflicts. The first line is the tree, then the files.
        Err(OperationError::Cli(failure)) if failure.status == 1 => {
            Some(split_lines(&failure.output).into_iter().skip(1).collect())
        }
        Err(_) => None,
    }
}

/// Whether `branch`'s work is in `base`: its commits are (a merge or a
/// fast-forward), or merging it would change nothing (a squash merge, whose
/// commits never look merged). Can miss a branch whose lines `base` has
/// since changed again. Unless `trusted`, the merge runs no driver or filter
/// the repository's configuration names (one that's needed counts as a
/// conflict, so the branch isn't merged).
pub fn is_merged(branch: &str, base: &str, root: &str, trusted: bool) -> bool {
    if git(&["merge-base", "--is-ancestor", branch, base], root).is_ok() {
        return true;
    }
    let env = if trusted {
        Some(environment())
    } else {
        cli::restricted_environment(root, &environment())
    };
    let Some(env) = env else {
        return false;
    };
    let Ok(merged) = git_in(
        &["merge-tree", "--write-tree", "--no-messages", base, branch],
        root,
        &env,
        600,
    ) else {
        return false;
    };
    let Ok(base_tree) = git(&["rev-parse", &format!("{base}^{{tree}}")], root) else {
        return false;
    };
    merged
        .stdout
        .split('\n')
        .find(|l| !l.is_empty())
        .unwrap_or("")
        == base_tree.stdout.trim()
}

/// Commits on `branch` that `base` lacks, and the reverse.
pub fn ahead_behind(branch: &str, base: &str, root: &str) -> Option<(usize, usize)> {
    let result = git(
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{branch}...{base}"),
        ],
        root,
    )
    .ok()?;
    let counts: Vec<usize> = result
        .stdout
        .split(['\t', ' ', '\n'])
        .filter_map(|n| n.parse().ok())
        .collect();
    (counts.len() == 2).then(|| (counts[0], counts[1]))
}

/// The commits on `branch` that `base` lacks, newest first, as (id,
/// subject): at most `limit`.
pub fn commits_ahead(branch: &str, base: &str, root: &str, limit: usize) -> Vec<(String, String)> {
    let Ok(result) = git(
        &[
            "log",
            "--format=%H%x09%s",
            "-n",
            &limit.to_string(),
            &format!("{base}..{branch}"),
            "--",
        ],
        root,
    ) else {
        return Vec::new();
    };
    split_lines(&result.stdout)
        .iter()
        .filter_map(|line| {
            let (id, subject) = line.split_once('\t').unwrap_or((line, ""));
            (!id.is_empty()).then(|| (id.to_string(), subject.to_string()))
        })
        .collect()
}

/// HEAD's latest move, from its reflog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadMove {
    pub time: SystemTime,
    /// How it moved ("pull: Fast-forward").
    pub subject: String,
    pub from: String,
    pub to: String,
}

/// HEAD's latest move; `None` without two reflog entries.
pub fn last_head_move(root: &str) -> Option<HeadMove> {
    let result = git(
        &[
            "reflog",
            "-2",
            "--date=unix",
            "--format=%H%x09%gd%x09%gs",
            "HEAD",
        ],
        root,
    )
    .ok()?;
    let lines: Vec<Vec<&str>> = result
        .stdout
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| l.splitn(3, '\t').collect())
        .collect();
    if lines.len() != 2 || lines[0].len() != 3 {
        return None;
    }
    let selector = lines[0][1];
    let open = selector.find('{')?;
    let close = selector.rfind('}')?;
    let seconds: f64 = selector.get(open + 1..close)?.parse().ok()?;
    Some(HeadMove {
        time: unix_time(seconds),
        subject: lines[0][2].into(),
        from: lines[1][0].into(),
        to: lines[0][0].into(),
    })
}

/// How HEAD last moved, from its reflog ("pull: Fast-forward", "checkout:
/// moving from main to topic"); `None` without a reflog.
pub fn head_reflog_subject(root: &str) -> Option<String> {
    let result = git(&["reflog", "-1", "--format=%gs", "HEAD"], root).ok()?;
    Some(trimmed(&result)).filter(|s| !s.is_empty())
}

/// The paths that differ between two commits (renames as a deletion and an
/// addition).
pub fn changed_paths(from: &str, to: &str, root: &str) -> Vec<String> {
    git(
        &["diff", "--name-only", "-z", "--no-renames", from, to, "--"],
        root,
    )
    .map(|r| split_nul(&r.stdout))
    .unwrap_or_default()
}

/// When `revision` was committed (committer date).
pub fn commit_date(revision: &str, root: &str) -> Option<SystemTime> {
    let result = git(&["log", "-1", "--format=%ct", revision, "--"], root).ok()?;
    trimmed(&result).parse::<f64>().ok().map(unix_time)
}

pub fn remotes(root: &str) -> Vec<String> {
    git(&["remote"], root)
        .map(|r| split_lines(&r.stdout))
        .unwrap_or_default()
}

/// The checked-out branch's name (`None` when detached).
pub fn current_branch(root: &str) -> Option<String> {
    let result = git(&["symbolic-ref", "--quiet", "--short", "HEAD"], root).ok()?;
    Some(trimmed(&result)).filter(|s| !s.is_empty())
}

/// The remote a branch publishes to, and tags are pushed to and browsed on:
/// `branch`'s configured remote (the checked-out branch's when `None`), else
/// `origin`, else the first one. `None` without remotes.
pub fn default_remote(root: &str, branch: Option<&str>) -> Option<String> {
    let remotes = remotes(root);
    let name = branch.map(str::to_string).or_else(|| current_branch(root));
    if let Some(name) = name {
        if let Ok(config) = git(&["config", "--get", &format!("branch.{name}.remote")], root) {
            let remote = trimmed(&config);
            if remotes.contains(&remote) {
                return Some(remote);
            }
        }
    }
    if remotes.iter().any(|r| r == "origin") {
        Some("origin".into())
    } else {
        remotes.into_iter().next()
    }
}

/// `default_remote` for each of `branches`, from two git processes however
/// many branches there are. Empty without remotes.
pub fn default_remotes(root: &str, branches: &[&str]) -> HashMap<String, String> {
    let remotes = remotes(root);
    let fallback = if remotes.iter().any(|r| r == "origin") {
        Some("origin".to_string())
    } else {
        remotes.first().cloned()
    };
    let (false, Some(fallback)) = (branches.is_empty(), fallback) else {
        return HashMap::new();
    };
    // "branch.<name>.remote\n<remote>\0" (names may contain dots); no match
    // at all is exit status 1.
    let mut configured = HashMap::new();
    if let Ok(result) = git(
        &["config", "-z", "--get-regexp", r"^branch\..*\.remote$"],
        root,
    ) {
        for entry in split_nul(&result.stdout) {
            let Some((key, value)) = entry.split_once('\n') else {
                continue;
            };
            if let Some(name) = key
                .strip_prefix("branch.")
                .and_then(|k| k.strip_suffix(".remote"))
            {
                configured.insert(name.to_string(), value.to_string());
            }
        }
    }
    branches
        .iter()
        .map(|branch| {
            let remote = configured
                .get(*branch)
                .filter(|r| remotes.contains(r))
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            (branch.to_string(), remote)
        })
        .collect()
}

/// The URL `remote` fetches from.
pub fn remote_url(remote: &str, root: &str) -> Option<String> {
    let result = git(&["remote", "get-url", remote], root).ok()?;
    Some(trimmed(&result)).filter(|s| !s.is_empty())
}

// MARK: Tags

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagInfo {
    pub name: String,
    /// The commit it points at.
    pub commit: String,
    /// Annotated tags carry a message, tagger and date of their own.
    pub is_annotated: bool,
    /// Tagger date (annotated) or the commit's date (lightweight).
    pub date: Option<SystemTime>,
    /// The annotation's first line, or the commit's subject.
    pub subject: String,
}

/// Tag names, newest first.
pub fn tags(root: &str) -> Vec<String> {
    git(&["tag", "--sort=-creatordate"], root)
        .map(|r| split_lines(&r.stdout))
        .unwrap_or_default()
}

/// Every tag with what it points at, newest first.
pub fn tag_details(root: &str) -> Vec<TagInfo> {
    let format = "--format=%(refname:short)%1f%(objecttype)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:unix)%1f%(contents:subject)%1e";
    let Ok(result) = git(
        &["for-each-ref", "--sort=-creatordate", format, "refs/tags"],
        root,
    ) else {
        return Vec::new();
    };
    result
        .stdout
        .split('\u{1e}')
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim_matches(['\n', '\r']).split('\u{1f}').collect();
            if fields.len() < 6 || fields[0].is_empty() {
                return None;
            }
            let annotated = fields[1] == "tag";
            Some(TagInfo {
                name: fields[0].into(),
                commit: if annotated && !fields[3].is_empty() {
                    fields[3].into()
                } else {
                    fields[2].into()
                },
                is_annotated: annotated,
                date: fields[4].parse::<f64>().ok().map(unix_time),
                subject: fields[5].into(),
            })
        })
        .collect()
}

/// Tag `revision`. With a message the tag is annotated (it records who
/// tagged it, when, and why); without one it is a lightweight name.
pub fn create_tag(
    name: &str,
    revision: &str,
    message: Option<&str>,
    force: bool,
    root: &str,
) -> GitResult {
    if !is_valid_ref_name(name) {
        return Err(OperationError::Invalid(format!(
            "\u{201c}{name}\u{201d} isn't a valid tag name."
        )));
    }
    let mut args = vec!["tag"];
    if force {
        args.push("--force");
    }
    let message = message.map(str::trim).unwrap_or("");
    if !message.is_empty() {
        args.extend(["--annotate", "--file=-", "--end-of-options", name, revision]);
        let stdin = format!("{message}\n");
        return done(git_with(
            &args,
            root,
            Call {
                stdin: Some(stdin.as_bytes()),
                ..Default::default()
            },
        ));
    }
    args.extend(["--end-of-options", name, revision]);
    done(git(&args, root))
}

pub fn delete_tag(name: &str, root: &str) -> GitResult {
    done(git(&["tag", "--delete", "--end-of-options", name], root))
}

/// The object a ref points at, unpeeled (an annotated tag's tag object).
pub fn resolve_ref(reference: &str, root: &str) -> Option<String> {
    let result = git(&["rev-parse", "--verify", "--quiet", reference], root).ok()?;
    Some(trimmed(&result)).filter(|s| !s.is_empty())
}

/// Point a ref at an object again (undoing a delete while the object is
/// still in the repository).
pub fn restore_ref(reference: &str, object: &str, root: &str) -> GitResult {
    done(git(&["update-ref", reference, object], root))
}

/// Commits in `to` that aren't in `from` (`git rev-list --count from..to`).
pub fn commit_count(from: &str, to: &str, root: &str) -> Option<usize> {
    let result = git(&["rev-list", "--count", &format!("{from}..{to}")], root).ok()?;
    trimmed(&result).parse().ok()
}

fn push_call(on_progress: Progress) -> Call {
    Call {
        timeout: Duration::from_secs(600),
        on_line: on_progress,
        ..Default::default()
    }
}

/// Push one tag to `remote` (`force`: a tag that was moved).
pub fn push_tag(
    name: &str,
    remote: &str,
    force: bool,
    root: &str,
    on_progress: Progress,
) -> GitResult {
    let refspec = format!("{}refs/tags/{name}", if force { "+" } else { "" });
    done(git_with(
        &["push", "--progress", remote, &refspec],
        root,
        push_call(on_progress),
    ))
}

/// Push every local tag to `remote`.
pub fn push_all_tags(remote: &str, root: &str, on_progress: Progress) -> GitResult {
    done(git_with(
        &["push", "--progress", "--tags", remote],
        root,
        push_call(on_progress),
    ))
}

/// Delete a branch on `remote` (the local branch is left alone).
pub fn delete_remote_branch(name: &str, remote: &str, root: &str) -> GitResult {
    done(git_with(
        &["push", remote, "--delete", &format!("refs/heads/{name}")],
        root,
        push_call(None),
    ))
}

/// Delete a branch on `remote` only while the remote still has it at
/// `commit` (a lease): the server refuses when someone pushed to it since.
pub fn delete_remote_branch_at(name: &str, remote: &str, commit: &str, root: &str) -> GitResult {
    done(git_with(
        &[
            "push",
            &format!("--force-with-lease=refs/heads/{name}:{commit}"),
            remote,
            &format!(":refs/heads/{name}"),
        ],
        root,
        push_call(None),
    ))
}

/// Delete a tag on `remote` (the local tag is left alone).
pub fn delete_remote_tag(name: &str, remote: &str, root: &str) -> GitResult {
    done(git_with(
        &["push", remote, "--delete", &format!("refs/tags/{name}")],
        root,
        push_call(None),
    ))
}

// MARK: Operations in progress

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationAction {
    Continue,
    Skip,
    Abort,
}

impl OperationAction {
    pub fn raw_value(self) -> &'static str {
        match self {
            OperationAction::Continue => "continue",
            OperationAction::Skip => "skip",
            OperationAction::Abort => "abort",
        }
    }
}

pub fn perform(action: OperationAction, operation: &RepoOperation, root: &str) -> GitResult {
    let command = match operation {
        RepoOperation::Merge => "merge",
        RepoOperation::Rebase { .. } => "rebase",
        RepoOperation::CherryPick => "cherry-pick",
        RepoOperation::Revert => "revert",
        RepoOperation::ApplyMailbox => "am",
        RepoOperation::Bisect => {
            return if action == OperationAction::Abort {
                done(git(&["bisect", "reset"], root))
            } else {
                Err(OperationError::Invalid(
                    "Bisect is driven from the terminal.".into(),
                ))
            };
        }
    };
    if action == OperationAction::Skip && !operation.can_skip() {
        return Err(OperationError::Invalid(format!(
            "{} can't skip a step.",
            operation.title()
        )));
    }
    done(git_with(
        &[command, &format!("--{}", action.raw_value())],
        root,
        Call {
            timeout: Duration::from_secs(600),
            ..Default::default()
        },
    ))
}

/// Mark conflicted files resolved (stage them).
pub fn mark_resolved<S: AsRef<str>>(paths: &[S], root: &str) -> GitResult {
    stage(paths, root)
}

// MARK: History edits

/// Resolve conflicted files by taking one side whole (`ours` is HEAD,
/// `theirs` the incoming change) and marking them resolved.
pub fn resolve_conflicts<S: AsRef<str>>(paths: &[S], take_ours: bool, root: &str) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    let side = if take_ours { "--ours" } else { "--theirs" };
    git(&literal(&strings(&["checkout", side, "--"], paths)), root)?;
    done(git(&literal(&strings(&["add", "--"], paths)), root))
}

/// Where conflicts were resolved: HEAD and the operation in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictPoint {
    pub head: Option<String>,
    pub operation: Option<RepoOperation>,
}

/// Record before `resolve_conflicts`, for `reopen_conflicts`.
pub fn conflict_point(root: &str) -> ConflictPoint {
    ConflictPoint {
        head: scoped_diff::resolve_commit(root, "HEAD"),
        operation: current_operation(root),
    }
}

/// Put resolved files back in conflict, markers and all (`git checkout -m`,
/// from the index's record of the resolved conflict): the undo of
/// `resolve_conflicts`. Refused once the operation has moved on from `point`
/// (committed, continued, skipped or aborted): git keeps that record past the
/// commit, and the conflict would come back into an index with no operation
/// left to finish.
pub fn reopen_conflicts<S: AsRef<str>>(
    paths: &[S],
    point: &ConflictPoint,
    root: &str,
) -> GitResult {
    if paths.is_empty() {
        return Ok(());
    }
    let now = conflict_point(root);
    if now != *point {
        let name = match &point.operation {
            Some(RepoOperation::Merge) => "merge",
            Some(RepoOperation::Rebase { .. }) => "rebase",
            Some(RepoOperation::CherryPick) => "cherry-pick",
            Some(RepoOperation::Revert) => "revert",
            Some(RepoOperation::Bisect) => "bisect",
            Some(RepoOperation::ApplyMailbox) => "patch series",
            None => {
                return Err(OperationError::Stale(
                    "The repository has moved on since the conflict was resolved, so it can't be put back.".into(),
                ))
            }
        };
        if now.operation.is_none() {
            return Err(OperationError::Stale(format!(
                "The {name} is no longer in progress, so the conflict can't be put back."
            )));
        }
        return Err(OperationError::Stale(format!(
            "The {name} has moved on since the conflict was resolved, so it can't be put back."
        )));
    }
    done(git(
        &literal(&strings(&["checkout", "-m", "--"], paths)),
        root,
    ))
}

/// `Keep` moves HEAD and updates the files the move changes, but stops
/// rather than touch local modifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
    Keep,
}

pub fn reset(mode: ResetMode, revision: &str, root: &str) -> GitResult {
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
        ResetMode::Keep => "--keep",
    };
    done(git(
        &["reset", flag, "--end-of-options", revision, "--"],
        root,
    ))
}

fn long_call() -> Call<'static> {
    Call {
        timeout: Duration::from_secs(600),
        ..Default::default()
    }
}

/// Merge `revision` into the current branch (a fast-forward when possible).
/// Conflicts leave the merge open for Continue or Abort.
pub fn merge(revision: &str, no_fast_forward: bool, root: &str) -> GitResult {
    let mut args = vec!["merge", "--no-edit"];
    if no_fast_forward {
        args.push("--no-ff");
    }
    args.extend(["--end-of-options", revision]);
    done(git_with(&args, root, long_call()))
}

/// Move the current branch up to `revision` when it only adds commits;
/// fails when the branch has commits of its own.
pub fn fast_forward(revision: &str, root: &str) -> GitResult {
    done(git_with(
        &["merge", "--ff-only", "--end-of-options", revision],
        root,
        long_call(),
    ))
}

/// Replay the current branch's own commits on top of `revision`. Conflicts
/// leave the rebase open for Continue, Skip or Abort.
pub fn rebase(revision: &str, root: &str) -> GitResult {
    done(git_with(
        &["rebase", "--end-of-options", revision],
        root,
        long_call(),
    ))
}

pub fn cherry_pick(revision: &str, root: &str) -> GitResult {
    done(git_with(
        &["cherry-pick", "--end-of-options", revision],
        root,
        long_call(),
    ))
}

pub fn revert(revision: &str, root: &str) -> GitResult {
    done(git_with(
        &["revert", "--no-edit", "--end-of-options", revision],
        root,
        long_call(),
    ))
}

pub fn checkout_detached(revision: &str, root: &str) -> GitResult {
    done(git(
        &["switch", "--detach", "--end-of-options", revision],
        root,
    ))
}

// MARK: Worktrees

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

pub fn worktrees(root: &str) -> Vec<Worktree> {
    worktree_list(root).unwrap_or_default()
}

/// `worktrees`, or `None` when git couldn't list them (rather than "there
/// are none").
pub fn worktree_list(root: &str) -> Option<Vec<Worktree>> {
    let result = git(&["worktree", "list", "--porcelain"], root).ok()?;
    let mut list = Vec::new();
    for block in result.stdout.split("\n\n").filter(|b| !b.is_empty()) {
        let mut worktree = Worktree {
            path: String::new(),
            head: None,
            branch: None,
            is_bare: false,
            is_detached: false,
            is_locked: false,
            is_prunable: false,
        };
        let mut path = None;
        for line in block.split('\n').filter(|l| !l.is_empty()) {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                worktree.head = Some(value.into());
            } else if let Some(reference) = line.strip_prefix("branch ") {
                worktree.branch = Some(
                    reference
                        .strip_prefix("refs/heads/")
                        .unwrap_or(reference)
                        .into(),
                );
            } else if line == "bare" {
                worktree.is_bare = true;
            } else if line == "detached" {
                worktree.is_detached = true;
            } else if line.starts_with("locked") {
                worktree.is_locked = true;
            } else if line.starts_with("prunable") {
                worktree.is_prunable = true;
            }
        }
        if let Some(path) = path {
            worktree.path = path;
            list.push(worktree);
        }
    }
    Some(list)
}

/// Add a worktree at `path`. With `new_branch`, create `branch` from `base`
/// (HEAD by default); otherwise check out the existing `branch`. A new
/// branch doesn't track its base unless `track` says so: from `origin/main`
/// it would take that as its upstream, and its first push would go to (or
/// be refused for) main. `track` is for a local copy of a remote branch
/// (`feature-x` from `origin/feature-x`), which should push back to it.
pub fn add_worktree(
    path: &str,
    branch: &str,
    new_branch: bool,
    base: Option<&str>,
    track: bool,
    root: &str,
) -> GitResult {
    let mut args = vec!["worktree", "add"];
    if new_branch {
        args.extend([
            if track { "--track" } else { "--no-track" },
            "-b",
            branch,
            "--end-of-options",
            path,
        ]);
        args.extend(base);
    } else {
        args.extend(["--end-of-options", path, branch]);
    }
    done(git_with(
        &args,
        root,
        Call {
            timeout: Duration::from_secs(300),
            ..Default::default()
        },
    ))
}

pub fn remove_worktree(path: &str, force: bool, root: &str) -> GitResult {
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(path);
    done(git(&args, root))
}

pub fn prune_worktrees(root: &str) -> GitResult {
    done(git(&["worktree", "prune"], root))
}
