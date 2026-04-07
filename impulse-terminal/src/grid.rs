//! Platform-agnostic grid types for rendering.
//!
//! These types are the interface between the terminal backend and platform
//! renderers. They have no dependencies on alacritty_terminal, so frontends
//! never need to link against it.

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

bitflags::bitflags! {
    /// Cell attribute flags (transmitted as u16 in the binary buffer).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
    pub struct CellFlags: u16 {
        const BOLD              = 1 << 0;
        const ITALIC            = 1 << 1;
        const UNDERLINE         = 1 << 2;
        const STRIKETHROUGH     = 1 << 3;
        const DIM               = 1 << 4;
        const INVERSE           = 1 << 5;
        const HIDDEN            = 1 << 6;
        const WIDE_CHAR         = 1 << 7;
        const WIDE_CHAR_SPACER  = 1 << 8;
        const DOUBLE_UNDERLINE  = 1 << 9;
        const UNDERCURL         = 1 << 10;
        const DOTTED_UNDERLINE  = 1 << 11;
        const DASHED_UNDERLINE  = 1 << 12;
    }
}

/// Cursor shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CursorShape {
    Block = 0,
    Beam = 1,
    Underline = 2,
    HollowBlock = 3,
    Hidden = 4,
}

/// Current cursor state for rendering.
#[derive(Clone, Debug, Serialize)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub visible: bool,
}

bitflags::bitflags! {
    /// Terminal mode flags relevant to renderers and input handling.
    /// Transmitted as u16 in the binary buffer header.
    #[derive(Clone, Copy, Debug, Default, Serialize)]
    pub struct TerminalMode: u16 {
        const SHOW_CURSOR         = 1 << 0;
        const APP_CURSOR          = 1 << 1;
        const APP_KEYPAD          = 1 << 2;
        const MOUSE_REPORT_CLICK  = 1 << 3;
        const MOUSE_MOTION        = 1 << 4;
        const MOUSE_DRAG          = 1 << 5;
        const MOUSE_SGR           = 1 << 6;
        const BRACKETED_PASTE     = 1 << 7;
        const FOCUS_IN_OUT        = 1 << 8;
        const ALT_SCREEN          = 1 << 9;
        const LINE_WRAP           = 1 << 10;
    }
}
