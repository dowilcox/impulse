//! Inline completion for the terminal input bar (the Swift
//! `InputCompletion`, itself ported from the Rust-era
//! `impulse-core/src/completion.rs`).
//!
//! Given the current input and working directory, `complete` returns the
//! single best completed line, shown as dimmed ghost text after what was
//! typed. Sources, in priority order:
//!
//! 1. Command history: the most recent command that extends the full input
//!    (Warp-style autosuggest).
//! 2. Word completion of the token at the cursor, driven by the shell
//!    parser: executables on the login PATH (plus a curated common-command
//!    list and shell builtins) for the command word, a built-in subcommand
//!    and flag table for popular tools, and filesystem entries for paths.
//!
//! Completed names are escaped for the terminal's shell, so a file called
//! `$(touch x)` is inserted as a name, never as shell syntax. A `~/` or
//! `$HOME/` typed bare is read as the home folder and stays as typed; every
//! other `~`, `$` or glob in a token is taken literally, as the parser and
//! path matching take it.
//!
//! `complete_candidates` lists the path candidates for the completion menu.

use std::path::Path;
use std::sync::OnceLock;

use super::{CompletionCandidate, CompletionResult, TextSpan};
use crate::shell::{login, quoting, ShellType};
use crate::shell_parser::{parse_shell_input, ShellCompletionKind};

/// The best inline completion for `input`: the full completed line (which
/// always starts with `input`), or `None` when there's no useful
/// completion. `history` is newest first.
pub fn complete<S: AsRef<str>>(
    input: &str,
    cwd: Option<&str>,
    history: &[S],
    shell: ShellType,
) -> Option<String> {
    if input.chars().all(char::is_whitespace) {
        return None;
    }
    if let Some(found) = history_continuation(input, history) {
        return Some(found.to_string());
    }

    let parsed = parse_shell_input(input, input.len());
    // Splicing into quoted or escaped tokens is ambiguous; history already
    // covers those cases.
    if parsed.incomplete {
        return None;
    }
    let completion = parsed.completion;
    if completion.prefix.is_empty() {
        return None;
    }

    let cwd = cwd.filter(|c| !c.is_empty());
    let home = home_prefix(raw_token(input, completion.span));
    let replacement = match completion.kind {
        ShellCompletionKind::Command => {
            complete_command(&completion.prefix).map(|c| quoting::escaped(&c, shell))
        }
        ShellCompletionKind::RedirectTarget => complete_path(&completion.prefix, cwd, home, shell),
        ShellCompletionKind::EnvAssignment => None,
        ShellCompletionKind::Argument => complete_argument(
            completion.command.as_deref(),
            completion.argument_index,
            &completion.prefix,
            cwd,
            home,
            shell,
        ),
    }?;
    splice(input, completion.span, &replacement)
}

/// The most recent command in `history` (newest first) that extends
/// `input`. Only compares strings, so it's cheap enough per keystroke on
/// the UI thread; the rest of `complete` reads the filesystem.
pub fn history_continuation<'a, S: AsRef<str>>(input: &str, history: &'a [S]) -> Option<&'a str> {
    if input.is_empty() {
        return None;
    }
    history
        .iter()
        .map(AsRef::as_ref)
        .find(|command| command.len() > input.len() && command.starts_with(input))
}

/// List the PATH's executables now, off the hot path, so the first
/// completion keystroke doesn't pay for the scan (or for capturing the
/// login shell's PATH). Safe to call from a background thread.
pub fn warm_cache() {
    path_executables();
}

/// Path candidates for the active token in `input`, for the menu.
///
/// Only argument and redirect-target tokens list filesystem entries; other
/// tokens get no candidates. Candidates are directories first, then files,
/// each in byte order, prefix matched (case-sensitive). Hidden (dot) entries
/// appear only when the token's basename starts with `.`. The list is
/// capped at `limit`.
pub fn complete_candidates(input: &str, cwd: Option<&str>, limit: usize) -> CompletionResult {
    let parsed = parse_shell_input(input, input.len());
    let completion = parsed.completion;
    let span = completion.span;
    let empty = || CompletionResult {
        span,
        candidates: Vec::new(),
    };
    if parsed.incomplete {
        return empty();
    }
    if !matches!(
        completion.kind,
        ShellCompletionKind::Argument | ShellCompletionKind::RedirectTarget
    ) {
        return empty();
    }

    let cwd = cwd.filter(|c| !c.is_empty());
    let (dir_part, _) = split_path_prefix(&completion.prefix);
    let home = home_prefix(raw_token(input, span));

    let mut matches = path_matches(&completion.prefix, cwd, home);
    matches.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    let candidates = matches
        .into_iter()
        .take(limit)
        .map(|m| {
            let value = format!("{dir_part}{}{}", m.name, if m.is_dir { "/" } else { "" });
            CompletionCandidate {
                kept_prefix: Some(home.unwrap_or_default().to_string()),
                ..CompletionCandidate::new(value, m.name, "path", m.is_dir)
            }
        })
        .collect();
    CompletionResult { span, candidates }
}

