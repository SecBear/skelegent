# skg-mcp

> Model Context Protocol (MCP) bridge for skelegent

[![crates.io](https://img.shields.io/crates/v/skg-mcp.svg)](https://crates.io/crates/skg-mcp)
[![docs.rs](https://docs.rs/skg-mcp/badge.svg)](https://docs.rs/skg-mcp)
[![license](https://img.shields.io/crates/l/skg-mcp.svg)](LICENSE-MIT)

## Overview

`skg-mcp` bridges the [Model Context Protocol](https://modelcontextprotocol.io) ecosystem to
skelegent's `ToolRegistry`. Two independent components:

- **`McpClient`** — connects to an MCP server, discovers its tools, and returns them as
  `Arc<dyn ToolDyn>` for use in a `ToolRegistry`
- **`McpServer`** — wraps a `ToolRegistry` and exposes its tools over the MCP protocol via stdio

Backed by the [`rmcp`](https://crates.io/crates/rmcp) library.

## Exports

- **`McpClient`** — `connect_stdio(Command)`, `connect_http(url)`, `discover_tools()`,
  `discover_tools_with_aliases(aliases)`, `close()` (`connect_sse` is deprecated)
- **`McpServer`** — `new(registry, name, version)`, `serve_stdio()`
- **`McpError`** — `Connection(String)`, `Protocol(String)`

## Usage

```toml
[dependencies]
skg-mcp = "0.4"
skg-tool = "0.4"
tokio = { version = "1", features = ["full"] }
```

### Consuming an MCP server's tools

```rust,no_run
use skg_mcp::McpClient;
use skg_tool::ToolRegistry;

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

### Exposing skelegent tools as an MCP server

```rust,no_run
use skg_mcp::McpServer;
use skg_tool::ToolRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ToolRegistry::new(); // populate as needed
    let server = McpServer::new(registry, "my-server", "0.1.0");
    server.serve_stdio().await?;
    Ok(())
}
```

## Part of the skelegent workspace

[skelegent](https://github.com/secbear/skelegent) is a composable async agentic AI framework for Rust.
