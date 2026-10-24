//! Impulse's language-server client, the Rust port of the Swift
//! `ImpulseLSP` target (docs/plans/rust-migration.md). It has no UI and no
//! executor of its own: requests are futures, server notifications arrive
//! on a channel, and the process is read on its own thread.

pub mod client;
pub mod completion;
pub mod framing;
pub mod registry;

pub use client::{path_uri, LspClient, ServerEvent};
pub use registry::{primary_server, resolve_command, ServerSpec};