/// `input` with the completion token replaced by `replacement` (shell
/// text). `None` unless that only adds to what was typed: when the typed
/// text reads differently escaped (a `~` that matched a folder named `~`),
/// there's nothing to suggest after the cursor.
fn splice(input: &str, span: TextSpan, replacement: &str) -> Option<String> {
    let start = span.start.min(input.len());
    let mut result = input.as_bytes()[..start].to_vec();
    result.extend_from_slice(replacement.as_bytes());
    if result.len() <= input.len() || !result.starts_with(input.as_bytes()) {
        return None;
    }
    String::from_utf8(result).ok()
}

/// The completion token as typed (quotes and backslashes included).
fn raw_token(input: &str, span: TextSpan) -> &str {
    let start = span.start.min(input.len());
    let end = span.end.clamp(start, input.len());
    input.get(start..end).unwrap_or("")
}

/// How a path token typed as `raw` starts with the home folder, the way the
/// shell reads it: a bare `~/` or `$HOME/`. Quoted or escaped they're plain
/// names, as is everything else in the token, so they're escaped when
/// inserted.
pub(crate) fn home_prefix(raw: &str) -> Option<&'static str> {
    ["~/", "$HOME/"].into_iter().find(|p| raw.starts_with(p))
}

// Command words

fn complete_command(prefix: &str) -> Option<String> {
    // A curated common command makes the single guess stable and useful
    // (`g` gives `git`, not `gpg`); the list is ordered by commonness.
    if let Some(common) = COMMON_COMMANDS.iter().find(|c| c.starts_with(prefix)) {
        return Some(common.to_string());
    }
    if let Some(builtin) = SHELL_BUILTINS.iter().find(|c| c.starts_with(prefix)) {
        return Some(builtin.to_string());
    }
    // Otherwise the shortest matching executable on PATH.
    path_executables()
        .iter()
        .filter(|name| name.starts_with(prefix))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .cloned()
}

/// The executables on the login PATH, sorted, listed once per process:
/// a tool installed mid-session won't complete until restart, which is an
/// acceptable trade for not listing PATH on every keystroke.
pub(crate) fn path_executables() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| scan_path_executables(&login::login_path()))
}

fn scan_path_executables(path: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if is_executable_entry(&entry.path()) {
                names.insert(name);
            }
        }
    }
    names.into_iter().collect()
}

/// A regular file or a symlink (not followed) with any execute bit set.
fn is_executable_entry(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let kind = meta.file_type();
    if !kind.is_file() && !kind.is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// Arguments: subcommands, flags, filesystem paths

/// The completed argument as shell text.
fn complete_argument(
    command: Option<&str>,
    argument_index: usize,
    prefix: &str,
    cwd: Option<&str>,
    home: Option<&str>,
    shell: ShellType,
) -> Option<String> {
    if prefix.starts_with('-') {
        return complete_flag(command, prefix).map(|f| quoting::escaped(f, shell));
    }
    // The first argument after a known command is usually a subcommand.
    if argument_index == 0 {
        if let Some(sub) = command.and_then(|c| complete_subcommand(c, prefix)) {
            return Some(quoting::escaped(sub, shell));
        }
    }
    complete_path(prefix, cwd, home, shell)
}

fn complete_subcommand(command: &str, prefix: &str) -> Option<&'static str> {
    let base = command_basename(command);
    let (_, subs) = SUBCOMMANDS.iter().find(|(name, _)| *name == base)?;
    subs.iter().find(|s| s.starts_with(prefix)).copied()
}

