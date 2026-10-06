//! Binary buffer format for efficient grid snapshot transport across FFI.
//!
//! Layout:
//!   Header (variable size):
//!     [0..2)   cols (u16 LE)
//!     [2..4)   lines (u16 LE)
//!     [4..6)   cursor row (u16 LE)
//!     [6..8)   cursor col (u16 LE)
//!     [8]      cursor shape (u8)
//!     [9]      cursor visible (u8: 0/1)
//!     [10..12) mode flags (u16 LE)
//!     [12..14) selection range count N (u16 LE)
//!     [14..16) search match range count M (u16 LE)
//!     [16 .. 16+N*6)  selection ranges:
//!       row(u16) | start_col(u16) | end_col(u16), inclusive
//!     [16+N*6 .. 16+N*6+M*6)  search match ranges (same format)
//!   Cell data (row-major, 12 bytes per cell):
//!     [0..4)   character (u32 LE, UTF-32 codepoint)
//!     [4..7)   fg RGB
//!     [7..10)  bg RGB
//!     [10..12) flags (u16 LE, CellFlags)
//!   Extras (after the cells, present when the snapshot is longer):
//!     [0..4)   entry count E (u32 LE)
//!     E entries: cell index (u32 LE) | kind (u8) | length L (u8) | L bytes
//!       kind 1: zero-width characters drawn with the cell (UTF-8)
//!       kind 2: underline color (RGB)
//!     Entries that don't fit the buffer are dropped.

#[cfg(test)]
use crate::grid::CursorShape;
use crate::grid::{CellFlags, CursorState, RgbColor, TerminalMode};

/// Bytes per cell in the binary buffer.
pub const CELL_STRIDE: usize = 12;

/// Fixed header size (before variable-length selection/search ranges).
pub const FIXED_HEADER_SIZE: usize = 16;

/// Bytes per range entry (row u16 + start_col u16 + inclusive end_col u16).
pub const RANGE_ENTRY_SIZE: usize = 6;

/// Extras entry kinds.
pub const EXTRA_ZERO_WIDTH: u8 = 1;
pub const EXTRA_UNDERLINE_COLOR: u8 = 2;

/// Room reserved for extras: the count plus 8 bytes per cell, enough for a
/// combining mark on every cell; past that, entries are dropped.
pub fn extras_capacity(cols: u16, lines: u16) -> usize {
    4 + cols as usize * lines as usize * 8
}

/// Write the extras after the cells at `offset`. Returns the end offset.
pub fn write_extras(buf: &mut [u8], offset: usize, extras: &[(u32, u8, Vec<u8>)]) -> usize {
    if buf.len() < offset + 4 {
        return offset;
    }
    let mut pos = offset + 4;
    let mut count: u32 = 0;
    for (index, kind, payload) in extras {
        let len = payload.len().min(255);
        if pos + 6 + len > buf.len() {
            break;
        }
        buf[pos..pos + 4].copy_from_slice(&index.to_le_bytes());
        buf[pos + 4] = *kind;
        buf[pos + 5] = len as u8;
        buf[pos + 6..pos + 6 + len].copy_from_slice(&payload[..len]);
        pos += 6 + len;
        count += 1;
    }
    buf[offset..offset + 4].copy_from_slice(&count.to_le_bytes());
    pos
}

/// A range highlight (selection or search match). `end_col` is inclusive.
#[derive(Clone, Copy, Debug)]
pub struct HighlightRange {
    pub row: u16,
    pub start_col: u16,
    pub end_col: u16,
}

/// Calculate the buffer size needed for a grid of the given dimensions.
pub fn buffer_size(cols: u16, lines: u16, selection_count: u16, search_count: u16) -> usize {
    FIXED_HEADER_SIZE
        + (selection_count as usize + search_count as usize) * RANGE_ENTRY_SIZE
        + (cols as usize * lines as usize * CELL_STRIDE)
}

/// Write the grid header into the buffer. Returns the offset where cell data begins.
pub fn write_header(
    buf: &mut [u8],
    cols: u16,
    lines: u16,
    cursor: &CursorState,
    mode: TerminalMode,
    selection_ranges: &[HighlightRange],
    search_ranges: &[HighlightRange],
) -> usize {
    let sel_count = selection_ranges.len() as u16;
    let search_count = search_ranges.len() as u16;

    buf[0..2].copy_from_slice(&cols.to_le_bytes());
    buf[2..4].copy_from_slice(&lines.to_le_bytes());
    buf[4..6].copy_from_slice(&(cursor.row as u16).to_le_bytes());
    buf[6..8].copy_from_slice(&(cursor.col as u16).to_le_bytes());
    buf[8] = cursor.shape as u8;
    buf[9] = cursor.visible as u8;
    buf[10..12].copy_from_slice(&mode.bits().to_le_bytes());
    buf[12..14].copy_from_slice(&sel_count.to_le_bytes());
    buf[14..16].copy_from_slice(&search_count.to_le_bytes());

    let mut offset = FIXED_HEADER_SIZE;
    for range in selection_ranges {
        buf[offset..offset + 2].copy_from_slice(&range.row.to_le_bytes());
        buf[offset + 2..offset + 4].copy_from_slice(&range.start_col.to_le_bytes());
        buf[offset + 4..offset + 6].copy_from_slice(&range.end_col.to_le_bytes());
        offset += RANGE_ENTRY_SIZE;
    }
    for range in search_ranges {
        buf[offset..offset + 2].copy_from_slice(&range.row.to_le_bytes());
        buf[offset + 2..offset + 4].copy_from_slice(&range.start_col.to_le_bytes());
        buf[offset + 4..offset + 6].copy_from_slice(&range.end_col.to_le_bytes());
        offset += RANGE_ENTRY_SIZE;
    }
    offset
}

