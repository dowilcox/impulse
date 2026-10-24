//! Turning one file's unified diff plus a hunk and line selection into a
//! smaller patch for `git apply` (the Swift `PatchBuilder`). This is how hunk
//! and line staging, unstaging and reverting work: the selected changes stay
//! changes, the rest of the hunk is neutralised.
//!
//! Pure text processing with no repository access; tested by round-tripping
//! the result through the real `git apply`.

use std::collections::{HashMap, HashSet};

/// Which parts of a file's diff to act on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchSelection {
    /// Hunk index to the selected line indices within it (indices count every
    /// context, added and removed line, but not "\ No newline at end of
    /// file" markers). `None` selects the whole hunk.
    pub hunks: HashMap<usize, Option<HashSet<usize>>>,
}

impl PatchSelection {
    pub fn whole_hunks(indices: &[usize]) -> Self {
        PatchSelection {
            hunks: indices.iter().map(|&index| (index, None)).collect(),
        }
    }

    pub fn lines(indices: &[usize], hunk: usize) -> Self {
        PatchSelection {
            hunks: HashMap::from([(hunk, Some(indices.iter().copied().collect()))]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    NothingSelected,
    PartialAddOrDelete,
    PartialRename,
    MalformedPatch,
    /// The file isn't UTF-8 text, so it can't be split byte-exactly.
    NotUtf8,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BuildError::NothingSelected => "No changes selected.",
            BuildError::PartialAddOrDelete => {
                "Part of a new or deleted file can't be staged on its own; select the whole file."
            }
            BuildError::PartialRename => {
                "Part of a renamed file can't be staged or unstaged on its own (it would undo the rename); select the whole file."
            }
            BuildError::MalformedPatch => "The diff couldn't be read.",
            BuildError::NotUtf8 => {
                "This file isn't UTF-8 text, so its hunks can't be staged, unstaged or reverted separately. Use the whole file instead."
            }
        })
    }
}

impl std::error::Error for BuildError {}

#[derive(Clone, Debug)]
struct Line {
    /// ' ', '+' or '-'.
    kind: char,
    text: String,
    /// Followed by "\ No newline at end of file".
    no_newline: bool,
}

#[derive(Clone, Debug)]
struct Hunk {
    old_start: i64,
    new_start: i64,
    /// Text after the closing "@@" (function context), with its space.
    section: String,
    lines: Vec<Line>,
}

struct Parsed {
    header: Vec<String>,
    hunks: Vec<Hunk>,
}

impl Parsed {
    fn is_new_file(&self) -> bool {
        self.header
            .iter()
            .any(|l| l.starts_with("new file mode") || l == "--- /dev/null")
    }

    fn is_deleted_file(&self) -> bool {
        self.header
            .iter()
            .any(|l| l.starts_with("deleted file mode") || l == "+++ /dev/null")
    }

    fn is_rename_or_copy(&self) -> bool {
        self.header
            .iter()
            .any(|l| l.starts_with("rename from ") || l.starts_with("copy from "))
    }
}

/// A patch with only the selected changes.
///
/// `reverse` builds it for `git apply --reverse` (unstaging from a staged
/// diff, or reverting working-tree changes): unselected additions then become
/// context and unselected removals are dropped; forward patches do the
/// opposite.
pub fn build(patch: &str, selection: &PatchSelection, reverse: bool) -> Result<String, BuildError> {
    let parsed = parse(patch).ok_or(BuildError::MalformedPatch)?;
    if selection.hunks.is_empty() {
        return Err(BuildError::NothingSelected);
    }

    // A patch with only some of a renamed file's changes keeps the rename
    // header, so applying it would move the rest back to the old path and
    // leave the new one untracked: such a file is taken whole or not at all.
    if parsed.is_rename_or_copy() {
        let whole = parsed.hunks.iter().enumerate().all(|(index, hunk)| {
            match selection.hunks.get(&index) {
                None => false,
                Some(None) => true,
                Some(Some(lines)) => hunk
                    .lines
                    .iter()
                    .enumerate()
                    .all(|(i, line)| line.kind == ' ' || lines.contains(&i)),
            }
        });
        if !whole {
            return Err(BuildError::PartialRename);
        }
    }

    let mut output = parsed.header.clone();
    let mut offset: i64 = 0;
    let mut emitted = 0;
    for (index, hunk) in parsed.hunks.iter().enumerate() {
        let Some(line_selection) = selection.hunks.get(&index) else {
            continue;
        };
        let partial = match line_selection {
            Some(chosen) => !hunk
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.kind != ' ')
                .all(|(i, _)| chosen.contains(&i)),
            None => false,
        };
        if partial && (parsed.is_new_file() || parsed.is_deleted_file()) {
            return Err(BuildError::PartialAddOrDelete);
        }

        let mut lines = Vec::new();
        for (line_index, line) in hunk.lines.iter().enumerate() {
            let selected = line_selection
                .as_ref()
                .is_none_or(|chosen| chosen.contains(&line_index));
            let as_context = || Line {
                kind: ' ',
                ..line.clone()
            };
            match line.kind {
                '+' if selected => lines.push(line.clone()),
                '+' if reverse => lines.push(as_context()),
                '+' => {}
                '-' if selected => lines.push(line.clone()),
                '-' if !reverse => lines.push(as_context()),
                '-' => {}
                _ => lines.push(line.clone()),
            }
        }
        if lines.iter().all(|line| line.kind == ' ') {
            continue;
        }

        let old_count = lines.iter().filter(|l| l.kind != '+').count() as i64;
        let new_count = lines.iter().filter(|l| l.kind != '-').count() as i64;
        // git searches for each hunk starting at its post-image position in
        // the file as already changed by the hunks before it. Forward, that's
        // the old position shifted by the emitted hunks. Reversed (the sides
        // swap), the target is the diff's new side: start from the hunk's new
        // position, shifted by the emitted hunks being undone, never from the
        // old position, which is off by every unselected earlier hunk and can
        // land the change in an identical block elsewhere. A hunk with
        // nothing to match (a zero-context insertion, or reverting a
        // zero-context deletion) goes exactly there, and an empty range's
        // start names the line before it: insert after that line.
        let (old_start, new_start);
        if reverse {
            new_start = hunk.new_start;
            old_start = if new_count == 0 {
                hunk.new_start + 1 + offset
            } else if old_count == 0 {
                (hunk.new_start - 1 + offset).max(0)
            } else {
                hunk.new_start + offset
            };
            offset += old_count - new_count;
        } else {
            old_start = hunk.old_start;
            new_start = if old_count == 0 {
                hunk.old_start + 1 + offset
            } else {
                old_start + offset
            };
            offset += new_count - old_count;
        }
        output.push(format!(
            "@@ -{} +{} @@{}",
            range(old_start, old_count),
            range(new_start, new_count),
            hunk.section
        ));
        for line in lines {
            output.push(format!("{}{}", line.kind, line.text));
            if line.no_newline {
                output.push("\\ No newline at end of file".into());
            }
        }
        emitted += 1;
    }
    if emitted == 0 {
        return Err(BuildError::NothingSelected);
    }
    Ok(output.join("\n") + "\n")
}

