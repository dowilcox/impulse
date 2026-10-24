//! Readers for the shells' own history files (the Swift
//! `ShellHistoryImport`), so Impulse's history can start with what the user
//! already typed elsewhere. A command without a time gets `DISTANT_PAST`,
//! as the Swift app gives it, so importing the same file from either app
//! adds nothing twice.
//!
//! Files are read as UTF-8; a stray invalid byte becomes U+FFFD in its line
//! rather than losing the whole file. Lines end at `\n`, with a trailing
//! `\r` dropped.

use super::database::{HistoryEntry, DISTANT_PAST};

/// zsh: plain lines, or EXTENDED_HISTORY `: <start>:<elapsed>;<command>`.
/// Multi-line commands continue with a trailing backslash. `data` is the raw
/// file (zsh "metafies" non-ASCII bytes; they're restored here).
pub fn zsh(data: &[u8]) -> Vec<HistoryEntry> {
    let text = String::from_utf8_lossy(&unmetafy(data)).into_owned();
    let mut entries = Vec::new();
    let mut pending: Option<(String, Option<f64>)> = None;
    let flush = |entries: &mut Vec<HistoryEntry>, pending: &mut Option<(String, Option<f64>)>| {
        if let Some((command, date)) = pending.take() {
            if HistoryEntry::should_record(&command) {
                entries.push(HistoryEntry::new(command, date.unwrap_or(DISTANT_PAST)));
            }
        }
    };

    for line in lines(&text, false) {
        if let Some((command, date)) = pending.take() {
            // Continuation of a multi-line command.
            let (line, continues) = match line.strip_suffix('\\') {
                Some(rest) => (rest, true),
                None => (line, false),
            };
            pending = Some((format!("{command}\n{line}"), date));
            if !continues {
                flush(&mut entries, &mut pending);
            }
            continue;
        }
        let mut line = line;
        let mut date = None;
        if line.starts_with(": ") {
            if let Some(semicolon) = line.find(';') {
                let meta = &line[2..semicolon];
                date = meta
                    .split(':')
                    .find(|s| !s.is_empty())
                    .and_then(|s| s.parse::<f64>().ok());
                line = &line[semicolon + 1..];
            }
        }
        match line.strip_suffix('\\') {
            Some(rest) => pending = Some((rest.to_string(), date)),
            None => {
                pending = Some((line.to_string(), date));
                flush(&mut entries, &mut pending);
            }
        }
    }
    flush(&mut entries, &mut pending);
    entries
}

/// bash from the file's bytes (see `bash` for the format).
pub fn bash_bytes(data: &[u8]) -> Vec<HistoryEntry> {
    bash(&String::from_utf8_lossy(data))
}

/// bash: one command per line, optionally preceded by `#<timestamp>` lines
/// (HISTTIMEFORMAT).
pub fn bash(text: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut date = None;
    for command in lines(text, true) {
        if let Some(seconds) = command
            .strip_prefix('#')
            .and_then(|s| s.parse::<f64>().ok())
        {
            date = Some(seconds);
            continue;
        }
        if HistoryEntry::should_record(command) {
            entries.push(HistoryEntry::new(command, date.unwrap_or(DISTANT_PAST)));
        }
        date = None;
    }
    entries
}

/// fish from the file's bytes (see `fish` for the format).
pub fn fish_bytes(data: &[u8]) -> Vec<HistoryEntry> {
    fish(&String::from_utf8_lossy(data))
}

/// fish: YAML-like `- cmd: <command>` / `  when: <seconds>` records, with
/// `\n` and `\\` escapes in the command.
pub fn fish(text: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut command: Option<String> = None;
    let mut date: Option<f64> = None;
    let flush =
        |entries: &mut Vec<HistoryEntry>, command: &mut Option<String>, date: &mut Option<f64>| {
            if let Some(command) = command.take() {
                if HistoryEntry::should_record(&command) {
                    entries.push(HistoryEntry::new(command, date.unwrap_or(DISTANT_PAST)));
                }
            }
            *date = None;
        };
    for line in lines(text, true) {
        if let Some(rest) = line.strip_prefix("- cmd: ") {
            flush(&mut entries, &mut command, &mut date);
            command = Some(unescape_fish(rest));
        } else if let Some(seconds) = line
            .strip_prefix("  when: ")
            .and_then(|s| s.parse::<f64>().ok())
        {
            date = Some(seconds);
        }
    }
    flush(&mut entries, &mut command, &mut date);
    entries
}

