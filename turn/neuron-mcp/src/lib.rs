#![deny(missing_docs)]
//! MCP client and server bridging MCP tools with neuron ToolRegistry.
//!
//! Two independent components:
//!
//! - [`McpClient`] connects to an MCP server, discovers its tools, and wraps
//!   each as a [`ToolDyn`](neuron_tool::ToolDyn) for use in a
//!   [`ToolRegistry`](neuron_tool::ToolRegistry).
//! - [`McpServer`] wraps a [`ToolRegistry`](neuron_tool::ToolRegistry) and
//!   exposes its tools via the MCP protocol over stdio.

pub mod client;
pub mod error;
pub mod server;

pub use client::McpClient;
pub use error::McpError;
pub use server::McpServer;
