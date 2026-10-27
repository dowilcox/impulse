//! The write path of the git layer (the Swift `GitCLI`).
//!
//! libgit2 (the rest of this crate) is used for reads: status, diffs, blame,
//! log. Mutations run the user's `git` binary instead so they behave exactly
//! like the git the user (and their agents) run in the terminal: hooks run,
//! commits are signed per config, LFS and clean/smudge filters apply,
//! credential helpers and SSH agents work for remote operations.
//!
//! Every command runs with `-c log.showSignature=false` before its own
//! arguments: `log.showSignature` (a repository's own, too) makes `log`,
//! `show` and `reflog` run `gpg.program` and print its lines among the output
//! Impulse parses. In a folder that isn't trusted, reads take
//! `restricted_environment`, which turns off every program the repository's
//! configuration names.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use crate::process;

/// Before every command's own arguments.
pub const ALWAYS_ARGUMENTS: [&str; 2] = ["-c", "log.showSignature=false"];

/// Output of a `git` invocation that exited 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Why a `git` invocation failed, classified from its output so the UI can
/// show a plain-English explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    GitNotFound,
    CommandLineToolsMissing,
    NotARepository,
    IdentityNotConfigured,
    IndexLocked,
    NonFastForward,
    NoUpstream,
    AuthenticationFailed,
    LocalChangesWouldBeOverwritten,
    MergeConflict,
    NothingToCommit,
    UnknownRevision,
    BranchAlreadyExists,
    TagAlreadyExists,
    /// `pull --ff-only` on a branch that has diverged from its upstream.
    Diverged,
    HookFailed,
    TimedOut,
    Cancelled,
    Other,
}

/// A failed `git` invocation, with the raw output kept for details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    pub kind: ErrorKind,
    /// The arguments passed to git (without the binary path).
    pub arguments: Vec<String>,
    pub status: i32,
    /// Raw stderr (or stdout when stderr was empty) from git.
    pub output: String,
}

impl CliError {
    /// One sentence describing what went wrong and what to do about it.
    pub fn message(&self) -> String {
        match self.kind {
            ErrorKind::GitNotFound => if cfg!(target_os = "macos") {
                "Git isn't installed or isn't on your PATH. Install it with Homebrew or the Xcode Command Line Tools."
            } else {
                "Git isn't installed or isn't on your PATH. Install it with your distribution's package manager."
            }
            .into(),
            ErrorKind::CommandLineToolsMissing => "Git needs the Xcode Command Line Tools. Run `xcode-select --install` in a terminal.".into(),
            ErrorKind::NotARepository => "This folder isn't inside a git repository.".into(),
            ErrorKind::IdentityNotConfigured => "Git doesn't know who you are yet. Set `git config --global user.name` and `user.email`.".into(),
            ErrorKind::IndexLocked => "Another git process is using this repository (index.lock exists). Wait for it to finish, then try again.".into(),
            ErrorKind::NonFastForward => "The remote has commits you don't have. Pull before pushing.".into(),
            ErrorKind::NoUpstream => "This branch has no upstream branch. Publish it first.".into(),
            ErrorKind::AuthenticationFailed => "Git couldn't authenticate with the remote. Run the command in a terminal to sign in.".into(),
            ErrorKind::LocalChangesWouldBeOverwritten => "Your uncommitted changes would be overwritten. Commit or stash them first.".into(),
            ErrorKind::MergeConflict => "The operation stopped with conflicts. Resolve them, then continue.".into(),
            ErrorKind::NothingToCommit => "There's nothing to commit.".into(),
            ErrorKind::UnknownRevision => "Git doesn't know that branch or revision.".into(),
            ErrorKind::BranchAlreadyExists => "A branch with that name already exists.".into(),
            ErrorKind::TagAlreadyExists => "A tag with that name already exists.".into(),
            ErrorKind::Diverged => "Your branch and its upstream have diverged, so a fast-forward isn't possible. Pull with rebase or merge.".into(),
            ErrorKind::HookFailed => "A git hook rejected the operation. See the hook output for details.".into(),
            ErrorKind::TimedOut => "Git took too long and was stopped.".into(),
            ErrorKind::Cancelled => "The git operation was cancelled.".into(),
            ErrorKind::Other => match self.output.split('\n').find(|line| !line.is_empty()) {
                Some(first) => first.to_string(),
                None => format!("Git failed (exit {}).", self.status),
            },
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "git {}: {}", self.arguments.join(" "), self.message())
    }
}

