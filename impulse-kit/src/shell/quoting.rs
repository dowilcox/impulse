//! Quoting for text Impulse types into a terminal on the user's behalf (the
//! Swift `ShellQuoting`): file names from completion, pasted or dropped
//! paths, shortcut arguments. Each reaches the program as one literal word
//! in the shell that's running.
//!
//! Control characters (C0, DEL and C1, except tab and newline) are never
//! typed raw, where a line editor would act on them as keys (^U clears the
//! line, ^M runs it); they're written as escapes the shell turns back into
//! the character.

use super::ShellType;

/// `text` as one word for `shell`: as is when every character is safe bare,
/// otherwise in single quotes. sh, bash and zsh close the quotes around a
/// `'` (`'\''`); fish reads `\\` and `\'` as escapes inside single quotes,
/// so those two get a backslash. Control characters other than tab and
/// newline go outside the quotes, written as escapes.
pub fn quoted(text: &str, shell: ShellType) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    if text.chars().all(is_safe) {
        return text.to_string();
    }
    let mut out = String::new();
    let mut run = String::new();
    let flush = |out: &mut String, run: &mut String| {
        if run.is_empty() {
            return;
        }
        match shell {
            ShellType::Fish => {
                let literal = run.replace('\\', "\\\\").replace('\'', "\\'");
                out.push_str(&format!("'{literal}'"));
            }
            ShellType::Bash | ShellType::Zsh => {
                out.push_str(&format!("'{}'", run.replace('\'', "'\\''")));
            }
        }
        run.clear();
    };
    for c in text.chars() {
        if is_encoded(c) {
            flush(&mut out, &mut run);
            out.push_str(&encoded_control(c, shell));
        } else {
            run.push(c);
        }
    }
    flush(&mut out, &mut run);
    out
}