fn unescape_fish(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut escaping = false;
    for c in text.chars() {
        if escaping {
            result.push(if c == 'n' { '\n' } else { c });
            escaping = false;
        } else if c == '\\' {
            escaping = true;
        } else {
            result.push(c);
        }
    }
    if escaping {
        result.push('\\');
    }
    result
}

/// The lines of `text`, split at each `\n`, without a trailing `\r`.
fn lines(text: &str, omitting_empty: bool) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(move |line| !omitting_empty || !line.is_empty())
}

/// Undo zsh's metafication: 0x83 marks that the next byte was XORed with 32.
fn unmetafy(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut bytes = data.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == 0x83 {
            if let Some(next) = bytes.next() {
                out.push(next ^ 32);
                continue;
            }
        }
        out.push(byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(entries: &[HistoryEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.command.as_str()).collect()
    }

    #[test]
    fn imports_crlf_and_invalid_utf8() {
        assert_eq!(
            commands(&bash("#1700000000\r\nls -la\r\nmake\r\n")),
            ["ls -la", "make"]
        );
        assert_eq!(bash("#1700000000\r\nls\r\n")[0].started_at, 1_700_000_000.0);
        assert_eq!(
            commands(&zsh(b": 1700000000:0;git log\r\nplain\r\n")),
            ["git log", "plain"]
        );
        assert_eq!(
            fish("- cmd: git status\r\n  when: 1700000000\r\n")[0].started_at,
            1_700_000_000.0
        );
        // One bad byte costs its line's character, not the file.
        let mut data = b"ls\necho ".to_vec();
        data.extend([0xFF, 0x0A]);
        data.extend(b"make\n");
        assert_eq!(
            commands(&bash_bytes(&data)),
            ["ls", "echo \u{FFFD}", "make"]
        );
        let mut fish_data = b"- cmd: a".to_vec();
        fish_data.push(0xC3);
        fish_data.extend(b"\n- cmd: b\n");
        assert_eq!(commands(&fish_bytes(&fish_data)), ["a\u{FFFD}", "b"]);
    }

    #[test]
    fn imports_zsh_history() {
        let mut data =
            b": 1700000000:0;git log\n: 1700000100:3;echo one \\\n two\nplain\n".to_vec();
        // A metafied "é" (0xC3 0xA9 becomes 0xC3 0x83 0x89).
        data.extend([0x65, 0x63, 0x68, 0x6F, 0x20, 0xC3, 0x83, 0xA9 ^ 32, 0x0A]);
        let entries = zsh(&data);
        assert_eq!(
            commands(&entries),
            ["git log", "echo one \n two", "plain", "echo é"]
        );
        assert_eq!(entries[0].started_at, 1_700_000_000.0);
        assert_eq!(entries[2].started_at, DISTANT_PAST);
    }

    #[test]
    fn imports_bash_and_fish_history() {
        let bash = bash("#1700000000\nls\n pwd\nmake test\n");
        assert_eq!(commands(&bash), ["ls", "make test"]);
        assert_eq!(bash[0].started_at, 1_700_000_000.0);

        let fish = fish(
            "- cmd: git status\n  when: 1700000000\n- cmd: echo a\\nb \\\\ c\n  when: 1700000005\n  paths:\n    - a\n",
        );
        assert_eq!(commands(&fish), ["git status", "echo a\nb \\ c"]);
        assert_eq!(fish[1].started_at, 1_700_000_005.0);
    }
}