impl std::error::Error for CliError {}

/// How to run one command.
pub struct RunOptions<'a> {
    /// Bytes written to git's standard input (a commit message for
    /// `commit -F -`, a patch for `apply --cached`).
    pub stdin: Option<&'a [u8]>,
    /// Extra variables layered over the inherited environment.
    pub environment: Option<&'a HashMap<String, String>>,
    /// Wall-clock limit (`None`: none); when exceeded, git and everything it
    /// started are terminated.
    pub timeout: Option<Duration>,
    /// Called for each stderr line as it arrives: git writes progress for
    /// fetch and push there.
    pub on_output_line: Option<&'a mut dyn FnMut(&str)>,
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        RunOptions {
            stdin: None,
            environment: None,
            timeout: Some(Duration::from_secs(120)),
            on_output_line: None,
        }
    }
}

/// Run `git <arguments>` in `directory` and wait for it to finish: the output
/// on exit status 0, otherwise a classified error.
pub fn run<S: AsRef<str>>(
    arguments: &[S],
    directory: &str,
    options: RunOptions,
) -> Result<CliOutput, CliError> {
    let arguments: Vec<String> = arguments.iter().map(|a| a.as_ref().to_string()).collect();
    let failure = |kind, status, output: String| CliError {
        kind,
        arguments: arguments.clone(),
        status,
        output,
    };
    let Some(git) = git_path() else {
        return Err(failure(ErrorKind::GitNotFound, -1, String::new()));
    };
    if is_missing_tools_shim(&git) {
        return Err(failure(
            ErrorKind::CommandLineToolsMissing,
            -1,
            String::new(),
        ));
    }

    let RunOptions {
        stdin,
        environment,
        timeout,
        on_output_line,
    } = options;
    let environment = merged_environment(environment);
    let mut lines = LineSplitter::default();
    let mut on_line = on_output_line;
    let mut on_stderr = |chunk: &[u8]| {
        if let Some(callback) = on_line.as_mut() {
            lines.feed(chunk, &mut **callback);
        }
    };
    let full: Vec<&str> = ALWAYS_ARGUMENTS
        .iter()
        .copied()
        .chain(arguments.iter().map(String::as_str))
        .collect();
    let result = process::run(
        &git,
        &full,
        process::Options {
            directory: Some(directory),
            environment: Some(&environment),
            stdin,
            timeout,
            on_stderr: Some(&mut on_stderr),
        },
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => return Err(failure(ErrorKind::GitNotFound, -1, error.to_string())),
    };
    if let Some(callback) = on_line.as_mut() {
        lines.flush(&mut **callback);
    }

    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
    if result.timed_out {
        return Err(failure(ErrorKind::TimedOut, result.status, stderr));
    }
    if result.status == 0 {
        return Ok(CliOutput {
            status: 0,
            stdout,
            stderr,
        });
    }
    let kind = classify(&format!("{stdout}\n{stderr}"), &arguments);
    let output = if stderr.is_empty() { stdout } else { stderr };
    Err(failure(kind, result.status, output))
}

/// Absolute path of the git binary on the login-shell PATH, or `None` when
/// git is unavailable. Looked up once.
pub fn git_path() -> Option<String> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| which("git").map(|path| past_tools_shim(&path)))
        .clone()
}

