//! Monaco's bracket pair guides and matching-bracket box, painted over the
//! editor: a line in each pair's color down the lines between its brackets
//! (30% strength, full for the innermost pair around the cursor), and a box
//! around the two brackets when the cursor touches one. The pairs and
//! guides come from `impulse_kit::bracket_pairs`, worked out on each edit
//! with the bracket colors (`EditorTab::paint_brackets`).

use std::rc::Rc;

use gpui_kit::component::input::{EditorState, RopeExt as _};
use gpui_kit::{fill, outline, point, px, size, Bounds, Hsla, Pixels, Window};
use impulse_kit::bracket_pairs::{self, Guide, Pair};

/// What to paint, kept by the editor between edits.
#[derive(Clone, Default)]
pub(crate) struct Brackets {
    pub pairs: Rc<Vec<Pair>>,
    pub guides: Rc<Vec<Guide>>,
}

/// The colors, from the theme's mode.
#[derive(Clone, Copy)]
pub(crate) struct BracketColors {
    /// Monaco's three bracket colors.
    pub levels: [Hsla; 3],
    /// `editorBracketMatch.border` and `.background`.
    pub match_border: Hsla,
    pub match_background: Hsla,
}

impl BracketColors {
    pub fn new(dark: bool) -> Self {
        let hex = |h: &str| crate::workbench_chrome::hsla(impulse_kit::chrome::Rgba::from_hex(h));
        let levels = if dark {
            bracket_pairs::DARK
        } else {
            bracket_pairs::LIGHT
        }
        .map(hex);
        BracketColors {
            levels,
            match_border: hex(if dark { "#888888" } else { "#B9B9B9" }),
            match_background: hex("#0064001a"),
        }
    }
}

impl Brackets {
    /// Paint the guides on the visible lines and the box around the pair
    /// the cursor touches.
    pub fn paint(
        &self,
        colors: BracketColors,
        state: &EditorState,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let Some(rows) = state.visible_row_range() else {
            return;
        };
        let text = state.text();
        let cursor = state.cursor();
        let row_bounds = |row: usize| {
            (row < text.lines_len()).then(|| {
                let start = text.line_start_offset(row);
                state.range_to_bounds(&(start..start))
            })?
        };
        // A character's width, measured on a visible line.
        let cell = rows.clone().find_map(|row| {
            if row >= text.lines_len() || text.line_len(row) == 0 {
                return None;
            }
            let start = text.line_start_offset(row);
            let first = text.slice(start..start + text.line_len(row)).to_string();
            let width = first.chars().next()?.len_utf8();
            let at = state.range_to_bounds(&(start..start + width))?;
            Some(at.size.width)
        });
        if let Some(cell) = cell {
            let active = bracket_pairs::active(&self.pairs, cursor);
            for guide in self.guides.iter() {
                let first = guide.lines.start.max(rows.start);
                let last = guide.lines.end.min(rows.end);
                if first >= last {
                    continue;
                }
                let color = colors.levels[guide.pair.depth % 3];
                let color = if Some(guide.pair) == active {
                    color
                } else {
                    color.opacity(0.3)
                };
                // Line by line, joined where they touch, so the guide
                // breaks at the view zones between them (a hidden run's
                // row, the git peek, a conflict's actions), as Monaco's.
                let mut run: Option<(Pixels, Pixels, Pixels)> = None;
                let mut segments = Vec::new();
                for row in first..last {
                    let Some(at) = row_bounds(row) else {
                        continue;
                    };
                    // To the line's last wrapped row.
                    let bottom = text_line_bottom(state, row).unwrap_or(at.bottom());
                    match run.as_mut() {
                        Some((_, _, end)) if (at.top() - *end).abs() < px(0.5) => *end = bottom,
                        _ => {
                            segments.extend(run.take());
                            run = Some((at.origin.x, at.top(), bottom));
                        }
                    }
                }
                segments.extend(run);
                for (left, top, bottom) in segments {
                    let x = left + cell * guide.column as f32;
                    if x < bounds.left() || x > bounds.right() {
                        continue;
                    }
                    window.paint_quad(fill(
                        Bounds::from_corners(point(x, top), point(x + px(1.), bottom)),
                        color,
                    ));
                }
            }
        }
        if let Some(pair) = bracket_pairs::matching(&self.pairs, cursor) {
            for at in [pair.open, pair.close] {
                if let Some(b) = state
                    .range_to_bounds(&(at..at + 1))
                    .filter(|b| b.top() >= bounds.top() && b.bottom() <= bounds.bottom())
                {
                    let b = Bounds::new(b.origin, size(b.size.width.max(px(1.)), b.size.height));
                    window.paint_quad(fill(b, colors.match_background));
                    window.paint_quad(outline(
                        b,
                        colors.match_border,
                        gpui_kit::BorderStyle::Solid,
                    ));
                }
            }
        }
    }
}

/// The bottom of buffer line `row`'s last wrapped row, from the last
/// layout.
fn text_line_bottom(state: &EditorState, row: usize) -> Option<Pixels> {
    let text = state.text();
    (row < text.lines_len())
        .then(|| text.line_end_offset(row))
        .and_then(|end| state.range_to_bounds(&(end..end)))
        .map(|b| b.bottom())
}
