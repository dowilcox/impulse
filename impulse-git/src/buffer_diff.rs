//! The editor's Diff View (Monaco's diff editor in the Swift app): a file's
//! text at HEAD, and which lines differ between it and the editor's buffer,
//! worked out on the texts themselves so the view follows unsaved edits.

use std::ops::Range;
use std::path::Path;

use git2::{DiffOptions, Patch, Repository};

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