/// The git that `path` runs. On macOS `/usr/bin/git` is an xcrun shim that
/// looks the developer tools' git up and starts it: two program launches
/// for every git command, which adds up where a security agent checks each
/// launch. So the tools' git runs directly, the one the shim would pick.
pub fn past_tools_shim(path: &str) -> String {
    if !cfg!(target_os = "macos") || path != "/usr/bin/git" || is_missing_tools_shim(path) {
        return path.to_string();
    }
    let Ok(found) = process::run(
        "/usr/bin/xcrun",
        &["--find", "git"],
        process::Options {
            timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        },
    ) else {
        return path.to_string();
    };
    let tools = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if found.status == 0 && tools.starts_with('/') && is_executable(Path::new(&tools)) {
        tools
    } else {
        path.to_string()
    }
}

/// On macOS `/usr/bin/git` is an xcrun shim: without the Command Line Tools
/// (or Xcode) installed, running it pops a system install dialog instead of
/// running git. That case is detected so the dialog never appears by
/// surprise. Elsewhere `/usr/bin/git` is git itself.
pub fn is_missing_tools_shim(path: &str) -> bool {
    if !cfg!(target_os = "macos") || path != "/usr/bin/git" {
        return false;
    }
    !Path::new("/Library/Developer/CommandLineTools/usr/bin/git").exists()
        && !Path::new("/Applications/Xcode.app/Contents/Developer/usr/bin/git").exists()
}