/// `text` with a backslash before each character a shell reads specially,
/// the way the shells' own Tab completion inserts a name (`my\ file`), so
/// what was typed before it still reads the same. Newlines and tabs go in
/// single quotes instead (a backslash before a newline continues the line),
/// and other control characters as escapes for `shell`.
pub fn escaped(text: &str, shell: ShellType) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\n' || c == '\t' {
            out.push('\'');
            out.push(c);
            out.push('\'');
        } else if is_encoded(c) {
            out.push_str(&encoded_control(c, shell));
        } else {
            if !is_safe(c) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

/// What replaces the token `typed` (as the user typed it, up to the cursor)
/// for a completion the shell itself came up with (fish's `complete -C`),
/// which arrives as the shell would print it: unescaped, with a leading
/// `~/` or `$NAME/` left as typed. That head stays as typed (so it still
/// expands) and the rest is escaped. A variable name the shell completed
/// (`$HO` to `$HOME`) goes in as is.
pub fn completion_text(value: &str, typed: &str, shell: ShellType) -> String {
    if is_variable(typed, true) && is_variable(value, false) && value.starts_with(typed) {
        return value.to_string();
    }
    let head = expansion_head(typed);
    if !head.is_empty() {
        if let Some(rest) = value.strip_prefix(head) {
            return format!("{head}{}", escaped(rest, shell));
        }
    }
    escaped(value, shell)
}

/// A leading `~/` or `$NAME/` of `typed`, which the shell expands (empty
/// when there's none).
pub fn expansion_head(typed: &str) -> &str {
    if typed.starts_with("~/") {
        return "~/";
    }
    if !typed.starts_with('$') {
        return "";
    }
    let Some(slash) = typed.find('/') else {
        return "";
    };
    if is_variable(&typed[..slash], false) {
        &typed[..=slash]
    } else {
        ""
    }
}

/// `$NAME`: a dollar sign and a variable name (ASCII letters, digits and
/// `_`, not starting with a digit).
fn is_variable(text: &str, allow_empty_name: bool) -> bool {
    let Some(name) = text.strip_prefix('$') else {
        return false;
    };
    let Some(first) = name.chars().next() else {
        return allow_empty_name;
    };
    !first.is_ascii_digit() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// `text` without control characters, which a line editor would act on as
/// keys (^U clears the line, ^M runs it). Tab and newline stay.
pub fn without_control_characters(text: &str) -> String {
    text.chars().filter(|&c| !is_encoded(c)).collect()
}

/// C0, DEL and C1.
pub fn is_control(c: char) -> bool {
    (c as u32) < 0x20 || (0x7F..=0x9F).contains(&(c as u32))
}

/// Control characters that can't go in quotes as they are: all but tab and
/// newline.
fn is_encoded(c: char) -> bool {
    is_control(c) && c != '\n' && c != '\t'
}

/// A control character written as an escape the shell turns back into it
/// (`Icon\r`, the file behind a custom folder icon, keeps its `\r`): bash
/// and zsh `$'\x0d'` (ANSI-C quotes, its UTF-8 bytes), fish `\u000d`
/// outside quotes. A plain POSIX sh without `$'...'` reads it as `$` and
/// some text, never as the control character itself.
fn encoded_control(c: char, shell: ShellType) -> String {
    match shell {
        ShellType::Fish => format!("\\u{:04x}", c as u32),
        ShellType::Bash | ShellType::Zsh => {
            let mut buffer = [0u8; 4];
            let bytes: String = c
                .encode_utf8(&mut buffer)
                .bytes()
                .map(|b| format!("\\x{b:02x}"))
                .collect();
            format!("$'{bytes}'")
        }
    }
}

/// Read literally, bare, by every supported shell anywhere in a word: ASCII
/// letters and digits, `/ _ . - + , : @`, and anything beyond ASCII and C1
/// (none of the shells gives those a meaning).
fn is_safe(c: char) -> bool {
    match c as u32 {
        0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A => true,
        0x2B | 0x2C | 0x2D | 0x2E | 0x2F | 0x3A | 0x40 | 0x5F => true,
        value => value > 0x9F,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::child_process::ChildProcess;
    use crate::completion::CompletionCandidate;
    use std::path::Path;

    /// Words that break naive quoting: quotes, backslashes (one at the end
    /// leaves a fish quote open), substitutions, newlines, option-looking
    /// names, globs and expansions, text beyond ASCII, and control
    /// characters (`Icon\r` is the file behind a custom folder icon).
    const WORDS: &[&str] = &[
        "plain",
        "it's",
        "a\\b",
        "trailing\\",
        "\\'",
        "a\\';echo INJECTED;#",
        "$(touch PWNED)",
        "`id`",
        "line1\nline2",
        "tab\there",
        "-rf",
        "日本語 café 🚀",
        "a b",
        "!event",
        "~user",
        "~/x",
        "*?[a]",
        "{a,b}",
        "#comment",
        "semi;colon&|<>",
        "\"double\"",
        "%self",
        "=cmd",
        "a^b",
        "$HOME",
        "",
        "Icon\r",
        "x\u{15}touch MARKER;echo '",
        "\u{1b}[2Jcls",
        "del\u{7f}",
        "c1\u{9b}x",
        "\r\r",
    ];

    struct Shell {
        kind: ShellType,
        path: &'static str,
        flags: &'static [&'static str],
    }

    /// The shells to try, with the flags that skip the user's startup files.
    /// zsh and fish are left out where they aren't installed.
    fn shells() -> Vec<Shell> {
        let mut shells = vec![
            Shell {
                kind: ShellType::Bash,
                path: "/bin/sh",
                flags: &[],
            },
            Shell {
                kind: ShellType::Bash,
                path: "/bin/bash",
                flags: &["--norc", "--noprofile"],
            },
        ];
        if let Some(path) = ["/bin/zsh", "/usr/bin/zsh"]
            .into_iter()
            .find(|p| Path::new(p).exists())
        {
            shells.push(Shell {
                kind: ShellType::Zsh,
                path,
                flags: &["-f"],
            });
        }
        if let Some(path) = [
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
            "/usr/bin/fish",
        ]
        .into_iter()
        .find(|p| Path::new(p).exists())
        {
            shells.push(Shell {
                kind: ShellType::Fish,
                path,
                flags: &["--no-config"],
            });
        }
        shells
    }

    /// What `shell` passes to a program for `word` (each argument printed
    /// followed by a NUL), run in an empty folder.
    fn arguments(word: &str, shell: &Shell) -> Vec<String> {
        let base = tempfile::tempdir().unwrap();
        let folder = base.path().join("work");
        let home = base.path().join("home");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let lang = if cfg!(target_os = "macos") {
            "en_US.UTF-8"
        } else {
            "C.UTF-8"
        };
        let script = format!("printf '%s\\0' {word}");
        let mut args: Vec<&str> = shell.flags.to_vec();
        args.extend(["-c", &script]);
        let result = ChildProcess::new(shell.path)
            .args(&args)
            .dir(&folder)
            .env_clear()
            .env("LANG", lang)
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.to_str().unwrap())
            .timeout(std::time::Duration::from_secs(10))
            .run()
            .unwrap();
        assert_eq!(
            std::fs::read_dir(&folder).unwrap().count(),
            0,
            "{word:?} ran something"
        );
        let output = String::from_utf8_lossy(&result.stdout).into_owned();
        let Some(output) = output.strip_suffix('\0') else {
            return Vec::new();
        };
        output.split('\0').map(String::from).collect()
    }

    /// Whether a plain sh reads `$'...'` (dash, Linux's /bin/sh, doesn't;
    /// macOS's /bin/sh is bash): words with control characters can't reach
    /// a program intact without it.
    fn reads_ansi_c_quotes(shell: &Shell) -> bool {
        arguments("$'\\x41'", shell) == ["A"]
    }

    #[test]
    fn quoted_words_reach_the_program_intact() {
        for shell in shells() {
            let ansi_c = reads_ansi_c_quotes(&shell);
            for word in WORDS {
                if !ansi_c && shell.kind != ShellType::Fish && word.chars().any(is_encoded) {
                    continue;
                }
                let quoted = quoted(word, shell.kind);
                assert_eq!(
                    arguments(&quoted, &shell),
                    [*word],
                    "{}: {word:?} as {quoted:?}",
                    shell.path
                );
            }
        }
    }

    #[test]
    fn escaped_words_reach_the_program_intact() {
        for shell in shells() {
            let ansi_c = reads_ansi_c_quotes(&shell);
            for word in WORDS.iter().filter(|w| !w.is_empty()) {
                if !ansi_c && shell.kind != ShellType::Fish && word.chars().any(is_encoded) {
                    continue;
                }
                let escaped = escaped(word, shell.kind);
                assert_eq!(
                    arguments(&escaped, &shell),
                    [*word],
                    "{}: {word:?} as {escaped:?}",
                    shell.path
                );
            }
        }
    }

    #[test]
    fn quoting_is_per_shell() {
        assert_eq!(quoted("it's", ShellType::Zsh), "'it'\\''s'");
        assert_eq!(quoted("it's", ShellType::Fish), "'it\\'s'");
        assert_eq!(quoted("a\\b", ShellType::Bash), "'a\\b'");
        assert_eq!(quoted("a\\b", ShellType::Fish), "'a\\\\b'");
        // Nothing to quote.
        assert_eq!(quoted("src/main.swift", ShellType::Fish), "src/main.swift");
        assert_eq!(quoted("café", ShellType::Bash), "café");
        assert_eq!(quoted("", ShellType::Bash), "''");
        assert_eq!(
            escaped("my file (1).txt", ShellType::Zsh),
            "my\\ file\\ \\(1\\).txt"
        );
    }

    #[test]
    fn control_characters_are_written_as_escapes() {
        // Never typed raw (^U would clear the line and run the rest), but kept.
        assert_eq!(quoted("Icon\r", ShellType::Bash), "'Icon'$'\\x0d'");
        assert_eq!(quoted("Icon\r", ShellType::Fish), "'Icon'\\u000d");
        assert_eq!(quoted("a\u{9b}b", ShellType::Zsh), "'a'$'\\xc2\\x9b''b'");
        assert_eq!(escaped("a\u{3}b", ShellType::Bash), "a$'\\x03'b");
        assert_eq!(escaped("a\u{3}b", ShellType::Fish), "a\\u0003b");
        for shell in [ShellType::Bash, ShellType::Zsh, ShellType::Fish] {
            let text = quoted("x\u{15}touch MARKER;echo '", shell);
            assert!(!text.chars().any(is_control));
        }
        assert_eq!(without_control_characters("a\tb\nc\r\u{0}"), "a\tb\nc");
    }

    #[test]
    fn shell_completions_keep_their_head() {
        let fish = ShellType::Fish;
        // fish answers as typed: a leading `~/` or `$NAME/` still expands.
        assert_eq!(completion_text("~/Library/", "~/Lib", fish), "~/Library/");
        assert_eq!(completion_text("~/Library/", "~/lib", fish), "~/Library/");
        assert_eq!(
            completion_text("$HOME/my dir/", "$HOME/my", fish),
            "$HOME/my\\ dir/"
        );
        assert_eq!(completion_text("$HOME", "$HO", fish), "$HOME");
        // Everything else is a name.
        assert_eq!(
            completion_text("x$(touch PWNED)/", "x", fish),
            "x\\$\\(touch\\ PWNED\\)/"
        );
        assert_eq!(completion_text("my dir/", "my\\ d", fish), "my\\ dir/");
        assert_eq!(
            completion_text("src/a b.txt", "src/a\\ ", fish),
            "src/a\\ b.txt"
        );
        assert_eq!(completion_text("feature/it's", "", fish), "feature/it\\'s");
        assert_eq!(
            completion_text("~$Report.docx", "~$", fish),
            "\\~\\$Report.docx"
        );
        assert_eq!(completion_text("~", "~", fish), "\\~");
        assert_eq!(completion_text("a\\\\x", "a\\\\", fish), "a\\\\\\\\x");
    }

    #[test]
    fn impulses_own_candidates_are_literal() {
        let path = |value: &str, kept: &str| CompletionCandidate {
            kept_prefix: Some(kept.to_string()),
            ..CompletionCandidate::new(value, value, "path", false)
        };
        // A folder named `~` or `$HOME` is a name, not the home folder.
        assert_eq!(path("~/", "").insertion("~", ShellType::Bash), "\\~/");
        assert_eq!(
            path("$HOME/", "").insertion("$H", ShellType::Bash),
            "\\$HOME/"
        );
        assert_eq!(
            path("~$Report.docx", "").insertion("~$", ShellType::Zsh),
            "\\~\\$Report.docx"
        );
        // A home prefix the completer followed stays as typed.
        assert_eq!(
            path("~/Library/", "~/").insertion("~/Lib", ShellType::Bash),
            "~/Library/"
        );
        assert_eq!(
            path("$HOME/my dir/", "$HOME/").insertion("$HOME/my", ShellType::Fish),
            "$HOME/my\\ dir/"
        );
        // Branches and other values too.
        let branch = CompletionCandidate::new("feat$x", "feat$x", "branch", false);
        assert_eq!(branch.insertion("feat$", ShellType::Bash), "feat\\$x");
    }
}
