//! Impulse's pure logic, ported to Rust for the full-Rust migration
//! (docs/plans/rust-migration.md). Mirrors the Swift `ImpulseKit` target:
//! no UI, no platform frameworks, so everything here is testable on any OS.
//!
//! The first modules are the Rust originals from tag `v0.29.0`, checked
//! against the golden fixtures the Swift port is checked against
//! (`impulse-macos/Tests/ImpulseKitTests/Fixtures/`).

pub mod chrome;
pub mod close_risk;
pub mod glob;
pub mod shell_parser;
pub mod theme;
