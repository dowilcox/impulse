//! Box-drawing and block characters drawn as shapes instead of font glyphs
//! (the Swift `TerminalRenderer.drawBoxDrawing` and `drawBlockElement`): a
//! font's glyphs don't fill the cell, so TUI borders, fills and sprites
//! (Claude Code's logo) show seams between cells. Edges are snapped to the
//! device's pixels so neighbouring cells meet exactly.

use gpui_kit::{fill, point, px, size, Bounds, Hsla, PathBuilder, Pixels, Window};

/// Whether `ch` is drawn here rather than shaped.
pub(crate) fn is_geometric(ch: char) -> bool {
    matches!(ch as u32,
        0x2500..=0x2503
        | 0x250C | 0x2510 | 0x2514 | 0x2518
        | 0x251C | 0x2524 | 0x252C | 0x2534 | 0x253C
        | 0x2550 | 0x2551
        | 0x256D..=0x2570
        | 0x2580..=0x259F)
}

/// Paint `ch` (one `is_geometric` says yes to) into `cell` in `color`.
pub(crate) fn paint(ch: char, cell: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let scale = window.scale_factor();
    let snap = |v: f32| (v * scale).round() / scale;
    let (x, y) = (f32::from(cell.origin.x), f32::from(cell.origin.y));
    let (w, h) = (f32::from(cell.size.width), f32::from(cell.size.height));
    let rect = |rx: f32, ry: f32, rw: f32, rh: f32, alpha: f32, window: &mut Window| {
        let (x0, y0) = (snap(rx), snap(ry));
        let (x1, y1) = (snap(rx + rw), snap(ry + rh));
        if x1 > x0 && y1 > y0 {
            window.paint_quad(fill(
                Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0))),
                color.opacity(color.a * alpha),
            ));
        }
    };
    let (mid_x, mid_y) = (snap(x + w / 2.), snap(y + h / 2.));
    let (thin, thick) = (1.0, 2.0);
    let code = ch as u32;
    match code {
        0x2500 => rect(x, mid_y - thin / 2., w, thin, 1., window),
        0x2501 => rect(x, mid_y - thick / 2., w, thick, 1., window),
        0x2502 => rect(mid_x - thin / 2., y, thin, h, 1., window),
        0x2503 => rect(mid_x - thick / 2., y, thick, h, 1., window),
        // Corners and junctions: each arm from the middle to an edge.
        0x250C | 0x2510 | 0x2514 | 0x2518 | 0x251C | 0x2524 | 0x252C | 0x2534 | 0x253C => {
            let (up, down, left, right) = match code {
                0x250C => (false, true, false, true),
                0x2510 => (false, true, true, false),
                0x2514 => (true, false, false, true),
                0x2518 => (true, false, true, false),
                0x251C => (true, true, false, true),
                0x2524 => (true, true, true, false),
                0x252C => (false, true, true, true),
                0x2534 => (true, false, true, true),
                _ => (true, true, true, true),
            };
            if up {
                rect(mid_x - thin / 2., y, thin, mid_y - y, 1., window);
            }
            if down {
                rect(mid_x - thin / 2., mid_y, thin, y + h - mid_y, 1., window);
            }
            if left {
                rect(x, mid_y - thin / 2., mid_x - x, thin, 1., window);
            }
            if right {
                rect(mid_x, mid_y - thin / 2., x + w - mid_x, thin, 1., window);
            }
        }
        0x2550 => {
            rect(x, mid_y - 2. - thin / 2., w, thin, 1., window);
            rect(x, mid_y + 2. - thin / 2., w, thin, 1., window);
        }
        0x2551 => {
            rect(mid_x - 2. - thin / 2., y, thin, h, 1., window);
            rect(mid_x + 2. - thin / 2., y, thin, h, 1., window);
        }
        // Rounded corners: an arm, a quarter circle, the other arm.
        0x256D..=0x2570 => {
            let r = w.min(h) / 2.;
            let (start, center, from, end) = match code {
                // ╭ from the bottom up, around to the right edge.
                0x256D => (
                    (mid_x, y + h),
                    (mid_x + r, mid_y + r),
                    std::f32::consts::PI,
                    (x + w, mid_y),
                ),
                // ╮ from the left edge, around and down.
                0x256E => (
                    (x, mid_y),
                    (mid_x - r, mid_y + r),
                    -std::f32::consts::FRAC_PI_2,
                    (mid_x, y + h),
                ),
                // ╯ from the top down, around to the left edge.
                0x256F => ((mid_x, y), (mid_x - r, mid_y - r), 0., (x, mid_y)),
                // ╰ from the right edge, around and up.
                _ => (
                    (x + w, mid_y),
                    (mid_x + r, mid_y - r),
                    std::f32::consts::FRAC_PI_2,
                    (mid_x, y),
                ),
            };
            let mut path = PathBuilder::stroke(px(thin));
            path.move_to(point(px(start.0), px(start.1)));
            for step in 0..=12 {
                let angle = from + std::f32::consts::FRAC_PI_2 * step as f32 / 12.;
                path.line_to(point(
                    px(center.0 + r * angle.cos()),
                    px(center.1 + r * angle.sin()),
                ));
            }
            path.line_to(point(px(end.0), px(end.1)));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        0x2580 => rect(x, y, w, h / 2., 1., window),
        // ▁ to ▇: the lower eighths.
        0x2581..=0x2587 => {
            let part = h * (code - 0x2580) as f32 / 8.;
            rect(x, y + h - part, w, part, 1., window);
        }
        0x2588 => rect(x, y, w, h, 1., window),
        // ▉ to ▏: the left eighths.
        0x2589..=0x258F => rect(x, y, w * (8 - (code - 0x2588)) as f32 / 8., h, 1., window),
        0x2590 => rect(x + w / 2., y, w / 2., h, 1., window),
        0x2591 => rect(x, y, w, h, 0.25, window),
        0x2592 => rect(x, y, w, h, 0.5, window),
        0x2593 => rect(x, y, w, h, 0.75, window),
        0x2594 => rect(x, y, w, h / 8., 1., window),
        0x2595 => rect(x + w * 7. / 8., y, w / 8., h, 1., window),
        // The quadrants: which of upper left, upper right, lower left and
        // lower right each fills.
        0x2596..=0x259F => {
            let [ul, ur, ll, lr] = match code {
                0x2596 => [false, false, true, false],
                0x2597 => [false, false, false, true],
                0x2598 => [true, false, false, false],
                0x2599 => [true, false, true, true],
                0x259A => [true, false, false, true],
                0x259B => [true, true, true, false],
                0x259C => [true, true, false, true],
                0x259D => [false, true, false, false],
                0x259E => [false, true, true, false],
                _ => [false, true, true, true],
            };
            let (hw, hh) = (w / 2., h / 2.);
            for (on, qx, qy) in [
                (ul, x, y),
                (ur, x + hw, y),
                (ll, x, y + hh),
                (lr, x + hw, y + hh),
            ] {
                if on {
                    rect(qx, qy, hw, hh, 1., window);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_lines_corners_and_blocks_but_not_text() {
        for ch in ['─', '│', '╭', '╯', '┼', '█', '▀', '░', '▚'] {
            assert!(is_geometric(ch), "{ch}");
        }
        for ch in ['a', '╳', '▶', '•', ' '] {
            assert!(!is_geometric(ch), "{ch}");
        }
    }
}
