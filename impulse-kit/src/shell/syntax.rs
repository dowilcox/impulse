//! Whether a command parses before it's typed at the shell's prompt.
//!
//! fish keeps a command it can't parse in its command line rather than
//! running it, and fires none of the events the integration reports, so the
//! error lands under a prompt the input bar hides and whatever is typed next
//! is added to the leftover (a pasted bash script, an `if … then … fi` fish
//! waits to see an `end` for). Checked first with `fish --no-execute`,
//! such a command stays in the bar with fish's error instead. bash and zsh
//! report their own errors at a fresh prompt, so they aren't checked.

use std::time::Duration;

use super::ShellType;
use crate::child_process::ChildProcess;

/// A parse takes a few milliseconds; past this the command just runs.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The shell can run it (or it couldn't be checked).
    Parses,
    Rejected {
        /// The shell's own message, as it prints it: what's wrong, the
        /// line and a caret under the spot.
        message: String,
        /// Byte offset in the command of the spot the caret marks.
        at: Option<usize>,
        /// bash can parse it (a bash script typed at fish).
        bash_parses: bool,
    },
}

/// Ask the shell at `shell_path` whether it can parse `command`, without
/// running any of it. Blocks: call it off the main thread.
pub fn check(shell: ShellType, shell_path: &str, command: &str) -> Verdict {
    if shell != ShellType::Fish {
        return Verdict::Parses;
    }
    let Some(message) = fish_error(shell_path, command) else {
        return Verdict::Parses;
    };
    Verdict::Rejected {
        at: error_offset(command, &message),
        bash_parses: bash_parses(command),
        message,
    }
}

/// fish's message for `command`, or `None` when it parses (or fish
/// couldn't be asked: the command then runs as it always did).
fn fish_error(shell_path: &str, command: &str) -> Option<String> {
    let output = ChildProcess::new(shell_path)
        .args(["--no-config", "--no-execute"])
        .stdin(command.as_bytes().to_vec())
        .timeout(CHECK_TIMEOUT)
        .run()
        .ok()?;
    if output.status == 0 || output.timed_out {
        return None;
    }
    let message = String::from_utf8_lossy(&output.stderr)
        .trim_end()
        .to_string();
    (!message.is_empty()).then_some(message)
}

fn bash_parses(command: &str) -> bool {
    ChildProcess::new("bash")
        .arg("-n")
        .stdin(command.as_bytes().to_vec())
        .timeout(CHECK_TIMEOUT)
        .run()
        .is_ok_and(|output| output.status == 0 && !output.timed_out)
}

/// Where in `command` fish's message points: its second line is the line
/// with the error, as written, and its third a caret under the spot.
pub fn error_offset(command: &str, message: &str) -> Option<usize> {
    let mut lines = message.lines().skip(1);
    let source = lines.next()?;
    let caret = lines.next().unwrap_or_default();
    let column = caret.chars().position(|c| c != ' ').unwrap_or(0);
    let mut start = 0;
    let mut fallback = None;
    for line in command.split('\n') {
        if line == source {
            let within = line
                .char_indices()
                .nth(column)
                .map_or(line.len(), |(i, _)| i);
            return Some(start + within);
        }
        if fallback.is_none() && line.trim() == source.trim() {
            fallback = Some(start + (line.len() - line.trim_start().len()));
        }
        start += line.len() + 1;
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_the_caret_on_the_quoted_line() {
        let script = "#!/bin/bash\nset -euo pipefail\nNAME=world\necho $NAME\n";
        let message = "fish: Unsupported use of '='. In fish, please use 'set NAME world'.\n\
                       NAME=world\n\
                       ^~~~~~~~~^";
        assert_eq!(
            error_offset(script, message),
            Some(script.find("NAME=").unwrap())
        );
        let message = "fish: Unexpected end of string, quotes are not balanced\n\
                       set x (echo \"unterminated\n            ^";
        let script = "echo ok\nset x (echo \"unterminated\n";
        assert_eq!(
            error_offset(script, message),
            Some(script.find("\"unterminated").unwrap())
        );
    }

    #[test]
    fn an_indented_line_is_found_by_its_text() {
        let script = "if true\n    echo (\nend";
        let message = "fish: Unexpected end of string\necho (\n     ^";
        assert_eq!(
            error_offset(script, message),
            Some(script.find("echo").unwrap())
        );
        assert_eq!(error_offset(script, "fish: something"), None);
        assert_eq!(error_offset(script, "fish: x\nnot there\n^"), None);
    }

    #[test]
    fn only_fish_is_checked() {
        assert_eq!(
            check(ShellType::Bash, "/nonexistent", "if"),
            Verdict::Parses
        );
        assert_eq!(check(ShellType::Zsh, "/nonexistent", "if"), Verdict::Parses);
        // A fish that can't be started lets the command through.
        assert_eq!(
            check(ShellType::Fish, "/nonexistent/fish", "if"),
            Verdict::Parses
        );
    }

    /// Against the real fish when it's installed.
    #[test]
    fn fish_rejects_bash_and_unclosed_blocks() {
        let Some(fish) = [
            "/usr/bin/fish",
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists()) else {
            return;
        };
        assert_eq!(
            check(ShellType::Fish, fish, "echo one\necho two"),
            Verdict::Parses
        );
        assert_eq!(
            check(ShellType::Fish, fish, "export FOO=bar"),
            Verdict::Parses
        );
        let script = "NAME=world\necho $NAME";
        match check(ShellType::Fish, fish, script) {
            Verdict::Rejected { message, at, .. } => {
                assert!(message.contains("NAME=world"), "{message}");
                assert_eq!(at, Some(0));
            }
            other => panic!("{other:?}"),
        }
        // Run with bash: the script quoted for fish, as one argument.
        let quoted =
            super::super::quoting::quoted("NAME='a b'\necho \"$NAME\" \\ $1", ShellType::Fish);
        let bash = format!("bash -c {quoted}");
        assert_eq!(check(ShellType::Fish, fish, &bash), Verdict::Parses);
        match check(
            ShellType::Fish,
            fish,
            "if [ -d /tmp ]; then\n  echo yes\nfi",
        ) {
            Verdict::Rejected {
                message,
                bash_parses,
                ..
            } => {
                assert!(message.contains("Missing end"), "{message}");
                assert!(bash_parses);
            }
            other => panic!("{other:?}"),
        }
    }
}
