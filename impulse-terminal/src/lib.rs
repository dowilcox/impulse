//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend that wraps
//! `alacritty_terminal`. Frontends only need to render a grid of styled cells
//! and forward input events.

mod backend;
mod config;
mod event;
mod grid;

pub use backend::{SelectionKind, TerminalBackend};
pub use config::{TerminalColors, TerminalConfig};
pub use event::TerminalEvent;
pub use grid::{CellFlags, CursorShape, CursorState, GridSnapshot, RgbColor, StyledCell, TerminalMode};
