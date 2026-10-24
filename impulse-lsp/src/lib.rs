//! Impulse's language-server client, the Rust port of the Swift
//! `ImpulseLSP` target (docs/plans/rust-migration.md). It has no UI and no
//! executor of its own: requests are futures, server notifications arrive
//! on a channel, and the process is read on its own thread.

pub mod client;
pub mod completion;
pub mod config;
pub mod edits;
pub mod framing;
pub mod managed;
pub mod registry;
pub mod signature;
pub mod workspace_edit;

pub use client::{path_uri, LspClient, ServerEvent};
pub use config::{LspConfig, ServerConfig};
pub use registry::resolve_command;
