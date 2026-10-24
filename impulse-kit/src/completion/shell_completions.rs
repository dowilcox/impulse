//! The shell's own completions (opt-in; the Swift `ShellCompletions`):
//! fish's `complete -C` answers with everything its completion scripts
//! know, one `value<TAB>description` per line. bash and zsh have no
//! equivalent that works outside an interactive session, so they keep the
//! built-in specs.

use std::collections::HashSet;
use std::time::Duration;

use super::CompletionCandidate;
use crate::child_process::ChildProcess;

/// How long fish gets to answer by default.
pub const FISH_TIMEOUT: Duration = Duration::from_millis(1500);

/// Whether `shell_path` is a shell this can ask.
pub fn supports(shell_path: &str) -> bool {
    shell_path.rsplit('/').next() == Some("fish")
}

/// Ask fish to complete `line` (the text up to the cursor) in `cwd`. Empty
/// after `timeout` or on any failure.
pub fn fish(
    line: &str,
    cwd: Option<&str>,
    fish_path: &str,
    timeout: Duration,
) -> Vec<CompletionCandidate> {
    // The line goes in an environment variable, so it's never parsed as
    // code. A timeout stops the whole process group, so a completion
    // script's helper that hangs can't outlive the request.
    let mut process = ChildProcess::new(fish_path)
        .args(["-c", "complete -C -- \"$IMPULSE_COMPLETE_LINE\""])
        .env("IMPULSE_COMPLETE_LINE", line)
        .timeout(timeout);
    if let Some(cwd) = cwd {
        process = process.dir(cwd);
    }
    match process.run() {
        Ok(result) if !result.timed_out => parse_fish(&result.stdout_text(), 200),
        _ => Vec::new(),
    }
}

/// `value<TAB>description` lines to candidates (kind `"shell"`; a trailing
/// `/` marks a directory, kind `"path"`), without duplicates, at most
/// `limit`.
pub fn parse_fish(output: &str, limit: usize) -> Vec<CompletionCandidate> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in output.split('\n').filter(|l| !l.is_empty()) {
        let (value, detail) = match line.split_once('\t') {
            Some((value, detail)) => (value, Some(detail.trim())),
            None => (line, None),
        };
        if value.is_empty() || !seen.insert(value) {
            continue;
        }
        let is_dir = value.ends_with('/');
        let display = if value.contains('/') {
            let trimmed = value.strip_suffix('/').unwrap_or(value);
            let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
            format!("{name}{}", if is_dir { "/" } else { "" })
        } else {
            value.to_string()
        };
        out.push(CompletionCandidate {
            value: value.to_string(),
            display,
            kind: if is_dir { "path" } else { "shell" }.to_string(),
            is_dir,
            git_status: None,
            detail: detail.filter(|d| !d.is_empty()).map(String::from),
            kept_prefix: None,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// `primary` first (the built-in specs), then what only the shell knew.
pub fn merge(
    primary: Vec<CompletionCandidate>,
    shell: Vec<CompletionCandidate>,
    limit: usize,
) -> Vec<CompletionCandidate> {
    let mut seen: HashSet<String> = primary.iter().map(|c| c.value.clone()).collect();
    let mut out = primary;
    out.extend(shell.into_iter().filter(|c| seen.insert(c.value.clone())));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::engine::{candidates, CompletionContext};
    use std::path::Path;

    #[test]
    fn parses_fish_output() {
        let out = parse_fish(
            "checkout\tCheckout and switch to a branch\ncherry-pick\tReapply a commit\nsrc/\nsrc/lib.rs\t\ncheckout\tdup\n",
            200,
        );
        let values: Vec<&str> = out.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["checkout", "cherry-pick", "src/", "src/lib.rs"]);
        assert_eq!(
            out[0].detail.as_deref(),
            Some("Checkout and switch to a branch")
        );
        assert_eq!(out[0].kind, "shell");
        assert!(out[2].is_dir && out[2].kind == "path" && out[2].display == "src/");
        assert!(out[3].display == "lib.rs" && out[3].detail.is_none());
    }

    #[test]
    fn merges_after_the_built_in_ones() {
        let shell = |_: &str| parse_fish("commit\tRecord changes\ncherry\tFind commits\n", 200);
        let context = CompletionContext::new(None).shell_completions(shell);
        let result = candidates("git c", &context, 50);
        let values: Vec<&str> = result.candidates.iter().map(|c| c.value.as_str()).collect();
        assert!(values.contains(&"commit") && values.contains(&"cherry"));
        assert_eq!(
            values.iter().filter(|v| **v == "commit").count(),
            1,
            "no duplicates"
        );
        let position = |value: &str| values.iter().position(|v| *v == value).unwrap();
        assert!(position("cherry") > position("commit"), "built-in first");
    }

    #[test]
    fn fish_completes_for_real() {
        // Only where fish is installed.
        let Some(fish_path) = [
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
            "/usr/bin/fish",
        ]
        .into_iter()
        .find(|p| Path::new(p).exists()) else {
            return;
        };
        let out = fish("set --erase\"; echo; ", Some("/"), fish_path, FISH_TIMEOUT);
        assert!(
            out.iter().all(|c| !c.value.contains('\n')),
            "the line is data, not code"
        );
        let out = fish("cd /us", Some("/"), fish_path, FISH_TIMEOUT);
        assert!(out.iter().any(|c| c.value == "/usr/"));
    }
}
