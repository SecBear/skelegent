//! echo-agent — the simplest possible agent.
//!
//! Demonstrates:
//!   1. Manual `ToolDyn` implementation (a `greet` tool)
//!   2. `TestProvider` as a scripted, API-key-free provider
//!   3. `CognitiveBuilder` fluent construction of `CognitiveOperator`
//!   4. Single-turn execution and output inspection
//!
//! Run with: `cargo run -p echo-agent`
//! No API keys or environment variables required.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::content::Content;
use layer0::operator::{Operator, OperatorInput, TriggerType};
use layer0::{DispatchContext, DispatchId, OperatorId};
use skg_context_engine::CognitiveBuilder;
use skg_tool::{ToolDyn, ToolError, ToolRegistry};
use skg_turn::test_utils::{TestProvider, make_text_response, make_tool_call_response};

// ── Step 1: Define a tool ─────────────────────────────────────────────────────
//
// Implement `ToolDyn` directly on a unit struct. This is the lowest-level path;
// the `#[skg_tool]` proc-macro generates this boilerplate when the "macros"
// feature is enabled on `skg-tool`.

struct GreetTool;

impl ToolDyn for GreetTool {
    fn name(&self) -> &str {
        "greet"
    }

    fn description(&self) -> &str {
        "Greet someone by name. Returns a greeting string."
    }

    // The JSON Schema that the model uses to construct call arguments.
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the person to greet."
                }
            },
            "required": ["name"]
        })
    }

    // The actual implementation. Receives parsed JSON, returns JSON.
    // `ctx` carries dispatch metadata (IDs, auth, extensions); not needed here.
    fn call(
        &self,
        input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let name = input["name"].as_str().unwrap_or("World");
            Ok(serde_json::json!(format!("Hello, {}!", name)))
        })
    }
}

// ── Step 2: Build a scripted provider ─────────────────────────────────────────
//
// `TestProvider` is a queue-based provider from `skg-turn`'s test-utils.
// It returns pre-scripted `InferResponse`s in order, no network needed.
//
// The react loop runs like this:
//   Turn 1 → model asks to call `greet({"name": "World"})`
//   Tool runs → produces "Hello, World!"
//   Turn 2 → model sees the result and produces a final text response

fn build_provider() -> TestProvider {
    TestProvider::with_responses(vec![
        // Turn 1: the model decides to call the `greet` tool.
        make_tool_call_response(
            "greet",    // tool name
            "call_001", // tool call ID (opaque string, echoed back)
            serde_json::json!({"name": "World"}),
        ),
        // Turn 2: the model produces a final text answer after seeing the result.
        make_text_response("The greeting was sent."),
    ])
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Step 3: Register the tool.
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(GreetTool));

    // Step 4: Build the CognitiveOperator via the fluent builder.
    //
    // CognitiveBuilder is a typestate builder:
    //   - `build()` is only callable after `.provider(...)` is set (compile-time check).
    //   - `.system_prompt()` and `.tools()` can be called in any order.
    let op = CognitiveBuilder::new()
        .system_prompt("You are a greeting agent. Use the greet tool when asked to greet someone.")
        .tools(registry)
        .provider(build_provider())
        .build();

    // Step 5: Construct input and execute.
    //
    // `DispatchContext` carries the dispatch ID and operator ID used in traces
    // and tool metadata. For a standalone example, use any stable string IDs.
    let input = OperatorInput::new(Content::text("Please greet the world"), TriggerType::User);
    let ctx = DispatchContext::new(
        DispatchId::new("echo-run-001"),
        OperatorId::new("echo-agent"),
    );

    let output = op.execute(input, &ctx).await?;

    // Step 6: Inspect and print the output.
    println!("Exit reason : {:?}", output.outcome);
    println!(
        "Response    : {}",
        output.message.as_text().unwrap_or("(no text)")
    );
    println!("Intents     : {}", output.intents.len());
    println!(
        "Tokens      : in={}, out={}",
        output.metadata.tokens_in, output.metadata.tokens_out
    );
    println!("Turns used  : {}", output.metadata.turns_used);

    Ok(())
}
