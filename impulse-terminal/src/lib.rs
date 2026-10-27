//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend. Frontends only
//! need to render a grid of styled cells and forward input events.

mod backend;
mod blocks;
mod buffer;
mod config;
mod event;
mod graphics;
mod grid;
mod history;
pub mod osc_scanner;
mod search;
mod transcript;

pub use backend::{
    BlockOverlay, BlockOverlayRegion, CommandBlockFlags, SelectionKind, TerminalBackend,
};
pub use blocks::{TerminalBlockId, TerminalCommandBlock, TerminalCommandBlockSummary};
pub use buffer::{
    buffer_size, write_cell, write_header, HighlightRange, CELL_STRIDE, EXTRA_IMAGE_CELL,
    EXTRA_UNDERLINE_COLOR, EXTRA_ZERO_WIDTH, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};
pub use config::{TerminalColors, TerminalConfig};
pub use event::TerminalEvent;
pub use graphics::{ImageLayer, ImagePlacement, TerminalImage};
pub use grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
pub use history::{
    CommandHistoryContext, CommandHistoryMatchKind, CommandHistoryQuery, CommandHistoryRecord,
    CommandHistorySearchResult, CommandHistoryStore,
};
pub use search::{SearchResult, SearchStats};
pub use transcript::{transcript, transcript_until};
