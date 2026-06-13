//! Inline command completion for the terminal input bar.
//!
//! Given the current input and working directory, [`complete`] returns the
//! single best completed line to show as dimmed ghost text. Sources, in
//! priority order:
//!
//! 1. Command history — the most recent command that extends the full input
//!    (Warp-style autosuggest).
//! 2. Context-aware word completion of the token at the cursor, driven by
//!    [`crate::shell_parser`]: executables on `PATH` (plus a curated common-
//!    command list and shell builtins) for the command word, a built-in
//!    subcommand/flag table for popular tools, and filesystem entries for
//!    path arguments.
//!
//! The result is the full completed line; the caller renders the suffix after
//! what the user has already typed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::shell_parser::{parse_shell_input, ShellCompletionKind, TextSpan};

/// Compute the best inline completion for `input`.
///
/// `history` should be ordered newest-first. Returns the full completed line
/// (which always starts with `input`), or `None` when there's no useful
/// completion.
pub fn complete(input: &str, cwd: Option<&str>, history: &[String]) -> Option<String> {
    if input.is_empty() || input.chars().all(char::is_whitespace) {
        return None;
    }

    // 1. History continuation — autosuggest the most recent command that
    //    extends the full input verbatim.
    if let Some(found) = history
        .iter()
        .find(|cmd| cmd.len() > input.len() && cmd.starts_with(input))
    {
        return Some(found.clone());
    }

    // 2. Context-aware word completion.
    let parsed = parse_shell_input(input, input.len());
    // Splicing raw candidates back into quoted/escaped tokens is ambiguous;
    // history already covers those cases, so bail out.
    if parsed.incomplete {
        return None;
    }
    let comp = &parsed.completion;
    if comp.prefix.is_empty() {
        return None;
    }

    let cwd_path = cwd.filter(|value| !value.is_empty()).map(Path::new);
    let candidate = match comp.kind {
        ShellCompletionKind::Command => complete_command(&comp.prefix),
        ShellCompletionKind::RedirectTarget => complete_path(&comp.prefix, cwd_path),
        ShellCompletionKind::EnvAssignment => None,
        ShellCompletionKind::Argument => complete_argument(
            comp.command.as_deref(),
            comp.argument_index,
            &comp.prefix,
            cwd_path,
        ),
    }?;

    splice(input, comp.span, &candidate)
}

/// Eagerly populate the `PATH` executable cache off the hot path, so the first
/// completion keystroke doesn't pay for the directory scan. Safe to call from
/// a background thread.
pub fn warm_cache() {
    let _ = path_executables();
}

/// Replace the completion token in `input` with `candidate`, keeping the text
/// before it verbatim. Returns `None` unless the result genuinely extends what
/// was typed (so the ghost suffix stays consistent).
fn splice(input: &str, span: TextSpan, candidate: &str) -> Option<String> {
    let start = span.start.min(input.len());
    let mut result = String::with_capacity(start + candidate.len());
    result.push_str(&input[..start]);
    result.push_str(candidate);
    if result.len() > input.len() && result.starts_with(input) {
        Some(result)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Command-word completion
// ---------------------------------------------------------------------------

fn complete_command(prefix: &str) -> Option<String> {
    // A curated common command makes the single guess stable and useful
    // (e.g. `g` → `git`, not `gpg`); the list is ordered by commonness.
    if let Some(common) = COMMON_COMMANDS.iter().find(|cmd| cmd.starts_with(prefix)) {
        return Some((*common).to_string());
    }
    if let Some(builtin) = SHELL_BUILTINS.iter().find(|cmd| cmd.starts_with(prefix)) {
        return Some((*builtin).to_string());
    }
    // Otherwise the shortest matching executable on PATH is a sensible default.
    path_executables()
        .iter()
        .filter(|exe| exe.starts_with(prefix))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .cloned()
}

fn path_executables() -> &'static Vec<String> {
    // Cached once per process — installing a new tool mid-session won't
    // autocomplete until restart, which is an acceptable trade for not
    // rescanning PATH on every keystroke.
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(scan_path_executables)
}

fn scan_path_executables() -> Vec<String> {
    let mut names = HashSet::new();
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !is_executable_entry(&entry) {
                continue;
            }
            if let Ok(name) = entry.file_name().into_string() {
                names.insert(name);
            }
        }
    }
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort();
    names
}

