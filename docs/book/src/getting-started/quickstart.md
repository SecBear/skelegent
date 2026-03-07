# Quickstart

This example creates an Anthropic provider, registers a tool, builds a ReAct operator, and runs a single invocation. The operator will call the model, use tools if needed, and return the result.

## Full example

```rust,no_run
use layer0::content::Content;
use layer0::operator::{Operator, OperatorInput, TriggerType};
use neuron_hooks::HookRegistry;
use neuron_op_react::{ReactConfig, ReactOperator};
use neuron_provider_anthropic::AnthropicProvider;
use neuron_tool::{ToolDyn, ToolError, ToolRegistry};
use neuron_state_memory::MemoryStore;
use neuron_turn_kit::FullContext;
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

    // 3. Configure the operator
    let config = ReactConfig {
        system_prompt: "You are a helpful assistant. Use tools when needed.".into(),
        default_model: "claude-haiku-4-5-20251001".into(),
        default_max_tokens: 4096,
        default_max_turns: 10,
    };

    // 4. Create the state reader
    let state_reader = Arc::new(MemoryStore::new());

    // 5. Create the context strategy
    let context_strategy = Box::new(FullContext);

    // 6. Build the ReAct operator
    let operator = ReactOperator::new(
        provider,
        tools,
        context_strategy,
        HookRegistry::new(),
        state_reader,
        config,
    );

    // 7. Create the input
    let input = OperatorInput::new(
        Content::text("What time is it right now?"),
        TriggerType::User,
    );

    // 8. Execute
    let output = operator.execute(input).await?;

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

3. **Operator construction.** `ReactOperator` implements `layer0::Operator`. It is generic over `P: Provider`, so it is constructed with a concrete provider type. The object-safe boundary is the `Operator` trait itself -- callers interact with `&dyn Operator` or `Box<dyn Operator>`.

4. **Execution.** `operator.execute(input)` runs the ReAct loop: assemble context, call the model, check for tool use, execute tools, repeat until the model produces a final response or a limit is reached.

5. **Output.** `OperatorOutput` contains the response message, exit reason (why the loop stopped), and metadata (tokens, cost, duration, sub-dispatch records).

## Next steps

- Read [Core Concepts](concepts.md) to understand the protocol architecture.
- See [Providers](../guides/providers.md) for details on configuring Anthropic, OpenAI, and Ollama.
- See [Tools](../guides/tools.md) for the full tool authoring guide.
- See [Operators](../guides/operators.md) for ReAct vs. single-shot configuration.
