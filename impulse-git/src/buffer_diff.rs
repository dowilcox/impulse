//! The editor's Diff View (Monaco's diff editor in the Swift app): a file's
//! text at HEAD, and which lines differ between it and the editor's buffer,
//! worked out on the texts themselves so the view follows unsaved edits.

use std::ops::Range;
use std::path::Path;

use git2::{DiffOptions, Patch, Repository};

use crate::word_diff::word_spans;

/// Lines (0-based, end exclusive) that differ on each side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineChanges {
    /// In the old text: removed or changed.
    pub removed: Vec<Range<u32>>,
    /// In the new text: added or changed.
    pub added: Vec<Range<u32>>,
}

/// The file's text at HEAD: empty for a file HEAD doesn't have (all of it
/// is new), `None` outside a repository or for a binary file.
pub fn head_text(file: &Path) -> Option<String> {
    let repo = Repository::discover(file.parent()?).ok()?;
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let file = file.canonicalize().ok()?;
    let relative = file.strip_prefix(&workdir).ok()?;
    let Ok(tree) = repo.head().and_then(|head| head.peel_to_tree()) else {
        // No commit yet.
        return Some(String::new());
    };
    let Ok(entry) = tree.get_path(relative) else {
        return Some(String::new());
    };
    let blob = entry.to_object(&repo).ok()?.peel_to_blob().ok()?;
    if blob.is_binary() {
        return None;
    }
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

/// The lines that differ between `old` and `new`, as `git diff` would mark
/// them (no context).
pub fn line_changes(old: &str, new: &str) -> LineChanges {
    let mut options = DiffOptions::new();
    options.context_lines(0);
    let Ok(patch) = Patch::from_buffers(
        old.as_bytes(),
        None,
        new.as_bytes(),
        None,
        Some(&mut options),
    ) else {
        return LineChanges::default();
    };
    let mut changes = LineChanges::default();
    for index in 0..patch.num_hunks() {
        let Ok((hunk, _)) = patch.hunk(index) else {
            continue;
        };
        // A side with no lines reports the line before it.
        if hunk.old_lines() > 0 {
            let start = hunk.old_start() - 1;
            changes.removed.push(start..start + hunk.old_lines());
        }
        if hunk.new_lines() > 0 {
            let start = hunk.new_start() - 1;
            changes.added.push(start..start + hunk.new_lines());
        }
    }
    changes
}

/// The byte ranges of `lines` in `text`, each line with its line break.
pub fn line_byte_ranges(text: &str, lines: &[Range<u32>]) -> Vec<Range<usize>> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    starts.push(text.len());
    let at = |line: u32| starts.get(line as usize).copied().unwrap_or(text.len());
    lines
        .iter()
        .map(|range| at(range.start)..at(range.end))
        .filter(|range| range.start < range.end)
        .collect()
}

/// The words that differ within lines a hunk replaced (Monaco's character
/// changes, tinted over its line tints): byte ranges in `old` and in `new`.
/// `hunks` are the lines (0-based, end exclusive) each hunk replaced and
/// what it put there. A hunk's lines pair up in order; lines left over
/// changed whole. Hunks that only add or only remove lines have none.
pub fn changed_words(
    old: &str,
    new: &str,
    hunks: impl IntoIterator<Item = (Range<usize>, Range<usize>)>,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (old_lines, new_lines) = (line_spans(old), line_spans(new));
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for (old_rows, new_rows) in hunks {
        if old_rows.is_empty() || new_rows.is_empty() {
            continue;
        }
        let olds = old_lines.get(old_rows).unwrap_or_default();
        let news = new_lines.get(new_rows).unwrap_or_default();
        for k in 0..olds.len().max(news.len()) {
            match (olds.get(k), news.get(k)) {
                (Some(o), Some(n)) => {
                    let (o_text, n_text) = (&old[o.clone()], &new[n.clone()]);
                    let (o_spans, n_spans) = word_spans(o_text, n_text);
                    removed.extend(joined(
                        old,
                        o_spans.iter().map(|s| bytes(o_text, o.start, s)),
                    ));
                    added.extend(joined(
                        new,
                        n_spans.iter().map(|s| bytes(n_text, n.start, s)),
                    ));
                }
                (Some(o), None) => removed.push(o.clone()),
                (None, Some(n)) => added.push(n.clone()),
                (None, None) => {}
            }
        }
    }
    removed.retain(|r| !r.is_empty());
    added.retain(|r| !r.is_empty());
    (removed, added)
}

