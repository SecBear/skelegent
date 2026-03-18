# Quickstart

This example creates an Anthropic provider, registers a tool, builds a `Context`, and runs `react_loop` directly. The loop will call the model, use tools if needed, and return the result.

## Full example

```rust,no_run
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::DispatchContext;
use layer0::id::{DispatchId, OperatorId};
use skg_context_engine::{Context, ReactLoopConfig, react_loop};
use skg_provider_anthropic::AnthropicProvider;
use skg_tool::{ToolDyn, ToolError, ToolRegistry};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A simple tool that returns the current time.
struct CurrentTimeTool;

impl ToolDyn for CurrentTimeTool {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Returns the current UTC time."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn call(
        &self,
        _input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        Box::pin(async {
            // In a real tool, you'd use chrono or std::time
            Ok(json!({ "time": "2026-02-28T12:00:00Z" }))
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create the provider (reads ANTHROPIC_API_KEY from env)
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("Set ANTHROPIC_API_KEY");
    let provider = AnthropicProvider::new(api_key);

    // 2. Build the tool registry
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(CurrentTimeTool));

    // 3. Configure the react loop
    let config = ReactLoopConfig {
        system_prompt: "You are a helpful assistant. Use tools when needed.".into(),
        model: Some("claude-haiku-4-5-20251001".into()),
        max_tokens: Some(4096),
        temperature: None,
    };

    // 4. Create a dispatch context (identifies the calling agent)
    let tool_ctx = DispatchContext::new(DispatchId::new("assistant"), OperatorId::new("assistant"));

    // 5. Build a Context and inject the user message
    let mut ctx = Context::new();
    ctx.inject_message(Message::new(Role::User, Content::text("What time is it right now?")))
        .await?;

    // 6. Run the react loop
    let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config).await?;

    println!("Response: {:?}", output.message);
    println!("Exit reason: {:?}", output.exit_reason);
    println!("Tokens: {} in, {} out",
        output.metadata.tokens_in,
        output.metadata.tokens_out,
    );
    println!("Cost: ${}", output.metadata.cost);

    Ok(())
}
```

## What is happening

1. **Provider creation.** `AnthropicProvider::new(api_key)` creates an HTTP client for the Anthropic Messages API. The provider implements the `Provider` trait, which is an internal (non-object-safe) trait used by operator implementations.

2. **Tool registration.** The `CurrentTimeTool` implements `ToolDyn` -- an object-safe trait that defines a tool's name, description, JSON Schema, and async execution. Tools are stored as `Arc<dyn ToolDyn>` in the `ToolRegistry`.

3. **Loop configuration.** `ReactLoopConfig` holds the system prompt, model, and token limits. It is a plain config struct -- not an operator. The react loop uses it to build a `CompileConfig` for each inference call.

4. **Context and execution.** `Context` is the conversation store -- it holds messages, assembly ops, and rules. You inject a user message, then call `react_loop()` which composes the core primitives: compile context, infer with the provider, apply context ops (append response, execute tools), repeat until the model produces a final response or a limit is reached.

5. **Output.** `OperatorOutput` contains the response message, exit reason (why the loop stopped), and metadata (tokens, cost, duration, sub-dispatch records).

> **Tip:** To use `react_loop` behind the object-safe `Operator` trait boundary, wrap it in your own struct that implements `Operator`. See the [Operators guide](../guides/operators.md) for the pattern.

## Next steps

- Read [Core Concepts](concepts.md) to understand the protocol architecture.
- See [Providers](../guides/providers.md) for details on configuring Anthropic, OpenAI, and Ollama.
- See [Tools](../guides/tools.md) for the full tool authoring guide.
- See [Operators](../guides/operators.md) for ReAct vs. single-shot configuration.