/// Environment for git: this process's environment with the login PATH, no
/// interactive prompts (a GUI can't answer them), and English messages so
/// `classify` can recognise them. `extra` goes on top.
pub fn merged_environment(extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert("PATH".into(), login_path());
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    env.entry("GIT_ASKPASS".into()).or_default();
    env.entry("SSH_ASKPASS".into()).or_default();
    env.insert("GCM_INTERACTIVE".into(), "never".into());
    env.insert("LC_MESSAGES".into(), "C".into());
    env.insert("GIT_EDITOR".into(), "true".into());
    if let Some(extra) = extra {
        env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    env
}

/// Environment for a git read in a folder that isn't trusted, so it runs no
/// program the repository's own configuration names: an fsmonitor hook,
/// hooks, a clean/smudge filter driver (`git status` runs clean filters on
/// files whose timestamps changed), a merge driver (`git merge-tree` runs
/// those), or a diff's external command or textconv (`git blame` runs
/// textconv drivers; one set empty makes git refuse rather than run
/// anything). Set through `GIT_CONFIG_KEY_<n>` rather than `-c`, which a driver
/// name containing "=" would get past. `None` when the configuration can't
/// be read, so the drivers aren't known: the read mustn't run then.
pub fn restricted_environment(
    root: &str,
    base: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let mut keys: Vec<String> = vec!["core.fsmonitor".into(), "core.hooksPath".into()];
    let mut values: Vec<String> = vec!["false".into(), "/dev/null".into()];
    let pattern = r"^(filter\..*\.(clean|smudge|process)|merge\..*\.driver|diff\..*\.(textconv|command)|diff\.external)$";
    let listed = run(
        &["config", "-z", "--name-only", "--get-regexp", pattern],
        root,
        RunOptions {
            environment: Some(base),
            timeout: Some(Duration::from_secs(15)),
            ..Default::default()
        },
    );
    let names: Vec<String> = match listed {
        Ok(result) => result.stdout.split('\0').map(str::to_string).collect(),
        // Exit 1 with nothing said: no driver is configured.
        Err(error)
            if error.kind != ErrorKind::TimedOut
                && error.status == 1
                && error.output.is_empty() =>
        {
            Vec::new()
        }
        Err(_) => return None,
    };
    let mut drivers = HashSet::new();
    for name in names.into_iter().filter(|n| !n.is_empty()) {
        if name.starts_with("merge.") {
            // A merge driver that always reports a conflict.
            keys.push(name);
            values.push("false".into());
            continue;
        }
        if name.starts_with("diff.") {
            keys.push(name);
            values.push(String::new());
            continue;
        }
        // Not required either: a required filter with no command fails.
        let driver = match name.rfind('.') {
            Some(dot) => name[..dot].to_string(),
            None => name.clone(),
        };
        keys.push(name);
        values.push(String::new());
        if drivers.insert(driver.clone()) {
            keys.push(format!("{driver}.required"));
            values.push("false".into());
        }
    }
    let mut env = base.clone();
    env.insert("GIT_CONFIG_COUNT".into(), keys.len().to_string());
    for (index, (key, value)) in keys.into_iter().zip(values).enumerate() {
        env.insert(format!("GIT_CONFIG_KEY_{index}"), key);
        env.insert(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    Some(env)
}

/// Classify git's output into a known failure kind.
pub fn classify<S: AsRef<str>>(output: &str, arguments: &[S]) -> ErrorKind {
    let text = output.to_lowercase();
    let has = |needle: &str| text.contains(needle);
    let first = arguments.first().map(|a| a.as_ref());
    let contains = |argument: &str| arguments.iter().any(|a| a.as_ref() == argument);

    if has("not a git repository") {
        return ErrorKind::NotARepository;
    }
    if has("please tell me who you are")
        || has("unable to auto-detect email address")
        || has("empty ident name")
    {
        return ErrorKind::IdentityNotConfigured;
    }
    if has("index.lock") && (has("file exists") || has("another git process")) {
        return ErrorKind::IndexLocked;
    }
    if has("would be overwritten by") || has("your local changes to the following files") {
        return ErrorKind::LocalChangesWouldBeOverwritten;
    }
    if has("already exists")
        && (first == Some("tag")
            || contains("--tags")
            || arguments
                .iter()
                .any(|a| a.as_ref().starts_with("refs/tags/")))
    {
        return ErrorKind::TagAlreadyExists;
    }
    if has("not possible to fast-forward") || has("diverging branches can't be fast-forwarded") {
        return ErrorKind::Diverged;
    }
    if has("[rejected]")
        || has("non-fast-forward")
        || has("updates were rejected")
        || has("fetch first")
    {
        return ErrorKind::NonFastForward;
    }
    if has("has no upstream branch")
        || has("no upstream configured")
        || has("there is no tracking information")
    {
        return ErrorKind::NoUpstream;
    }
    if has("authentication failed")
        || has("could not read username")
        || has("permission denied (publickey")
        || has("terminal prompts disabled")
        || has("could not read from remote repository")
    {
        return ErrorKind::AuthenticationFailed;
    }
    if has("conflict (")
        || has("merge conflict")
        || has("fix conflicts and then")
        || has("needs merge")
        || has("you have unmerged paths")
    {
        return ErrorKind::MergeConflict;
    }
    if has("nothing to commit") || has("no changes added to commit") {
        return ErrorKind::NothingToCommit;
    }
    if has("already exists") && matches!(first, Some("branch" | "switch" | "checkout" | "worktree"))
    {
        return ErrorKind::BranchAlreadyExists;
    }
    if has("did not match any file(s) known to git")
        || has("invalid reference")
        || has("unknown revision")
        || has("not a valid object name")
        || has("invalid branch name")
    {
        return ErrorKind::UnknownRevision;
    }
    if (has("hook") && (has("declined") || has("failed") || has("exit")))
        || (first == Some("commit") && has("pre-commit"))
    {
        return ErrorKind::HookFailed;
    }
    ErrorKind::Other
}

/// Splits streamed bytes into lines on `\n` and `\r` (git progress rewrites
/// one line with carriage returns), skipping empty ones.
#[derive(Default)]
struct LineSplitter {
    buffer: Vec<u8>,
}

impl LineSplitter {
    fn feed(&mut self, data: &[u8], on_line: &mut dyn FnMut(&str)) {
        self.buffer.extend_from_slice(data);
        while let Some(index) = self.buffer.iter().position(|&b| b == b'\n' || b == b'\r') {
            let line: Vec<u8> = self.buffer.drain(..=index).collect();
            let line = String::from_utf8_lossy(&line[..index]);
            if !line.is_empty() {
                on_line(&line);
            }
        }
    }

    fn flush(&mut self, on_line: &mut dyn FnMut(&str)) {
        if !self.buffer.is_empty() {
            on_line(&String::from_utf8_lossy(&self.buffer));
            self.buffer.clear();
        }
    }
}

// MARK: Login-shell PATH (ImpulseKit's `LoginShell` in the Swift app)

/// The `PATH` an interactive login shell would see. Apps started from the
/// Dock, Finder or a desktop launcher inherit a minimal `PATH`, which misses
/// Homebrew, mise/asdf shims, `~/.local/bin` and what rc files add, so `git`
/// is resolved against the user's real login environment. Captured once, with
/// a timeout, and kept; falls back to this process's `PATH` plus common
/// install prefixes.
pub fn login_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let captured = capture_login_path().unwrap_or_default();
        let fallback =
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
        merge_paths(&[&captured, &fallback, "/opt/homebrew/bin:/usr/local/bin"])
    })
    .clone()
}