#[cfg(unix)]
fn is_executable_entry(entry: &std::fs::DirEntry) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = entry.metadata() else {
        return false;
    };
    (meta.is_file() || meta.file_type().is_symlink()) && meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_entry(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().map(|t| t.is_file()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Argument completion (subcommands, flags, filesystem paths)
// ---------------------------------------------------------------------------

fn complete_argument(
    command: Option<&str>,
    argument_index: usize,
    prefix: &str,
    cwd: Option<&Path>,
) -> Option<String> {
    if prefix.starts_with('-') {
        return complete_flag(command, prefix);
    }
    // The first argument after a known command is usually a subcommand.
    if argument_index == 0 {
        if let Some(command) = command {
            if let Some(sub) = complete_subcommand(command, prefix) {
                return Some(sub);
            }
        }
    }
    complete_path(prefix, cwd)
}

fn complete_subcommand(command: &str, prefix: &str) -> Option<String> {
    let base = command_basename(command);
    let subs = SUBCOMMANDS.iter().find(|(name, _)| *name == base)?.1;
    subs.iter()
        .find(|sub| sub.starts_with(prefix))
        .map(|sub| (*sub).to_string())
}

fn complete_flag(command: Option<&str>, prefix: &str) -> Option<String> {
    if let Some(command) = command.map(command_basename) {
        if let Some((_, flags)) = FLAGS.iter().find(|(name, _)| *name == command) {
            if let Some(flag) = flags.iter().find(|flag| flag.starts_with(prefix)) {
                return Some((*flag).to_string());
            }
        }
    }
    COMMON_FLAGS
        .iter()
        .find(|flag| flag.starts_with(prefix))
        .map(|flag| (*flag).to_string())
}

fn complete_path(prefix: &str, cwd: Option<&Path>) -> Option<String> {
    // Keep the directory part exactly as typed; match only the trailing name.
    let (dir_part, base) = match prefix.rfind('/') {
        Some(index) => (&prefix[..=index], &prefix[index + 1..]),
        None => ("", prefix),
    };

    let search_dir = resolve_dir(dir_part, cwd)?;
    let entries = std::fs::read_dir(&search_dir).ok()?;

    let mut best: Option<(String, bool)> = None;
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Hidden entries only surface when the user typed a leading dot.
        if name.starts_with('.') && !base.starts_with('.') {
            continue;
        }
        if !name.starts_with(base) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        // Alphabetically-first match keeps the guess stable across keystrokes.
        if best.as_ref().map(|(b, _)| name < *b).unwrap_or(true) {
            best = Some((name, is_dir));
        }
    }

    let (name, is_dir) = best?;
    let mut candidate = format!("{dir_part}{name}");
    if is_dir {
        candidate.push('/');
    }
    Some(candidate)
}

fn resolve_dir(dir_part: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = dir_part.strip_prefix("~/") {
        let home = std::env::var_os("HOME")?;
        return Some(PathBuf::from(home).join(rest));
    }
    if dir_part == "~" || dir_part == "~/" {
        return std::env::var_os("HOME").map(PathBuf::from);
    }
    let path = Path::new(dir_part);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        // Relative (including the empty dir part) resolves against the cwd.
        cwd.map(|cwd| cwd.join(path))
    }
}

fn command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

// ---------------------------------------------------------------------------
// Built-in tables
// ---------------------------------------------------------------------------