fn complete_flag(command: Option<&str>, prefix: &str) -> Option<&'static str> {
    if let Some(command) = command.map(command_basename) {
        if let Some((_, flags)) = FLAGS.iter().find(|(name, _)| *name == command) {
            if let Some(flag) = flags.iter().find(|f| f.starts_with(prefix)) {
                return Some(flag);
            }
        }
    }
    COMMON_FLAGS.iter().find(|f| f.starts_with(prefix)).copied()
}

/// A filesystem entry that prefix-matches a path token.
struct PathMatch {
    /// The entry's file name as read from disk.
    name: String,
    is_dir: bool,
}

/// Split a path prefix into its directory part (kept verbatim) and the
/// trailing basename being matched: `alpha/inn` gives (`alpha/`, `inn`).
fn split_path_prefix(prefix: &str) -> (&str, &str) {
    match prefix.rfind('/') {
        Some(index) => (&prefix[..=index], &prefix[index + 1..]),
        None => ("", prefix),
    }
}

/// Every entry in the folder `prefix` names whose name starts with its
/// trailing basename. Hidden entries surface only when the user typed a
/// leading dot. Empty when the folder can't be resolved or read.
fn path_matches(prefix: &str, cwd: Option<&str>, home: Option<&str>) -> Vec<PathMatch> {
    let (dir_part, base) = split_path_prefix(prefix);
    let Some(search_dir) = resolve_dir(dir_part, cwd, home) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&search_dir) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') && !base.starts_with('.') {
            continue;
        }
        if !name.starts_with(base) {
            continue;
        }
        // Not following symlinks: a symlink to a folder isn't a folder.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        matches.push(PathMatch { name, is_dir });
    }
    matches
}

/// The best path for `prefix` as shell text: the home `~/` or `$HOME/` it
/// was found under stays as typed, and the rest of the path (the folder
/// part, as matched, and the name) is escaped.
fn complete_path(
    prefix: &str,
    cwd: Option<&str>,
    home: Option<&str>,
    shell: ShellType,
) -> Option<String> {
    let (dir_part, _) = split_path_prefix(prefix);
    // The first match in byte order keeps the guess stable across keystrokes.
    let best = path_matches(prefix, cwd, home)
        .into_iter()
        .min_by(|a, b| a.name.cmp(&b.name))?;
    let path = format!(
        "{dir_part}{}{}",
        best.name,
        if best.is_dir { "/" } else { "" }
    );
    let candidate = CompletionCandidate {
        kept_prefix: Some(home.unwrap_or_default().to_string()),
        ..CompletionCandidate::new(path, best.name, "path", best.is_dir)
    };
    Some(candidate.insertion(prefix, shell))
}

/// The folder `dir_part` names: under the home folder after a `home`
/// prefix, else absolute, else in `cwd`. Everything else is taken
/// literally, as the shell would once it's escaped.
fn resolve_dir(dir_part: &str, cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    if let Some(rest) = home.and_then(|h| dir_part.strip_prefix(h)) {
        let home_dir = std::env::var("HOME").ok()?;
        return Some(if rest.is_empty() {
            home_dir
        } else {
            format!("{home_dir}/{rest}")
        });
    }
    if dir_part.starts_with('/') {
        return Some(dir_part.to_string());
    }
    // Relative, including the empty folder part, resolves against the cwd.
    let cwd = cwd?;
    Some(if dir_part.is_empty() {
        cwd.to_string()
    } else {
        format!("{cwd}/{dir_part}")
    })
}

pub(crate) fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

// Built-in tables

/// Common commands, ordered by rough frequency so the first prefix match is
/// usually the intended one.
pub(crate) const COMMON_COMMANDS: &[&str] = &[
    "git", "cd", "ls", "cargo", "npm", "npx", "node", "pnpm", "yarn", "python", "python3", "pip",
    "pip3", "docker", "kubectl", "make", "cmake", "go", "rustc", "rustup", "ssh", "scp", "curl",
    "wget", "grep", "rg", "fd", "find", "cat", "bat", "less", "tail", "head", "echo", "touch",
    "mkdir", "rmdir", "rm", "cp", "mv", "ln", "chmod", "chown", "tar", "zip", "unzip", "brew",
    "code", "vim", "nvim", "nano", "open", "kill", "ps", "top", "htop", "df", "du", "tree",
    "source", "export", "sudo", "man", "which", "history", "clear", "exit",
];