/// Joins colon-separated PATH strings, dropping empty and duplicate entries
/// while keeping first-seen order.
pub fn merge_paths(paths: &[&str]) -> String {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in paths {
        for entry in path.split(':').filter(|e| !e.is_empty()) {
            if seen.insert(entry) {
                out.push(entry);
            }
        }
    }
    out.join(":")
}

/// `name` on the login `PATH`, or as given when it's a path.
pub fn which(name: &str) -> Option<String> {
    if name.contains('/') {
        return is_executable(Path::new(name)).then(|| name.to_string());
    }
    login_path()
        .split(':')
        .map(|dir| format!("{dir}/{name}"))
        .find(|candidate| is_executable(Path::new(candidate)))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The PATH of an interactive login shell, like a terminal's, so entries
/// added in `.zshrc` or `.bashrc` count too. An rc file that never finishes
/// (one that execs tmux) falls back to a login-only shell.
/// `IMPULSE_RESOLVING_ENVIRONMENT=1` lets rc files skip slow or interactive
/// setup.
fn capture_login_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| Path::new(s).exists())?;
    // `printf '%s' "$PATH"` works in bash, zsh and fish (fish joins PATH
    // variables with ':' when quoted). Markers isolate it from rc-file noise.
    let script = "printf '__IMPULSE_PATH__%s__IMPULSE_PATH__' \"$PATH\"";
    capture_path(&shell, &["-i", "-l", "-c", script], 5)
        .or_else(|| capture_path(&shell, &["-l", "-c", script], 3))
}

fn capture_path(shell: &str, arguments: &[&str], timeout: u64) -> Option<String> {
    let mut environment: HashMap<String, String> = std::env::vars().collect();
    environment.insert("IMPULSE_RESOLVING_ENVIRONMENT".into(), "1".into());
    let result = process::run(
        shell,
        arguments,
        process::Options {
            environment: Some(&environment),
            timeout: Some(Duration::from_secs(timeout)),
            ..Default::default()
        },
    )
    .ok()?;
    if result.timed_out {
        return None;
    }
    let output = String::from_utf8_lossy(&result.stdout);
    let parts: Vec<&str> = output.split("__IMPULSE_PATH__").collect();
    let path = parts.get(1)?.trim();
    (parts.len() >= 3 && !path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_progress_lines_on_carriage_returns() {
        let mut lines = Vec::new();
        let mut splitter = LineSplitter::default();
        let mut push = |line: &str| lines.push(line.to_string());
        splitter.feed(b"Counting: 1%\rCounting: 50%\rCount", &mut push);
        splitter.feed(b"ing: 100%\n\nremote: done", &mut push);
        splitter.flush(&mut push);
        assert_eq!(
            lines,
            [
                "Counting: 1%",
                "Counting: 50%",
                "Counting: 100%",
                "remote: done"
            ]
        );
    }

    #[test]
    fn merges_paths_in_order_without_duplicates() {
        assert_eq!(merge_paths(&["/a:/b", "", "/b:/c:", "/a"]), "/a:/b:/c");
    }
}