/// `ranges` of `text` with the ones only whitespace apart joined, so a run
/// of changed words reads as one change.
fn joined(text: &str, ranges: impl Iterator<Item = Range<usize>>) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match out.last_mut() {
            Some(last) if text[last.end..range.start].trim().is_empty() => last.end = range.end,
            _ => out.push(range),
        }
    }
    out
}

/// Each line's byte range, without its line break.
fn line_spans(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut lines = Vec::new();
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches('\n').trim_end_matches('\r');
        lines.push(start..start + content.len());
        start += line.len();
    }
    lines
}

/// A UTF-16 word span of `line` (which starts at byte `at`) in bytes.
fn bytes(line: &str, at: usize, span: &crate::hunks::WordSpan) -> Range<usize> {
    let byte = |unit: u32| {
        let mut units = 0;
        for (index, c) in line.char_indices() {
            if units >= unit as usize {
                return index;
            }
            units += c.len_utf16();
        }
        line.len()
    };
    at + byte(span.start)..at + byte(span.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_changed_added_and_removed_lines() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nd\ne\n";
        let changes = line_changes(old, new);
        assert_eq!(changes.removed, vec![1..2]);
        assert_eq!(changes.added, vec![1..2, 4..5]);
        assert_eq!(line_changes("x\ny\n", "y\n").removed, vec![0..1]);
        assert!(line_changes("x\ny\n", "y\n").added.is_empty());
        assert_eq!(line_changes(old, old), LineChanges::default());
    }

    #[test]
    fn finds_the_changed_words_of_replaced_lines() {
        let old = "a\nlet x = cache.get(id);\nz\n";
        let new = "a\nlet x = forecastCache.get(id);\nnew line\nz\nadded\n";
        let (_, joined) = changed_words("a b c d\n", "a X Y d\n", [(0..1, 0..1)]);
        assert_eq!(joined, vec![2..5]);
        let (removed, added) = changed_words(old, new, [(1..2, 1..3), (3..3, 4..5)]);
        assert_eq!(
            removed.iter().map(|r| &old[r.clone()]).collect::<Vec<_>>(),
            vec!["cache.get(id);"]
        );
        assert_eq!(
            added.iter().map(|r| &new[r.clone()]).collect::<Vec<_>>(),
            vec!["forecastCache.get(id);", "new line"]
        );
    }

    #[test]
    fn line_ranges_include_their_breaks() {
        let text = "ab\ncd\nef";
        assert_eq!(line_byte_ranges(text, &[1..2]), vec![3..6]);
        assert_eq!(line_byte_ranges(text, &[2..3]), vec![6..8]);
        assert_eq!(line_byte_ranges(text, &[0..3]), vec![0..8]);
    }

    #[test]
    fn reads_the_head_version() {
        let root =
            &std::env::temp_dir().join(format!("impulse-buffer-diff-{}", std::process::id()));
        std::fs::create_dir_all(root).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap()
        };
        run(&["init", "-q"]);
        std::fs::write(root.join("f.txt"), "one\n").unwrap();
        run(&["add", "f.txt"]);
        run(&[
            "-c",
            "user.name=a",
            "-c",
            "user.email=a@b",
            "commit",
            "-qm",
            "m",
        ]);
        std::fs::write(root.join("f.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "x\n").unwrap();
        assert_eq!(head_text(&root.join("f.txt")).as_deref(), Some("one\n"));
        assert_eq!(head_text(&root.join("new.txt")).as_deref(), Some(""));
        let _ = std::fs::remove_dir_all(root);
    }
}
