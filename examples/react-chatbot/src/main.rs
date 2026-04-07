//! react-chatbot — ReAct agent with multiple tools across turns.
//!
//! Demonstrates:
//! - `#[skg_tool]` proc macro for tool definition
//! - `AgentBuilder` for operator construction
//! - `ReactLoopConfig` with `max_tool_retries` and `temperature`
//! - `FunctionProvider` for deterministic, key-free execution
//! - Multi-turn execution: two tool calls followed by a final text response
//!
//! Run with:
//!   cargo run -p react-chatbot
//!
//! No API keys required — all responses are scripted via FunctionProvider.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use layer0::content::Content;
use layer0::operator::{Operator, OperatorInput, TriggerType};
use layer0::{DispatchContext, DispatchId, OperatorId};
use serde_json::{Value, json};
use skg_context_engine::{AgentBuilder, ReactLoopConfig};
use skg_tool::{ToolError, skg_tool};
use skg_turn::infer::{InferRequest, InferResponse};
use skg_turn::provider::ProviderError;
use skg_turn::test_utils::{FunctionProvider, make_text_response, make_tool_call_response};

// ── Tool definitions ──────────────────────────────────────────────────────────
//
// The `#[skg_tool]` macro generates a `<PascalCase>Tool` struct implementing
// `ToolDyn` from this async function. The function name becomes the tool name
// (via the `name` attribute), the parameters become the JSON schema, and
// `Option<T>` parameters are marked optional in the schema.

/// Returns mock current weather for a city.
///
/// In a real agent this would call a weather API; here it returns static data
/// so the example runs without network access.
#[skg_tool(
    name = "get_weather",
    description = "Get the current weather for a city"
)]
async fn get_weather(city: String) -> Result<Value, ToolError> {
    // Mock weather data — real implementation would call an external API.
    Ok(json!({
        "city": city,
        "temperature_f": 72,
        "condition": "partly cloudy",
        "humidity_pct": 58
    }))
}

/// Returns mock current time for a timezone.
#[skg_tool(name = "get_time", description = "Get the current time in a timezone")]
async fn get_time(timezone: String) -> Result<Value, ToolError> {
    // Mock time data keyed by timezone abbreviation.
    let time = match timezone.to_uppercase().as_str() {
        "EST" | "ET" => "14:32:07",
        "PST" | "PT" => "11:32:07",
        "UTC" | "GMT" => "19:32:07",
        _ => "12:00:00",
    };
    Ok(json!({ "timezone": timezone, "time": time, "format": "HH:MM:SS" }))
}