/// Common commands, ordered by rough frequency so the first prefix match is
/// usually the intended one.
const COMMON_COMMANDS: &[&str] = &[
    "git", "cd", "ls", "cargo", "npm", "npx", "node", "pnpm", "yarn", "python", "python3", "pip",
    "pip3", "docker", "kubectl", "make", "cmake", "go", "rustc", "rustup", "ssh", "scp", "curl",
    "wget", "grep", "rg", "fd", "find", "cat", "bat", "less", "tail", "head", "echo", "touch",
    "mkdir", "rmdir", "rm", "cp", "mv", "ln", "chmod", "chown", "tar", "zip", "unzip", "brew",
    "code", "vim", "nvim", "nano", "open", "kill", "ps", "top", "htop", "df", "du", "tree",
    "source", "export", "sudo", "man", "which", "history", "clear", "exit",
];

/// POSIX/zsh/fish shell builtins worth completing when nothing else matches.
const SHELL_BUILTINS: &[&str] = &[
    "alias", "bg", "bind", "builtin", "command", "declare", "dirs", "disown", "eval", "exec", "fg",
    "function", "getopts", "hash", "jobs", "let", "local", "popd", "printf", "pushd", "read",
    "readonly", "return", "set", "setenv", "test", "trap", "type", "typeset", "ulimit", "umask",
    "unalias", "unset", "unsetenv", "wait",
];

/// First-argument subcommands for popular tools, ordered by rough frequency so
/// the single inline guess is the commonly-intended one.
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

    fn history(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn history_continuation_wins() {
        let hist = history(&["git status", "cargo build"]);
        assert_eq!(
            complete("git st", None, &hist).as_deref(),
            Some("git status")
        );
        // Trailing space continues the most recent matching command.
        assert_eq!(
            complete("cargo ", None, &hist).as_deref(),
            Some("cargo build")
        );
    }

    #[test]
    fn completes_command_word_from_common_list() {
        assert_eq!(complete("gi", None, &[]).as_deref(), Some("git"));
        assert_eq!(complete("car", None, &[]).as_deref(), Some("cargo"));
    }

    #[test]
    fn completes_subcommand_for_known_command() {
        assert_eq!(
            complete("git stat", None, &[]).as_deref(),
            Some("git status")
        );
        assert_eq!(
            complete("cargo bui", None, &[]).as_deref(),
            Some("cargo build")
        );
    }

    #[test]
    fn completes_flags() {
        assert_eq!(complete("ls --al", None, &[]).as_deref(), Some("ls --all"));
        assert_eq!(
            complete("git --me", None, &[]).as_deref(),
            Some("git --message")
        );
        // Falls back to the common flag set for unknown commands.
        assert_eq!(
            complete("frobnicate --he", None, &[]).as_deref(),
            Some("frobnicate --help")
        );
    }

    #[test]
    fn completes_filesystem_paths() {
        let dir = std::env::temp_dir().join("impulse-completion-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("alpha")).unwrap();
        std::fs::write(dir.join("alpha").join("inner.txt"), b"x").unwrap();
        std::fs::write(dir.join("alpha-file.txt"), b"x").unwrap();

        let cwd = dir.to_str().unwrap();
        // `alpha` (dir) sorts before `alpha-file.txt`; directories get a slash.
        assert_eq!(
            complete("cat alph", Some(cwd), &[]).as_deref(),
            Some("cat alpha/")
        );
        // Within a subdirectory, the directory part is preserved verbatim.
        assert_eq!(
            complete("cat alpha/inn", Some(cwd), &[]).as_deref(),
            Some("cat alpha/inner.txt")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_completion_for_empty_or_blank_input() {
        assert_eq!(complete("", None, &[]), None);
        assert_eq!(complete("   ", None, &[]), None);
    }

    #[test]
    fn skips_incomplete_quoted_tokens() {
        assert_eq!(complete("cat \"unterminated", None, &[]), None);
    }
}
