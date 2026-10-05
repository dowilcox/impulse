//! Text transcripts of a terminal's scrollback, used to bring a terminal's
//! recent output back after a restart.
//!
//! A transcript is the last N grid rows as text, optionally with SGR
//! sequences for colors and styles. Rows joined by a soft wrap are emitted as
//! one line so the restored terminal re-wraps them at its own width; trailing
//! blanks are dropped. Feeding a transcript back through the VT parser
//! reproduces the output (not the screen state of full-screen programs).

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// About the last `max_rows` rows of output (ending at the last row with
/// content at or above the cursor), as text. A wrapped line is never cut:
/// the start moves up to its first row. With `with_sgr`, colors and styles
/// are included as SGR sequences and the text ends with a reset.
pub fn transcript<T>(term: &Term<T>, max_rows: usize, with_sgr: bool) -> String {
    transcript_until(term, None, max_rows, with_sgr)
}

/// Like `transcript`, ending at grid line `last_line` (inclusive) when given,
/// e.g. the last command's output so an idle prompt isn't included.
pub fn transcript_until<T>(
    term: &Term<T>,
    last_line: Option<i32>,
    max_rows: usize,
    with_sgr: bool,
) -> String {
    let grid = term.grid();
    let top = -(grid.history_size() as i32);
    let columns = grid.columns();
    let mut bottom = grid.cursor.point.line.0.min(grid.screen_lines() as i32 - 1);
    if let Some(last_line) = last_line {
        bottom = bottom.min(last_line);
    }
    while bottom >= top && !(0..columns).any(|c| is_visible(&grid[Line(bottom)][Column(c)])) {
        bottom -= 1;
    }
    if max_rows == 0 || bottom < top {
        return String::new();
    }
    let mut start = (bottom - max_rows as i32 + 1).max(top);
    while start > top && wraps(&grid[Line(start - 1)][Column(columns - 1)]) {
        start -= 1;
    }

    let mut out = String::new();
    let mut style = Style::default();
    for line in start..=bottom {
        let row = &grid[Line(line)];
        let soft_wrapped = line < bottom && wraps(&row[Column(columns - 1)]);
        // A wrapped row continues on the next one, so its full width counts;
        // otherwise stop at the last visible cell.
        let end = if soft_wrapped {
            columns
        } else {
            (0..columns)
                .rev()
                .find(|&c| is_visible(&row[Column(c)]))
                .map_or(0, |c| c + 1)
        };
        for column in 0..end {
            let cell = &row[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if with_sgr {
                let next = Style::of(cell);
                if next != style {
                    out.push_str(&next.sgr());
                    style = next;
                }
            }
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if !soft_wrapped {
            if with_sgr && style != Style::default() {
                // Don't carry a background color past the end of the line.
                out.push_str("\x1b[0m");
                style = Style::default();
            }
            out.push_str("\r\n");
        }
    }
    // Drop trailing empty lines (the blank space below the last output).
    while out.ends_with("\r\n\r\n") {
        out.truncate(out.len() - 2);
    }
    if with_sgr && style != Style::default() {
        out.push_str("\x1b[0m");
    }
    out
}

fn wraps(cell: &Cell) -> bool {
    cell.flags.contains(Flags::WRAPLINE)
}

fn is_visible(cell: &Cell) -> bool {
    (cell.c != ' ' && cell.c != '\0')
        || !matches!(cell.bg, Color::Named(NamedColor::Background))
        || cell
            .flags
            .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES)
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color,
    bg: Color,
    flags: Flags,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            flags: Flags::empty(),
        }
    }
}

impl Style {
    const STYLE_FLAGS: Flags = Flags::BOLD
        .union(Flags::DIM)
        .union(Flags::ITALIC)
        .union(Flags::ALL_UNDERLINES)
        .union(Flags::INVERSE)
        .union(Flags::HIDDEN)
        .union(Flags::STRIKEOUT);

    fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags & Self::STYLE_FLAGS,
        }
    }

    /// A full SGR (reset, then this style), so it never depends on state.
    fn sgr(&self) -> String {
        let mut params = vec!["0".to_string()];
        let f = self.flags;
        if f.contains(Flags::BOLD) {
            params.push("1".into());
        }
        if f.contains(Flags::DIM) || is_dim(self.fg) {
            params.push("2".into());
        }
        if f.contains(Flags::ITALIC) {
            params.push("3".into());
        }
        if f.intersects(Flags::ALL_UNDERLINES) {
            params.push("4".into());
        }
        if f.contains(Flags::INVERSE) {
            params.push("7".into());
        }
        if f.contains(Flags::HIDDEN) {
            params.push("8".into());
        }
        if f.contains(Flags::STRIKEOUT) {
            params.push("9".into());
        }
        params.extend(color_param(self.fg, false));
        params.extend(color_param(self.bg, true));
        format!("\x1b[{}m", params.join(";"))
    }
}