/// POSIX, zsh and fish builtins worth completing when nothing else matches.
const SHELL_BUILTINS: &[&str] = &[
    "alias", "bg", "bind", "builtin", "command", "declare", "dirs", "disown", "eval", "exec", "fg",
    "function", "getopts", "hash", "jobs", "let", "local", "popd", "printf", "pushd", "read",
    "readonly", "return", "set", "setenv", "test", "trap", "type", "typeset", "ulimit", "umask",
    "unalias", "unset", "unsetenv", "wait",
];

/// First-argument subcommands for popular tools, ordered by rough frequency
/// so the single inline guess is the commonly intended one.
const SUBCOMMANDS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "status",
            "add",
            "commit",
            "checkout",
            "push",
            "pull",
            "branch",
            "log",
            "diff",
            "merge",
            "fetch",
            "clone",
            "rebase",
            "reset",
            "restore",
            "stash",
            "switch",
            "show",
            "remote",
            "tag",
            "config",
            "init",
            "revert",
            "cherry-pick",
            "mv",
            "rm",
            "worktree",
        ],
    ),
    (
        "cargo",
        &[
            "build", "run", "test", "check", "clippy", "fmt", "add", "new", "init", "update",
            "doc", "clean", "bench", "fix", "install", "publish", "remove", "fetch", "tree",
        ],
    ),
    (
        "npm",
        &[
            "install",
            "run",
            "start",
            "test",
            "init",
            "ci",
            "update",
            "audit",
            "publish",
            "link",
            "ls",
            "outdated",
            "pack",
            "uninstall",
            "version",
        ],
    ),
    (
        "pnpm",
        &[
            "install", "run", "add", "start", "test", "build", "update", "exec", "dlx", "init",
            "link", "list", "outdated", "remove",
        ],
    ),
    (
        "yarn",
        &[
            "install", "add", "run", "start", "test", "build", "dev", "init", "remove", "upgrade",
        ],
    ),
    (
        "docker",
        &[
            "run", "build", "ps", "exec", "logs", "compose", "images", "pull", "push", "stop",
            "start", "rm", "rmi", "inspect", "tag", "volume",
        ],
    ),
    (
        "kubectl",
        &[
            "get",
            "describe",
            "apply",
            "logs",
            "exec",
            "delete",
            "create",
            "rollout",
            "scale",
            "config",
            "port-forward",
        ],
    ),
    (
        "brew",
        &[
            "install",
            "update",
            "upgrade",
            "list",
            "search",
            "info",
            "uninstall",
            "reinstall",
            "outdated",
            "cleanup",
            "doctor",
        ],
    ),
    (
        "rustup",
        &[
            "update",
            "default",
            "show",
            "toolchain",
            "target",
            "component",
            "override",
        ],
    ),
    (
        "go",
        &[
            "run", "build", "test", "get", "mod", "install", "vet", "clean",
        ],
    ),
];

/// Per-command flags worth completing before the generic set.
const FLAGS: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "--all",
            "--amend",
            "--force",
            "--message",
            "--no-verify",
            "--set-upstream",
        ],
    ),
    (
        "cargo",
        &[
            "--all-features",
            "--bin",
            "--features",
            "--lib",
            "--package",
            "--release",
            "--workspace",
        ],
    ),
    (
        "ls",
        &[
            "--all",
            "--almost-all",
            "--color",
            "--human-readable",
            "--long",
            "--reverse",
        ],
    ),
    (
        "rg",
        &[
            "--count",
            "--fixed-strings",
            "--glob",
            "--hidden",
            "--ignore-case",
            "--line-number",
            "--no-ignore",
        ],
    ),
    (
        "docker",
        &[
            "--detach",
            "--file",
            "--interactive",
            "--name",
            "--publish",
            "--rm",
            "--tty",
            "--volume",
        ],
    ),
];

