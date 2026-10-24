//! Syntax colors in Review (the Swift `SyntaxHighlighter`, which runs
//! highlight.js): each file's old and new sides are tokenized separately,
//! their hunks' lines joined in order, by GPUI Kit's tree-sitter
//! highlighter in the file's language, colored by the theme's syntax colors
//! (`kit_theme`). Context lines take the new side's colors. Files past
//! `MAX_LINES` of diff stay plain.

use std::ops::Range;

use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_kit::component::input::Rope;
use gpui_kit::Hsla;
use impulse_git::hunks::DiffLineKind;
use impulse_git::scoped_diff::FileDiff;

/// More diff lines than this aren't highlighted.
const MAX_LINES: usize = 5_000;

/// A line's colored byte ranges of its content.
pub(super) type LineColors = Vec<(Range<usize>, Hsla)>;

/// Colors by hunk, then line.
#[derive(Default)]
pub(super) struct FileSyntax {
    pub hunks: Vec<Vec<LineColors>>,
}

/// The colors of `diff`'s lines, in the language git's diff names.
pub(super) fn highlight(diff: &FileDiff, theme: &HighlightTheme) -> FileSyntax {
    let mut out = FileSyntax {
        hunks: diff
            .hunks
            .iter()
            .map(|h| vec![Vec::new(); h.lines.len()])
            .collect(),
    };
    let total: usize = diff.hunks.iter().map(|h| h.lines.len()).sum();
    if total == 0 || total > MAX_LINES || diff.is_binary {
        return out;
    }
    let grammar = crate::editor_tab::grammar_for(&diff.language);
    if grammar.is_empty() || grammar == "plaintext" || grammar == "text" {
        return out;
    }
    for old_side in [true, false] {
        // The side's lines in order, and where each starts.
        let mut text = String::new();
        let mut placed: Vec<(usize, usize, usize)> = Vec::new();
        for (h, hunk) in diff.hunks.iter().enumerate() {
            for (l, line) in hunk.lines.iter().enumerate() {
                let on_side = match line.kind {
                    DiffLineKind::Context => true,
                    DiffLineKind::Removed => old_side,
                    DiffLineKind::Added => !old_side,
                };
                if !on_side {
                    continue;
                }
                placed.push((h, l, text.len()));
                text.push_str(&line.content);
                text.push('\n');
            }
        }
        if text.is_empty() {
            continue;
        }
        let mut highlighter = SyntaxHighlighter::new(grammar);
        highlighter.update(None, &Rope::from(text.as_str()), None);
        let styles = highlighter.styles(&(0..text.len()), theme);
        // Both lists are in text order: walk them together.
        let mut s = 0;
        for &(h, l, start) in &placed {
            let line = &diff.hunks[h].lines[l];
            // Context lines are colored once, from the new side.
            if old_side && line.kind == DiffLineKind::Context {
                continue;
            }
            let end = start + line.content.len();
            while s < styles.len() && styles[s].0.end <= start {
                s += 1;
            }
            let colors = &mut out.hunks[h][l];
            for (range, style) in styles[s..].iter().take_while(|(r, _)| r.start < end) {
                let Some(color) = style.color else {
                    continue;
                };
                let from = range.start.max(start) - start;
                let to = range.end.min(end) - start;
                if from < to {
                    colors.push((from..to, color));
                }
            }
        }
    }
    out
}
