//! Impulse's pure logic, ported to Rust for the full-Rust migration
//! (docs/plans/rust-migration.md). Mirrors the Swift `ImpulseKit` target:
//! no UI, no platform frameworks, so everything here is testable on any OS.
//!
//! The first modules are the Rust originals from tag `v0.29.0`, checked
//! against the golden fixtures the Swift port is checked against
//! (`impulse-macos/Tests/ImpulseKitTests/Fixtures/`).

pub mod buffer_edits;
pub mod chrome;
pub mod close_risk;
pub mod command_palette;
pub mod css_color;
pub mod directory_lister;
pub mod document_symbols;
pub mod drag_reorder;
pub mod file_tree;
pub mod fuzzy;
pub mod glob;
pub mod grouped_order;
pub mod layout_tree;
pub mod line_edits;
pub mod markdown_preview;
pub mod markdown_theme;
pub mod monaco_theme;
pub mod paths;
pub mod problems;
pub mod project_replace;
pub mod review_comments;
pub mod save_command;
pub mod search_result;
pub mod shell_parser;
pub mod svg_preview;
pub mod text_file;
pub mod theme;
pub mod trust;
pub mod update_checker;
pub mod uri;
