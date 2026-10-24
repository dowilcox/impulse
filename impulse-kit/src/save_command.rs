//! The programs Impulse runs after saving a file, a file type's formatter
//! and the commands on save (the Swift `SaveCommand`): their arguments, a
//! one-line reason when one fails, and what the editor does with the file
//! once one has run.

/// The placeholder replaced with the saved file's path in arguments.
pub const FILE_PLACEHOLDER: &str = "{file}";

/// Failure summaries longer than this many characters are cut.
pub const MAX_SUMMARY_LENGTH: usize = 200;

/// `args` with every `{file}` replaced by `path` (also inside a longer
/// argument, such as `--stdin-filepath={file}`).
pub fn expand_arguments(args: &[String], path: &str) -> Vec<String> {
    args.iter()
        .map(|arg| arg.replace(FILE_PLACEHOLDER, path))
        .collect()
}

fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Why a run failed, in one line: the first non-blank line of its error
/// output, else of its output, else its exit status. Long lines are cut.
pub fn failure_summary(status: i32, stdout: &[u8], stderr: &[u8], timed_out: bool) -> String {
    if timed_out {
        return "It didn't finish in time and was stopped.".into();
    }
    for data in [stderr, stdout] {
        let text = String::from_utf8_lossy(data);
        let line = text
            .split(is_newline)
            .map(|l| l.trim_matches(|c: char| c.is_whitespace() && !is_newline(c)))
            .find(|l| !l.is_empty());
        if let Some(line) = line {
            if line.chars().count() > MAX_SUMMARY_LENGTH {
                let cut: String = line.chars().take(MAX_SUMMARY_LENGTH - 1).collect();
                return cut + "…";
            }
            return line.to_string();
        }
    }
    format!("It exited with status {status}.")
}

/// Whether the editor should show what a formatter or command on save left
/// in the file (`rewritten`) in place of what it saved: only when it
/// succeeded, and not when it emptied a file that had text (a tool that
/// died mid-write, or printed its result instead of writing it).
pub fn should_adopt(rewritten: &str, saved: &str, succeeded: bool) -> bool {
    if !succeeded {
        return false;
    }
    let blank = |text: &str| text.chars().all(char::is_whitespace);
    !blank(rewritten) || blank(saved)
}

/// What the editor does once a formatter or command on save finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteOutcome {
    /// Show the file's new text as the buffer (one undoable edit).
    Adopt,
    /// The file holds what was saved, or the command's good result:
    /// remember it as what's on disk. `dirty` when the buffer isn't that.
    Record { dirty: bool },
    /// The file holds something nobody can vouch for: what a failed or
    /// emptying run left, or another program's write while it ran. Keep
    /// what the save knew about the disk, so the next save asks before
    /// replacing it; the buffer isn't what's on disk.
    KeepSavedState,
}

/// `saved` is what the save wrote, `disk` what the file holds now and
/// `buffer` what the editor holds.
pub fn rewrite_outcome(saved: &str, disk: &str, buffer: &str, succeeded: bool) -> RewriteOutcome {
    if disk == saved {
        return RewriteOutcome::Record {
            dirty: buffer != disk,
        };
    }
    if !should_adopt(disk, saved, succeeded) {
        return RewriteOutcome::KeepSavedState;
    }
    // Typing since the save is kept (unsaved) instead of the result.
    if buffer == saved {
        RewriteOutcome::Adopt
    } else {
        RewriteOutcome::Record {
            dirty: buffer != disk,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RewriteOutcome::*;

    #[test]
    fn file_placeholder_is_replaced_everywhere() {
        let args: Vec<String> = [
            "--write",
            "{file}",
            "--stdin-filepath={file}",
            "--file",
            ".",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            expand_arguments(&args, "/tmp/a b.ts"),
            vec![
                "--write",
                "/tmp/a b.ts",
                "--stdin-filepath=/tmp/a b.ts",
                "--file",
                "."
            ]
        );
        assert!(expand_arguments(&[], "/x").is_empty());
    }

    #[test]
    fn failure_summary_prefers_the_first_error_line() {
        let summary = failure_summary(
            2,
            b"progress\n",
            b"\n  \n  error: unexpected token  \nat line 3\n",
            false,
        );
        assert_eq!(summary, "error: unexpected token");
    }

    #[test]
    fn failure_summary_falls_back_to_output_then_status() {
        assert_eq!(
            failure_summary(1, b"Found 2 problems\n", b"", false),
            "Found 2 problems"
        );
        assert_eq!(
            failure_summary(127, b"", b" \n", false),
            "It exited with status 127."
        );
        assert_eq!(
            failure_summary(143, b"", b"x", true),
            "It didn't finish in time and was stopped."
        );
    }

    #[test]
    fn long_lines_are_cut() {
        let line = "e".repeat(500);
        let summary = failure_summary(1, b"", line.as_bytes(), false);
        assert_eq!(summary.chars().count(), MAX_SUMMARY_LENGTH);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn only_a_successful_non_empty_rewrite_is_adopted() {
        assert!(should_adopt("let a = 1\n", "let a=1", true));
        // Failed or stopped: whatever it left behind isn't shown.
        assert!(!should_adopt("let a", "let a=1", false));
        // Emptied a file that had text.
        assert!(!should_adopt("", "let a=1", true));
        assert!(!should_adopt(" \n\n", "let a=1", true));
        // A blank file may stay blank.
        assert!(should_adopt("", "\n", true));
    }

    #[test]
    fn a_good_result_is_shown_unless_typing_moved_on() {
        assert_eq!(rewrite_outcome("a=1", "a = 1", "a=1", true), Adopt);
        assert_eq!(
            rewrite_outcome("a=1", "a = 1", "a=12", true),
            Record { dirty: true }
        );
    }

    #[test]
    fn untouched_file_is_recorded_and_only_newer_typing_is_dirty() {
        assert_eq!(
            rewrite_outcome("x", "x", "x", true),
            Record { dirty: false }
        );
        assert_eq!(
            rewrite_outcome("x", "x", "x", false),
            Record { dirty: false }
        );
        assert_eq!(
            rewrite_outcome("x", "x", "xy", true),
            Record { dirty: true }
        );
    }

    #[test]
    fn formatter_then_reloading_command_leaves_a_clean_buffer_clean() {
        // The formatter's result F is shown; the command's "saved" may still
        // be the text from before formatting, and it changed nothing.
        assert_eq!(
            rewrite_outcome("S", "F", "F", true),
            Record { dirty: false }
        );
    }

    #[test]
    fn a_failed_runs_leftovers_keep_the_saved_disk_state() {
        // Whether the formatter half-wrote the file or another program wrote
        // to it meanwhile, it isn't taken as known: the next save asks.
        assert_eq!(
            rewrite_outcome("a=1", "written by an agent", "a=1", false),
            KeepSavedState
        );
        assert_eq!(rewrite_outcome("a=1", "", "a=1", true), KeepSavedState);
    }
}
