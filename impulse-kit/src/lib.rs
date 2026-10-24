//! Impulse's pure logic, ported to Rust for the full-Rust migration
//! (docs/plans/rust-migration.md). Mirrors the Swift `ImpulseKit` target:
//! no UI, no platform frameworks, so everything here is testable on any OS.
//!
//! The first modules are the Rust originals from tag `v0.29.0`, checked
//! against the golden fixtures the Swift port is checked against
//! (`impulse-macos/Tests/ImpulseKitTests/Fixtures/`).

pub mod accessible_text;
pub mod agents;
pub mod buffer_edits;
pub mod child_process;
pub mod chrome;
pub mod close_risk;
pub mod command_palette;
pub mod completion;
pub mod css_color;
pub mod directory_lister;
pub mod document_symbols;
pub mod drag_reorder;
pub mod file_tree;
pub mod fuzzy;
pub mod git;
pub mod glob;
pub mod grouped_order;
pub mod history;
pub mod key_encoder;
pub mod keybindings;
pub mod kitty_keyboard;
pub mod layout_tree;
pub mod line_edits;
pub mod markdown_preview;
pub mod markdown_theme;
pub mod menus;
pub mod monaco_theme;
pub mod occurrences;
pub mod paths;
pub mod problems;
pub mod project;
pub mod project_replace;
pub mod recent_workspaces;
pub mod review_comments;
pub mod save_command;
pub mod search_result;
pub mod session;
pub mod settings;
pub mod shell;
pub mod shell_parser;
pub mod svg_preview;
pub mod terminal_find;
pub mod terminal_hints;
pub mod terminal_paths;
pub mod text_file;
pub mod theme;
pub mod trust;
pub mod update_checker;
pub mod uri;
