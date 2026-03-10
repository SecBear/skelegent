# MCP

> **Note:** The MCP integration API may shift as the Model Context Protocol specification evolves. This page provides a summary of the current design. See the `skg-mcp` crate for the latest API.

## Overview

`skg-mcp` provides a client for the [Model Context Protocol](https://modelcontextprotocol.io/) (MCP). MCP is an open protocol for connecting AI models to external data sources and tools. The skelegent MCP client connects to MCP servers and exposes their tools as `ToolDyn` implementations that can be registered in a `ToolRegistry`.

This means tools hosted on MCP servers can be used by skelegent operators alongside locally defined tools, with no difference in how the operator interacts with them.

## Integration pattern

The typical flow is:

1. Connect to one or more MCP servers.
2. Discover available tools from each server.
3. Wrap each MCP tool as an `Arc<dyn ToolDyn>`.
4. Register them in the operator's `ToolRegistry`.

The operator's ReAct loop then calls MCP tools the same way it calls local tools -- through the `ToolDyn` interface.

## When to use MCP

MCP is useful when:
- You want to expose tools from existing MCP-compatible servers (database access, file systems, APIs).
- You want to share tool definitions across multiple applications.
- You want to decouple tool implementation from operator configuration.

For tools that are specific to your application and do not need to be shared, implementing `ToolDyn` directly is simpler.

## Crate

The `skg-mcp` crate depends on `layer0` and `skg-tool`. Enable it via the `mcp` feature flag on the `skelegent` umbrella crate:

```toml
[dependencies]
skelegent = { version = "0.4", features = ["mcp"] }
```
