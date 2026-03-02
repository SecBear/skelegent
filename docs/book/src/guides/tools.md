# Tools

Tools give operators the ability to take actions: read files, make HTTP requests, query databases, or perform any side-effecting operation. The tool system is built around the `ToolDyn` trait and the `ToolRegistry`.

## The ToolDyn trait

```rust
pub trait ToolDyn: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn call(
        &self,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>;
}
```

`ToolDyn` is object-safe. Tools are stored as `Arc<dyn ToolDyn>` and can be composed dynamically at runtime. The four methods:

- **`name()`** -- Unique identifier for the tool. This is what the model uses to request the tool.
- **`description()`** -- Human-readable description. Sent to the model as part of the tool definition.
- **`input_schema()`** -- JSON Schema describing the tool's parameters. The model generates input conforming to this schema.
- **`call()`** -- Async execution. Takes JSON input, returns JSON output or a `ToolError`.

## Creating a tool

Implement `ToolDyn` for any struct:

```rust
use neuron_tool::{ToolDyn, ToolError};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

struct ReadFileTool;

impl ToolDyn for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read"
                }
            },
            "required": ["path"]
        })
    }

    fn call(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let path = input["path"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidInput("missing 'path'".into()))?;

            let contents = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            Ok(json!({ "contents": contents }))
        })
    }
}
```

## The ToolRegistry

`ToolRegistry` is a named collection of tools:

```rust
use neuron_tool::ToolRegistry;
use std::sync::Arc;

let mut registry = ToolRegistry::new();
registry.register(Arc::new(ReadFileTool));
registry.register(Arc::new(WriteFileTool));
registry.register(Arc::new(BashTool));

// Look up by name
if let Some(tool) = registry.get("read_file") {
    let result = tool.call(json!({"path": "/tmp/test.txt"})).await?;
}

// Iterate all tools (e.g., to build tool definitions for the model)
for tool in registry.iter() {
    println!("{}: {}", tool.name(), tool.description());
}
```

Tools are keyed by name. Registering a tool with the same name as an existing tool overwrites it.

## AliasedTool

`AliasedTool` wraps an existing tool under a different name. This is useful when importing tools from external systems (e.g., MCP servers) where upstream names do not match your desired naming scheme:

```rust
use neuron_tool::AliasedTool;
use std::sync::Arc;

let original: Arc<dyn ToolDyn> = Arc::new(ReadFileTool);
let aliased = Arc::new(AliasedTool::new("read", original));

assert_eq!(aliased.name(), "read");
// description, schema, and call behavior are delegated to the inner tool
```

## Tool errors

```rust
pub enum ToolError {
    NotFound(String),         // Tool not found in registry
    ExecutionFailed(String),  // Tool execution failed
    InvalidInput(String),     // Input didn't match schema
    Other(Box<dyn Error>),    // Catch-all
}
```

## How tools integrate with operators

The `ReactOperator` uses a `ToolRegistry` internally. When the model responds with a `ToolUse` content block, the operator:

1. Looks up the tool by name in the registry.
2. Fires `PreToolUse` hooks (which may skip or modify the call).
3. Calls `tool.call(input)`.
4. Fires `PostToolUse` hooks (which may modify the output).
5. Backfills the tool result into the conversation context.
6. Calls the model again with the updated context.

This continues until the model produces a final text response (no more tool use), a limit is reached, or a hook halts execution.

## Tool schema design tips

- Use `"required"` to mark parameters the model must provide.
- Include `"description"` on each property -- the model uses these to understand what to pass.
- Keep schemas simple. Complex nested schemas increase the chance of the model producing invalid input.
- Return structured JSON from `call()`. The model reads the tool result to decide its next action.


## Migration from Rho tools

If you're migrating from Rho:
- Each `rho-tools` function maps to a `neuron_tool::ToolDyn` implementation.
- Register tools in a `ToolRegistry`; the operator builds tool definitions from this registry.
- Model/provider wiring from `rho-ai` maps to `neuron-turn` + `neuron-provider-*` crates; your loop logic lives in a custom operator if you need barriers/steering.