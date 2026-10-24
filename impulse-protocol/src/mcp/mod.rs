//! `impulse mcp`: the MCP server agents start themselves, and the tools it
//! offers them.

pub mod server;
pub mod tools;

pub use server::{McpServer, PROTOCOL_VERSIONS, STATELESS_VERSIONS};
pub use tools::{McpContext, McpTool, McpToolError};