/// Write a single cell into the buffer at the given offset.
#[inline]
pub fn write_cell(
    buf: &mut [u8],
    offset: usize,
    ch: char,
    fg: RgbColor,
    bg: RgbColor,
    flags: CellFlags,
) {
    let cp = ch as u32;
    buf[offset..offset + 4].copy_from_slice(&cp.to_le_bytes());
    buf[offset + 4] = fg.r;
    buf[offset + 5] = fg.g;
    buf[offset + 6] = fg.b;
    buf[offset + 7] = bg.r;
    buf[offset + 8] = bg.g;
    buf[offset + 9] = bg.b;
    buf[offset + 10..offset + 12].copy_from_slice(&flags.bits().to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_size() {
        assert_eq!(
            buffer_size(80, 24, 0, 0),
            FIXED_HEADER_SIZE + 80 * 24 * CELL_STRIDE
        );
        assert_eq!(
            buffer_size(80, 24, 2, 1),
            FIXED_HEADER_SIZE + 3 * RANGE_ENTRY_SIZE + 80 * 24 * CELL_STRIDE
        );
    }

    #[test]
    fn test_write_header_roundtrip() {
        let cols: u16 = 80;
        let lines: u16 = 24;
        let cursor = CursorState {
            row: 5,
            col: 10,
            shape: CursorShape::Beam,
            visible: true,
        };
        let mode = TerminalMode::SHOW_CURSOR | TerminalMode::APP_CURSOR;
        let sel = vec![HighlightRange {
            row: 3,
            start_col: 5,
            end_col: 20,
        }];

        let buf_size = buffer_size(cols, lines, sel.len() as u16, 0);
        let mut buf = vec![0u8; buf_size];
        let cell_offset = write_header(&mut buf, cols, lines, &cursor, mode, &sel, &[]);

        // Read back header
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), 80);
        assert_eq!(u16::from_le_bytes([buf[2], buf[3]]), 24);
        assert_eq!(u16::from_le_bytes([buf[4], buf[5]]), 5); // cursor row
        assert_eq!(u16::from_le_bytes([buf[6], buf[7]]), 10); // cursor col
        assert_eq!(buf[8], CursorShape::Beam as u8);
        assert_eq!(buf[9], 1); // visible
        assert_eq!(u16::from_le_bytes([buf[12], buf[13]]), 1); // 1 selection range
        assert_eq!(u16::from_le_bytes([buf[14], buf[15]]), 0); // 0 search ranges

        // Selection range
        assert_eq!(u16::from_le_bytes([buf[16], buf[17]]), 3); // row
        assert_eq!(u16::from_le_bytes([buf[18], buf[19]]), 5); // start_col
        assert_eq!(u16::from_le_bytes([buf[20], buf[21]]), 20); // end_col

        assert_eq!(cell_offset, FIXED_HEADER_SIZE + RANGE_ENTRY_SIZE);
    }

    #[test]
    fn test_extras_roundtrip_and_truncation() {
        let extras = vec![
            (3u32, EXTRA_ZERO_WIDTH, "\u{301}".as_bytes().to_vec()),
            (7u32, EXTRA_UNDERLINE_COLOR, vec![255, 0, 128]),
        ];
        let mut buf = vec![0u8; 64];
        let end = write_extras(&mut buf, 10, &extras);
        assert_eq!(u32::from_le_bytes([buf[10], buf[11], buf[12], buf[13]]), 2);
        assert_eq!(u32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]), 3);
        assert_eq!(buf[18], EXTRA_ZERO_WIDTH);
        assert_eq!(buf[19], 2);
        assert_eq!(&buf[20..22], "\u{301}".as_bytes());
        assert_eq!(buf[26], EXTRA_UNDERLINE_COLOR);
        assert_eq!(&buf[28..31], &[255, 0, 128]);
        assert_eq!(end, 31);

        // Only the first entry fits: the count says so.
        let mut small = vec![0u8; 10 + 4 + 8];
        let end = write_extras(&mut small, 10, &extras);
        assert_eq!(
            u32::from_le_bytes([small[10], small[11], small[12], small[13]]),
            1
        );
        assert_eq!(end, 22);
    }

    #[test]
    fn test_write_cell_roundtrip() {
        let mut buf = [0u8; CELL_STRIDE];
        let fg = RgbColor::new(255, 128, 0);
        let bg = RgbColor::new(0, 0, 30);
        let flags = CellFlags::BOLD | CellFlags::ITALIC;

        write_cell(&mut buf, 0, 'A', fg, bg, flags);

        assert_eq!(
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            'A' as u32
        );
        assert_eq!(buf[4], 255); // fg.r
        assert_eq!(buf[5], 128); // fg.g
        assert_eq!(buf[6], 0); // fg.b
        assert_eq!(buf[7], 0); // bg.r
        assert_eq!(buf[8], 0); // bg.g
        assert_eq!(buf[9], 30); // bg.b
        assert_eq!(
            u16::from_le_bytes([buf[10], buf[11]]),
            (CellFlags::BOLD | CellFlags::ITALIC).bits()
        );
    }
}