fn range(start: i64, count: i64) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

// MARK: Parsing

fn parse(patch: &str) -> Option<Parsed> {
    let mut lines: Vec<&str> = patch.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut header = Vec::new();
    let mut hunks = Vec::new();
    let mut index = 0;
    while index < lines.len() && !lines[index].starts_with("@@") {
        header.push(lines[index].to_string());
        index += 1;
    }
    while index < lines.len() {
        let mut current = parse_hunk_header(lines[index])?;
        index += 1;
        while index < lines.len() && !lines[index].starts_with("@@") {
            let raw = lines[index];
            match raw.chars().next() {
                // "\ No newline at end of file" belongs to the line before.
                Some('\\') => {
                    if let Some(last) = current.lines.last_mut() {
                        last.no_newline = true;
                    }
                }
                Some(kind @ (' ' | '+' | '-')) => current.lines.push(Line {
                    kind,
                    text: raw[1..].to_string(),
                    no_newline: false,
                }),
                // Some tools drop the leading space of empty context lines.
                None => current.lines.push(Line {
                    kind: ' ',
                    text: String::new(),
                    no_newline: false,
                }),
                Some(_) => return None,
            }
            index += 1;
        }
        hunks.push(current);
    }
    Some(Parsed { header, hunks })
}

/// "@@ -a,b +c,d @@ section"; a count defaults to 1 when left out.
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    if !line.starts_with("@@ -") {
        return None;
    }
    let after_at = &line[3..];
    let close = after_at.find(" @@")?;
    let ranges: Vec<&str> = after_at[..close]
        .split(' ')
        .filter(|r| !r.is_empty())
        .collect();
    if ranges.len() != 2 || !ranges[0].starts_with('-') || !ranges[1].starts_with('+') {
        return None;
    }
    let parse_range = |text: &str| -> Option<(i64, i64)> {
        let parts: Vec<&str> = text[1..].split(',').collect();
        let start = parts[0].parse().ok()?;
        let count = match parts.get(1) {
            Some(count) => count.parse().ok()?,
            None => 1,
        };
        Some((start, count))
    };
    let (old_start, _) = parse_range(ranges[0])?;
    let (new_start, _) = parse_range(ranges[1])?;
    Some(Hunk {
        old_start,
        new_start,
        section: after_at[close + 3..].to_string(),
        lines: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_FILE: &str = "diff --git a/n.txt b/n.txt\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";

    #[test]
    fn a_partial_selection_of_a_new_file_is_refused() {
        assert_eq!(
            build(NEW_FILE, &PatchSelection::lines(&[0], 0), false),
            Err(BuildError::PartialAddOrDelete)
        );
        let whole = build(NEW_FILE, &PatchSelection::whole_hunks(&[0]), false).unwrap();
        assert!(whole.contains("+two"));
    }

    #[test]
    fn unselected_lines_become_context_or_go() {
        let patch = "--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@ fn x\n a\n-b\n+B\n c\n\\ No newline at end of file\n";
        // Forward: only the removal; the unselected addition is dropped.
        let forward = build(patch, &PatchSelection::lines(&[1], 0), false).unwrap();
        assert_eq!(
            forward,
            "--- a/f\n+++ b/f\n@@ -1,3 +1,2 @@ fn x\n a\n-b\n c\n\\ No newline at end of file\n"
        );
        // Reverse: only the addition; the unselected removal is dropped.
        let reverse = build(patch, &PatchSelection::lines(&[2], 0), true).unwrap();
        assert_eq!(
            reverse,
            "--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@ fn x\n a\n+B\n c\n\\ No newline at end of file\n"
        );
        assert_eq!(
            build(patch, &PatchSelection::default(), false),
            Err(BuildError::NothingSelected)
        );
        assert_eq!(
            build("@@ nonsense", &PatchSelection::whole_hunks(&[0]), false),
            Err(BuildError::MalformedPatch)
        );
    }
}
