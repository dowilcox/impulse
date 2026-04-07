//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend. Frontends only
//! need to render a grid of styled cells and forward input events.

mod grid;

pub use grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
