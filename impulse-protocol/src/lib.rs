//! The conversation between Impulse and the programs in its terminals, the
//! Rust port of the Swift `ImpulseProtocol` target (docs/plans/rust-migration.md):
//!
//! - [`control`]: the control socket's messages, one JSON line each way,
//!   byte for byte the Swift app's, so either app answers either `impulse`.
//! - [`args`]: `impulse`'s command line turned into a request.
//! - [`command`]: a request read as a typed [`Command`], for the app's
//!   control server.
//! - [`client`]: the socket client `impulse` and `impulse mcp` send with.
//! - [`mcp`]: the MCP server agents start (`impulse mcp`) and its tools.
//! - [`agent_tools`]: what the app answers agents with (task listings,
//!   turns, review comments, problems, command blocks, unsaved files,
//!   project settings), as text and as JSON.
//!
//! The control server itself (accepting connections, who the caller is)
//! belongs to the app.

pub mod agent_tools;
pub mod args;
pub mod client;
pub mod command;
pub mod control;
pub mod json;
pub mod mcp;

pub use client::{ControlChannel, ControlClient, Failure};
pub use command::Command;
pub use control::{ControlRequest, ControlResponse};