/// Flags accepted by almost everything.
const COMMON_FLAGS: &[&str] = &["--help", "--version", "--verbose", "--quiet"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const NO_HISTORY: &[&str] = &[];

    fn complete_in(input: &str, cwd: Option<&str>) -> Option<String> {
        complete(input, cwd, NO_HISTORY, ShellType::Bash)
    }

    #[test]
    fn history_continuation_wins() {
        let history = ["git status", "cargo build"];
        assert_eq!(
            complete("git st", None, &history, ShellType::Bash).as_deref(),
            Some("git status")
        );
        // A trailing space continues the most recent matching command.
        assert_eq!(
            complete("cargo ", None, &history, ShellType::Bash).as_deref(),
            Some("cargo build")
        );
    }

    #[test]
    fn completes_the_command_word_from_the_common_list() {
        assert_eq!(complete_in("gi", None).as_deref(), Some("git"));
        assert_eq!(complete_in("car", None).as_deref(), Some("cargo"));
    }

    #[test]
    fn completes_subcommands_of_known_commands() {
        assert_eq!(complete_in("git stat", None).as_deref(), Some("git status"));
        assert_eq!(
            complete_in("cargo bui", None).as_deref(),
            Some("cargo build")
        );
    }

    #[test]
    fn completes_flags() {
        assert_eq!(complete_in("ls --al", None).as_deref(), Some("ls --all"));
        assert_eq!(
            complete_in("git --me", None).as_deref(),
            Some("git --message")
        );
        // Falls back to the common flags for unknown commands.
        assert_eq!(
            complete_in("frobnicate --he", None).as_deref(),
            Some("frobnicate --help")
        );
    }

    #[test]
    fn completes_filesystem_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("alpha")).unwrap();
        fs::write(dir.path().join("alpha/inner.txt"), "x").unwrap();
        fs::write(dir.path().join("alpha-file.txt"), "x").unwrap();
        let cwd = dir.path().to_str();
        // `alpha` (a folder) sorts before `alpha-file.txt`; folders get a slash.
        assert_eq!(complete_in("cat alph", cwd).as_deref(), Some("cat alpha/"));
        // Within a folder, the folder part is kept verbatim.
        assert_eq!(
            complete_in("cat alpha/inn", cwd).as_deref(),
            Some("cat alpha/inner.txt")
        );
    }

    #[test]
    fn path_completions_are_escaped() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("x$(touch PWNED)")).unwrap();
        fs::create_dir_all(dir.path().join("my dir")).unwrap();
        fs::write(dir.path().join("my dir/it's.txt"), "x").unwrap();
        let cwd = dir.path().to_str();
        // A name is inserted as a name, never as shell syntax.
        assert_eq!(
            complete_in("cd x", cwd).as_deref(),
            Some("cd x\\$\\(touch\\ PWNED\\)/")
        );
        assert_eq!(complete_in("cat my", cwd).as_deref(), Some("cat my\\ dir/"));
        // A token typed escaped continues as typed.
        assert_eq!(
            complete_in("cat my\\ dir/i", cwd).as_deref(),
            Some("cat my\\ dir/it\\'s.txt")
        );
        // `~/` and `$HOME/` stay unquoted so they still expand. The Swift test
        // uses ~/Library; any plainly named folder in the home folder does.
        let Some(folder) = plain_home_folder() else {
            return;
        };
        let typed = &folder[..folder.len() - 1];
        assert_eq!(
            complete_in(&format!("ls ~/{typed}"), cwd),
            Some(format!("ls ~/{folder}/"))
        );
        assert_eq!(
            complete_in(&format!("ls $HOME/{typed}"), cwd),
            Some(format!("ls $HOME/{folder}/"))
        );
        let menu = complete_candidates(&format!("ls ~/{typed}"), cwd, 5);
        assert_eq!(
            menu.candidates[0].insertion(&format!("~/{typed}"), ShellType::Bash),
            format!("~/{folder}/")
        );
    }

    /// A folder in the home folder whose name needs no escaping and that
    /// comes first among the entries starting with it minus its last letter.
    fn plain_home_folder() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let names: Vec<(String, bool)> = fs::read_dir(&home)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                Some((name, e.file_type().ok()?.is_dir()))
            })
            .collect();
        names
            .iter()
            .filter(|(name, is_dir)| {
                *is_dir
                    && name.len() >= 2
                    && !name.starts_with('.')
                    && name.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .find(|(name, _)| {
                let typed = &name[..name.len() - 1];
                !names
                    .iter()
                    .any(|(other, _)| other.starts_with(typed) && other < name)
            })
            .map(|(name, _)| name.clone())
    }

    #[test]
    fn literal_names_are_never_expanded() {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["~/inner", "$HOME"] {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        for file in ["~$Report.docx", "a\\b"] {
            fs::write(dir.path().join(file), "x").unwrap();
        }
        let cwd = dir.path().to_str();
        let menu = |input: &str| -> Vec<String> {
            let result = complete_candidates(input, cwd, 10);
            let typed = &input[result.span.start..result.span.end];
            result
                .candidates
                .iter()
                .map(|c| c.insertion(typed, ShellType::Bash))
                .collect()
        };
        // A folder named `~` in this folder isn't the home folder: `rm -rf ~/`
        // would be. Escaped, it no longer continues what was typed, so there's
        // no ghost text, and the menu inserts it escaped.
        assert_eq!(complete_in("rm -rf ~", cwd), None);
        assert_eq!(menu("rm -rf ~"), ["\\~/", "\\~\\$Report.docx"]);
        assert_eq!(complete_in("ls $H", cwd), None);
        assert_eq!(menu("ls $H"), ["\\$HOME/"]);
        assert_eq!(complete_in("ls ~$", cwd), None);
        assert_eq!(menu("ls ~$"), ["\\~\\$Report.docx"]);
        // Typed escaped, `~/` is that folder, and stays escaped.
        assert_eq!(
            complete_in("ls \\~/in", cwd).as_deref(),
            Some("ls \\~/inner/")
        );
        // A backslash in a name keeps its escape.
        assert_eq!(complete_in("cat a\\\\", cwd).as_deref(), Some("cat a\\\\b"));
        assert_eq!(menu("cat a"), ["a\\\\b"]);
    }

    #[test]
    fn no_completion_for_empty_or_blank_input() {
        assert_eq!(complete_in("", None), None);
        assert_eq!(complete_in("   ", None), None);
    }

    #[test]
    fn skips_incomplete_quoted_tokens() {
        assert_eq!(complete_in("cat \"unterminated", None), None);
    }

    // The menu's path candidates

    /// A temporary folder laid out like a WordPress project root.
    fn wp_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["wp-content", "wp-admin", "wp-includes"] {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        fs::write(dir.path().join("readme.md"), "x").unwrap();
        dir
    }

    fn values(result: &CompletionResult) -> Vec<&str> {
        result.candidates.iter().map(|c| c.value.as_str()).collect()
    }

    #[test]
    fn candidates_list_matching_folders_first() {
        let dir = wp_fixture();
        let result = complete_candidates("cd wp-", dir.path().to_str(), 50);
        // Three folders, in order, each ending with `/`.
        assert_eq!(
            values(&result),
            ["wp-admin/", "wp-content/", "wp-includes/"]
        );
        assert!(result
            .candidates
            .iter()
            .all(|c| c.is_dir && c.kind == "path"));
        assert_eq!(result.candidates[0].display, "wp-admin");
        // The span covers exactly the `wp-` token.
        assert_eq!(result.span, TextSpan { start: 3, end: 6 });
    }

    #[test]
    fn candidates_for_an_empty_prefix_list_everything() {
        let dir = wp_fixture();
        let result = complete_candidates("cd ", dir.path().to_str(), 50);
        assert_eq!(
            values(&result),
            ["wp-admin/", "wp-content/", "wp-includes/", "readme.md"]
        );
    }

    #[test]
    fn hidden_candidates_only_after_a_dot() {
        let dir = wp_fixture();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let plain = complete_candidates("cd wp-", dir.path().to_str(), 50);
        assert!(plain.candidates.iter().all(|c| c.display != ".git"));
        let dotted = complete_candidates("cd .", dir.path().to_str(), 50);
        assert!(dotted.candidates.iter().any(|c| c.display == ".git"));
    }

    #[test]
    fn no_candidates_for_the_command_word() {
        let dir = wp_fixture();
        let result = complete_candidates("gi", dir.path().to_str(), 50);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn candidates_respect_the_limit() {
        let dir = wp_fixture();
        let result = complete_candidates("cd wp-", dir.path().to_str(), 2);
        assert_eq!(values(&result), ["wp-admin/", "wp-content/"]);
    }

    #[test]
    fn candidates_serialize_to_the_contract_json() {
        let dir = wp_fixture();
        let result = complete_candidates("cd wp-c", dir.path().to_str(), 50);
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["span"]["start"], 3);
        assert_eq!(json["span"]["end"], 7);
        let first = &json["candidates"][0];
        assert_eq!(first["value"], "wp-content/");
        assert_eq!(first["display"], "wp-content");
        assert_eq!(first["kind"], "path");
        assert_eq!(first["is_dir"], true);
        assert!(first["git_status"].is_null());
    }
}
