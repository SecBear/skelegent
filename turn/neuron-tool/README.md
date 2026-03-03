# neuron-tool

> Tool interface and registry for neuron agents

[![crates.io](https://img.shields.io/crates/v/neuron-tool.svg)](https://crates.io/crates/neuron-tool)
[![docs.rs](https://docs.rs/neuron-tool/badge.svg)](https://docs.rs/neuron-tool)
[![license](https://img.shields.io/crates/l/neuron-tool.svg)](LICENSE-MIT)

## Overview

`neuron-tool` defines the `ToolDyn` trait and `ToolRegistry` that operators use to expose callable
functions to models. Tools are described via JSON Schema, called with a `serde_json::Value` input,
and return a `serde_json::Value` result. Any tool source (local function, MCP server, HTTP
endpoint) implements `ToolDyn`.

## Exports

- **`ToolDyn`** — object-safe trait: `name()`, `description()`, `input_schema()`, `call(input)`,
  `maybe_streaming()`, `concurrency_hint()`
- **`ToolRegistry`** — `new()`, `register(Arc<dyn ToolDyn>)`, `get(name)`, `iter()`, `len()`,
  `is_empty()`
- **`ToolDynStreaming`** — optional streaming trait: `call_streaming(input, on_chunk)`
- **`ToolConcurrencyHint`** — `Shared` | `Exclusive` (default)
- **`AliasedTool`** — wraps a `ToolDyn` under a different name: `new(alias, inner)`, `inner()`
- **`ToolError`** — `NotFound`, `ExecutionFailed`, `InvalidInput`, `Other`

## Usage

```toml
[dependencies]
neuron-tool = "0.4"
serde_json = "1"
```

### Implementing a tool

```rust,no_run
use neuron_tool::{ToolDyn, ToolError};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

pub struct UppercaseTool;

impl ToolDyn for UppercaseTool {
    fn name(&self) -> &str { "uppercase" }

    fn description(&self) -> &str { "Convert text to uppercase" }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            },
            "required": ["text"]
        })
    }

    fn call(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let text = input["text"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidInput("missing text".into()))?;
            Ok(json!({ "result": text.to_uppercase() }))
        })
    }
}
```

### Registering and calling tools

```rust,no_run
use neuron_tool::ToolRegistry;
use std::sync::Arc;

let mut registry = ToolRegistry::new();
registry.register(Arc::new(UppercaseTool));

if let Some(tool) = registry.get("uppercase") {
    let result = tool.call(serde_json::json!({"text": "hello"})).await?;
}
```

## Part of the neuron workspace

[neuron](https://github.com/secbear/neuron) is a composable async agentic AI framework for Rust.
