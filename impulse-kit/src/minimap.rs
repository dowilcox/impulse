//! The minimap's layout (Monaco's `minimap`, the `minimap_enabled`
//! setting): each line drawn as runs of one-pixel-wide characters, two
//! pixels tall, scrolled in proportion to the editor so its top and bottom
//! meet the document's.

/// Pixels per line and per character (Monaco's scale 1).
pub const ROW_HEIGHT: f32 = 2.0;
pub const COLUMN_WIDTH: f32 = 1.0;
/// Columns drawn (Monaco's `maxColumn`), which is also its width.
pub const MAX_COLUMNS: usize = 120;

/// The runs of visible characters on `line` as (first column, columns),
/// tabs expanded to `tab_size`, cut at `max_columns`.
pub fn runs(line: &str, tab_size: usize, max_columns: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut column = 0;
    for c in line.chars() {
        if column >= max_columns {
            break;
        }
        if c == '\t' {
            let size = tab_size.max(1);
            column += size - column % size;
            continue;
        }
        if !c.is_whitespace() {
            match out.last_mut() {
                Some((start, len)) if *start + *len == column => *len += 1,
                _ => out.push((column, 1)),
            }
        }
        column += 1;
    }
    out
}

/// The first document row the minimap shows: 0 while every line fits,
/// otherwise in proportion to how far the editor is scrolled.
pub fn first_row(total: usize, fits: usize, first_visible: usize, visible: usize) -> usize {
    if total <= fits {
        return 0;
    }
    let scrollable = total.saturating_sub(visible).max(1);
    let ratio = first_visible.min(scrollable) as f32 / scrollable as f32;
    ((total - fits) as f32 * ratio).round() as usize
}

/// The editor's top row that centers the document row under the pointer
/// (a click or drag on the minimap).
pub fn scroll_target(clicked: usize, total: usize, visible: usize) -> usize {
    clicked
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_skip_spaces_and_expand_tabs() {
        assert_eq!(
            runs("let x = 1;", 4, 120),
            vec![(0, 3), (4, 1), (6, 1), (8, 2)]
        );
        assert_eq!(runs("\tfoo", 4, 120), vec![(4, 3)]);
        assert_eq!(runs("  \t a", 4, 120), vec![(5, 1)]);
        assert_eq!(runs("abcdef", 4, 3), vec![(0, 3)], "cut at the edge");
        assert!(runs("   ", 4, 120).is_empty());
    }

    #[test]
    fn scrolls_in_proportion() {
        assert_eq!(first_row(100, 400, 50, 40), 0, "everything fits");
        assert_eq!(first_row(1000, 400, 0, 40), 0);
        assert_eq!(first_row(1000, 400, 960, 40), 600, "the ends meet");
        assert_eq!(first_row(1000, 400, 480, 40), 300);
        assert_eq!(scroll_target(100, 1000, 40), 80);
        assert_eq!(scroll_target(5, 1000, 40), 0);
        assert_eq!(scroll_target(999, 1000, 40), 960);
    }
}
