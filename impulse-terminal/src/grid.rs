//! Grid snapshot types for rendering.
//!
//! These types are the interface between the terminal backend and platform
//! renderers. They are deliberately simple value types with no dependencies on
//! alacritty_terminal, so frontends never need to link against it.

use serde::{Deserialize, Serialize};

/// RGB color value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Cell attribute flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CellFlags(u16);

impl CellFlags {
    pub const NONE: Self = Self(0);
    pub const BOLD: Self = Self(1 << 0);
    pub const ITALIC: Self = Self(1 << 1);
    pub const UNDERLINE: Self = Self(1 << 2);
    pub const STRIKETHROUGH: Self = Self(1 << 3);
    pub const DIM: Self = Self(1 << 4);
    pub const INVERSE: Self = Self(1 << 5);
    pub const HIDDEN: Self = Self(1 << 6);
    pub const WIDE_CHAR: Self = Self(1 << 7);
    pub const WIDE_CHAR_SPACER: Self = Self(1 << 8);
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 9);
    pub const UNDERCURL: Self = Self(1 << 10);
    pub const DOTTED_UNDERLINE: Self = Self(1 << 11);
    pub const DASHED_UNDERLINE: Self = Self(1 << 12);
    pub const HYPERLINK: Self = Self(1 << 13);

    #[inline]
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }
}

impl std::ops::BitOr for CellFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// A single cell in the terminal grid, ready for rendering.
#[derive(Clone, Debug, Serialize)]
pub struct StyledCell {
    pub character: char,
    pub fg: RgbColor,
    pub bg: RgbColor,
    pub flags: CellFlags,
}

/// Cursor shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
    HollowBlock,
    Hidden,
}

/// Current cursor state for rendering.
#[derive(Clone, Debug, Serialize)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub visible: bool,
}

/// Complete snapshot of the visible terminal grid, ready for rendering.
#[derive(Clone, Debug, Serialize)]
pub struct GridSnapshot {
    pub cells: Vec<Vec<StyledCell>>,
    pub cursor: CursorState,
    pub has_selection: bool,
    /// For each row, ranges of selected columns (if any).
    pub selection_ranges: Vec<(usize, usize, usize)>,
    pub cols: usize,
    pub lines: usize,
    pub mode: TerminalMode,
}

/// Terminal mode flags relevant to renderers and input handling.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct TerminalMode {
    pub show_cursor: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub mouse_report_click: bool,
    pub mouse_motion: bool,
    pub mouse_drag: bool,
    pub mouse_sgr: bool,
    pub bracketed_paste: bool,
    pub focus_in_out: bool,
    pub alt_screen: bool,
    pub line_wrap: bool,
}
