//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend. Frontends only
//! need to render a grid of styled cells and forward input events.

mod backend;
mod blocks;
mod buffer;
mod config;
mod event;
mod grid;
pub mod osc_scanner;
mod search;

pub use backend::{SelectionKind, TerminalBackend};
pub use blocks::{TerminalBlockId, TerminalCommandBlock};
pub use buffer::{
    buffer_size, write_cell, write_header, HighlightRange, CELL_STRIDE, FIXED_HEADER_SIZE,
    RANGE_ENTRY_SIZE,
};
pub use config::{TerminalColors, TerminalConfig};
pub use event::TerminalEvent;
pub use grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
pub use search::SearchResult;