fn is_dim(color: Color) -> bool {
    matches!(color, Color::Named(n) if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
        .contains(&(n as usize)) || n == NamedColor::DimForeground)
}

fn color_param(color: Color, background: bool) -> Option<String> {
    let base = if background { 40 } else { 30 };
    match color {
        Color::Named(named) => {
            let index = named as usize;
            match index {
                0..=7 => Some((base + index).to_string()),
                8..=15 => Some((base + 60 + index - 8).to_string()),
                _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                    .contains(&index) =>
                {
                    Some((base + index - NamedColor::DimBlack as usize).to_string())
                }
                // Foreground, background, cursor and their bright/dim
                // variants: the terminal's default.
                _ => None,
            }
        }
        Color::Indexed(index) => Some(format!("{};5;{}", base + 8, index)),
        Color::Spec(rgb) => Some(format!("{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    fn term_with(columns: usize, lines: usize, output: &[u8]) -> Term<VoidListener> {
        let size = TermSize::new(columns, lines);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, output);
        term
    }

    #[test]
    fn plain_lines_drop_trailing_blanks() {
        let term = term_with(20, 5, b"one\r\ntwo   \r\nthree\r\n");
        assert_eq!(transcript(&term, 100, false), "one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn soft_wrapped_lines_are_joined() {
        // 25 chars on a 10-column terminal wrap onto three rows.
        let term = term_with(10, 6, b"abcdefghijklmnopqrstuvwxy\r\nnext\r\n");
        assert_eq!(
            transcript(&term, 100, false),
            "abcdefghijklmnopqrstuvwxy\r\nnext\r\n"
        );
    }

    #[test]
    fn keeps_the_last_rows_without_cutting_a_wrapped_line() {
        let term = term_with(10, 4, b"1\r\n2\r\n3\r\nabcdefghijklmno\r\n");
        // Rows: "1","2","3","abcdefghij","klmno", then the empty cursor row.
        assert_eq!(transcript(&term, 2, false), "abcdefghijklmno\r\n");
        // One row would start at the wrap continuation: take the whole line.
        assert_eq!(transcript(&term, 1, false), "abcdefghijklmno\r\n");
        assert_eq!(transcript(&term, 3, false), "3\r\nabcdefghijklmno\r\n");
        assert_eq!(transcript(&term, 0, false), "");
    }

    #[test]
    fn colors_and_styles_become_sgr() {
        let term = term_with(
            30,
            4,
            b"\x1b[1;31mred\x1b[0m plain \x1b[38;5;208mx\x1b[48;2;1;2;3my\x1b[0m\r\n",
        );
        let text = transcript(&term, 10, true);
        assert_eq!(
            text,
            "\x1b[0;1;31mred\x1b[0m plain \x1b[0;38;5;208mx\x1b[0;38;5;208;48;2;1;2;3my\x1b[0m\r\n"
        );
    }

    #[test]
    fn can_end_before_the_cursor() {
        let term = term_with(20, 5, b"out 1\r\nout 2\r\n$ ");
        assert_eq!(
            transcript_until(&term, Some(1), 100, false),
            "out 1\r\nout 2\r\n"
        );
        assert_eq!(transcript_until(&term, Some(0), 100, false), "out 1\r\n");
        assert_eq!(transcript(&term, 100, false), "out 1\r\nout 2\r\n$\r\n");
    }

    #[test]
    fn wide_characters_are_emitted_once() {
        let term = term_with(10, 3, "日本\r\n".as_bytes());
        assert_eq!(transcript(&term, 10, false), "日本\r\n");
    }

    #[test]
    fn round_trips_through_the_parser() {
        let original = term_with(12, 4, b"\x1b[32mok\x1b[0m done\r\nline two\r\n");
        let text = transcript(&original, 100, true);
        let restored = term_with(12, 4, text.as_bytes());
        assert_eq!(transcript(&restored, 100, true), text);
    }
}
