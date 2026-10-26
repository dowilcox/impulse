//! Coding agents in Impulse's terminals (the Swift `ImpulseKit/Agents`
//! folder): telling which agent a process is, what it's doing from what a
//! terminal sees and what its hooks report, setting up its hooks and
//! Impulse's MCP server in its own configuration, and the tasks agents ask
//! for.

pub mod foundation_json;
pub mod hook_events;
pub mod hook_installer;
pub mod jsonc_edit;
pub mod known_agents;
pub mod mcp_installer;
pub mod state_machine;
pub mod task_requests;
