# neuron-mcp

> Model Context Protocol (MCP) bridge for neuron

[![crates.io](https://img.shields.io/crates/v/neuron-mcp.svg)](https://crates.io/crates/neuron-mcp)
[![docs.rs](https://docs.rs/neuron-mcp/badge.svg)](https://docs.rs/neuron-mcp)
[![license](https://img.shields.io/crates/l/neuron-mcp.svg)](LICENSE-MIT)

## Overview

`neuron-mcp` bridges the [Model Context Protocol](https://modelcontextprotocol.io) ecosystem to
neuron's `ToolRegistry`. Two independent components:

- **`McpClient`** — connects to an MCP server, discovers its tools, and returns them as
  `Arc<dyn ToolDyn>` for use in a `ToolRegistry`
- **`McpServer`** — wraps a `ToolRegistry` and exposes its tools over the MCP protocol via stdio

Backed by the [`rmcp`](https://crates.io/crates/rmcp) library.

## Exports

- **`McpClient`** — `connect_stdio(Command)`, `connect_sse(url)`, `discover_tools()`,
  `discover_tools_with_aliases(aliases)`, `close()`
- **`McpServer`** — `new(registry, name, version)`, `serve_stdio()`
- **`McpError`** — `Connection(String)`, `Protocol(String)`

## Usage

```toml
[dependencies]
neuron-mcp = "0.4"
neuron-tool = "0.4"
tokio = { version = "1", features = ["full"] }
```

### Consuming an MCP server's tools

```rust,no_run
use neuron_mcp::McpClient;
use neuron_tool::ToolRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = tokio::process::Command::new("uvx");
    cmd.args(["mcp-server-fetch"]);

    let client = McpClient::connect_stdio(cmd).await?;
    let tools = client.discover_tools().await?;

    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    // registry now contains all tools from the MCP server
    client.close().await?;
    Ok(())
}
```

### Exposing neuron tools as an MCP server

```rust,no_run
use neuron_mcp::McpServer;
use neuron_tool::ToolRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ToolRegistry::new(); // populate as needed
    let server = McpServer::new(registry, "my-server", "0.1.0");
    server.serve_stdio().await?;
    Ok(())
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