/// Returns mock web search results for a query.
#[skg_tool(
    name = "search",
    description = "Search the web and return a short list of results",
    concurrent
)]
async fn search(query: String, limit: Option<i32>) -> Result<Value, ToolError> {
    // Mock results — limit defaults to 3 when absent.
    let n = limit.unwrap_or(3).max(1) as usize;
    let results: Vec<Value> = (1..=n)
        .map(|i| json!({ "rank": i, "title": format!("Result {i} for: {query}"), "url": format!("https://example.com/{i}") }))
        .collect();
    Ok(json!({ "query": query, "results": results }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Step 1: Scripted provider responses ───────────────────────────────────
    //
    // FunctionProvider wraps a closure that is called once per inference turn.
    // We pre-load a VecDeque so responses are returned in order, and an atomic
    // counter so we can print which turn the model is on.
    //
    // Turn 1 → model calls get_weather(city="NYC")
    // Turn 2 → model calls get_time(timezone="EST")
    // Turn 3 → model returns a text summary using both results

    let responses: Arc<Mutex<VecDeque<InferResponse>>> = Arc::new(Mutex::new(VecDeque::from([
        make_tool_call_response("get_weather", "call-weather-1", json!({"city": "NYC"})),
        make_tool_call_response("get_time", "call-time-1", json!({"timezone": "EST"})),
        make_text_response(
            "Based on my research: New York City is currently 72°F and partly cloudy. \
                 The local time in the Eastern timezone is 14:32. \
                 Great weather for an afternoon in the city!",
        ),
    ])));

    let turn_counter = Arc::new(AtomicUsize::new(0));

    let provider = FunctionProvider::new({
        let responses = Arc::clone(&responses);
        let counter = Arc::clone(&turn_counter);
        move |req: InferRequest| -> Result<InferResponse, ProviderError> {
            let turn = counter.fetch_add(1, Ordering::SeqCst) + 1;

            // Show what the model received this turn (message count is a proxy
            // for conversation depth since the full context grows each turn).
            println!(
                "\n[turn {turn}] model infer — {} message(s) in context, {} tool(s) available",
                req.messages.len(),
                req.tools.len(),
            );

            let resp = responses.lock().unwrap().pop_front().ok_or_else(|| {
                ProviderError::TransientError {
                    message: "FunctionProvider: response queue exhausted".into(),
                    status: None,
                }
            })?;

            // Print what the model is "deciding" this turn.
            if resp.tool_calls.is_empty() {
                println!("[turn {turn}] model returns text response (loop will end)");
            } else {
                for call in &resp.tool_calls {
                    println!(
                        "[turn {turn}] model calls tool `{}` with input: {}",
                        call.name, call.input
                    );
                }
            }

            Ok(resp)
        }
    });

    // ── Step 2: Build the AgentOperator ──────────────────────────────────────
    //
    // AgentBuilder is a typestate builder — `.provider()` is the last
    // required step, after which `.build()` becomes available.
    //
    // ReactLoopConfig captures per-loop settings:
    //   - max_tool_retries: how many times to retry a tool call on InvalidInput
    //   - temperature: forwarded to the provider on each inference call
    //
    // Tools registered via `.tool()` are added to the internal ToolRegistry
    // and their JSON schemas are forwarded to the model on every inference call.

    let config = ReactLoopConfig {
        // Retry a tool call up to 2 times if the model sends invalid input
        // (ToolError::InvalidInput feeds the error + schema back so the model
        // can correct the call; each call ID gets its own retry budget).
        max_tool_retries: 2,
        // Sampling temperature forwarded to the provider.
        temperature: Some(0.7),
        ..Default::default()
    };

    println!("Building AgentOperator with 3 tools and scripted FunctionProvider...");
    println!(
        "Config: max_tool_retries={}, temperature={:?}",
        config.max_tool_retries, config.temperature
    );

    let op = AgentBuilder::new()
        .system_prompt(
            "You are a helpful assistant with access to weather, time, and search tools. \
             Always use tools to gather information before answering.",
        )
        .config(config)
        // Register tools; the macro generated GetWeatherTool, GetTimeTool, SearchTool.
        .tool(Arc::new(GetWeatherTool::new()))
        .tool(Arc::new(GetTimeTool::new()))
        .tool(Arc::new(SearchTool::new()))
        // provider() advances the builder to WithProvider state, enabling build().
        .provider(provider)
        .build();

    // ── Step 3: Execute ───────────────────────────────────────────────────────
    //
    // DispatchContext carries dispatch metadata (ID, operator ID, auth, extensions)
    // through every boundary. The operator does not fabricate one — we supply it.

    let dispatch_ctx = DispatchContext::new(
        DispatchId::new("react-chatbot-demo"),
        OperatorId::new("react-chatbot"),
    );

    let input = OperatorInput::new(
        Content::text("What's the weather in NYC and what time is it in EST?"),
        TriggerType::User,
    );

    println!("\nUser: What's the weather in NYC and what time is it in EST?");
    println!("---");

    let output = op.execute(input, &dispatch_ctx).await?;

    // ── Step 4: Print results ─────────────────────────────────────────────────

    println!("\n--- Final Output ---");
    println!(
        "Agent: {}",
        output.message.as_text().unwrap_or("(no text response)")
    );
    println!("\n--- Execution Summary ---");
    println!("Exit reason : {}", output.outcome);
    println!("Turns used  : {}", output.metadata.turns_used);
    println!("Tokens in   : {}", output.metadata.tokens_in);
    println!("Tokens out  : {}", output.metadata.tokens_out);
    println!("Cost        : ${}", output.metadata.cost);

    // sub_dispatches records cross-operator dispatches (when this operator calls
    // other operators via a Dispatcher). Tool calls within the ReAct loop are
    // tracked in the Context's metrics, not here — so this list is empty for a
    // standalone single-operator run, which is the expected behavior.
    if output.metadata.sub_dispatches.is_empty() {
        println!("Sub-dispatches: (none — tool calls within the loop are not sub-dispatches)");
    } else {
        println!("Sub-dispatches:");
        for sd in &output.metadata.sub_dispatches {
            println!(
                "  - {} ({}ms, success={})",
                sd.name,
                sd.duration.as_millis(),
                sd.success
            );
        }
    }

    Ok(())
}
