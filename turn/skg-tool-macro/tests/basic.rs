// Integration tests for the `#[skg_tool]` proc macro.
// Each test exercises the generated struct through the `ToolDyn` trait.

use serde_json::{Value, json};
use layer0::{DispatchContext, DispatchId, OperatorId};
use skg_tool::{ToolConcurrencyHint, ToolDyn, ToolError};
use skg_tool_macro::skg_tool;

// ── Test 1: basic required-parameter tool ─────────────────────────────────────

#[skg_tool(name = "get_weather", description = "Get current weather")]
async fn get_weather(location: String) -> Result<Value, ToolError> {
    Ok(json!({"location": location, "temp": 72}))
}

#[tokio::test]
async fn test_basic_tool_metadata() {
    let tool = GetWeatherTool::new();
    assert_eq!(tool.name(), "get_weather");
    assert_eq!(tool.description(), "Get current weather");
    assert_eq!(tool.concurrency_hint(), ToolConcurrencyHint::Exclusive);
}

#[tokio::test]
async fn test_basic_tool_schema() {
    let tool = GetWeatherTool::new();
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["location"]["type"], "string");

    let required = schema["required"]
        .as_array()
        .expect("required must be an array");
    assert_eq!(required.len(), 1);
    assert!(required.iter().any(|v| v == "location"));
}

#[tokio::test]
async fn test_basic_tool_call() {
    let tool = GetWeatherTool::new();
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-agent"));
    let input = json!({"location": "San Francisco"});
    let result = tool.call(input, &ctx).await.expect("call must succeed");
    assert_eq!(result["location"], "San Francisco");
    assert_eq!(result["temp"], 72);
}

// ── Test 2: optional parameter ────────────────────────────────────────────────

#[skg_tool(name = "search", description = "Search for things")]
async fn search(query: String, limit: Option<i32>) -> Result<Value, ToolError> {
    Ok(json!({"query": query, "limit": limit}))
}

#[tokio::test]
async fn test_optional_param_schema() {
    let tool = SearchTool::new();
    let schema = tool.input_schema();
    assert_eq!(schema["properties"]["query"]["type"], "string");
    assert_eq!(schema["properties"]["limit"]["type"], "integer");

    let required = schema["required"]
        .as_array()
        .expect("required must be an array");
    // Only `query` is required; `limit` is optional
    assert_eq!(required.len(), 1);
    assert!(required.iter().any(|v| v == "query"));
    assert!(!required.iter().any(|v| v == "limit"));
}

#[tokio::test]
async fn test_optional_param_absent_deserialises_as_none() {
    let tool = SearchTool::new();
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-agent"));
    // `limit` is not present in the input JSON
    let input = json!({"query": "rust proc macros"});
    let result = tool.call(input, &ctx).await.expect("call must succeed");
    assert_eq!(result["query"], "rust proc macros");
    // serde_json serialises None as null
    assert!(result["limit"].is_null());
}

#[tokio::test]
async fn test_optional_param_present_deserialises_correctly() {
    let tool = SearchTool::new();
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-agent"));
    let input = json!({"query": "rust", "limit": 10});
    let result = tool.call(input, &ctx).await.expect("call must succeed");
    assert_eq!(result["query"], "rust");
    assert_eq!(result["limit"], 10);
}

// ── Test 3: DispatchContext parameter ─────────────────────────────────────────

#[skg_tool(name = "agent_info", description = "Returns agent info from context")]
async fn agent_info(ctx: &DispatchContext, label: String) -> Result<Value, ToolError> {
    let operator_str = ctx.operator_id.to_string();
    Ok(json!({"agent": operator_str, "label": label}))
}

#[tokio::test]
async fn test_ctx_param_excluded_from_schema() {
    let tool = AgentInfoTool::new();
    let schema = tool.input_schema();
    // `ctx` must NOT appear as a property in the schema
    assert!(schema["properties"]["ctx"].is_null());
    // `label` must appear
    assert_eq!(schema["properties"]["label"]["type"], "string");
    let required = schema["required"]
        .as_array()
        .expect("required must be an array");
    assert!(required.iter().any(|v| v == "label"));
}

#[tokio::test]
async fn test_ctx_param_passed_through_to_function() {
    let tool = AgentInfoTool::new();
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("my-agent"));
    let input = json!({"label": "hello"});
    let result = tool.call(input, &ctx).await.expect("call must succeed");
    assert_eq!(result["label"], "hello");
    assert_eq!(result["agent"], "my-agent");
}

// ── Test 4: concurrent flag ───────────────────────────────────────────────────

#[skg_tool(
    name = "parallel_op",
    description = "Safe to run concurrently",
    concurrent
)]
async fn parallel_op(value: i64) -> Result<Value, ToolError> {
    Ok(json!({"value": value}))
}

#[tokio::test]
async fn test_concurrent_flag_sets_shared_hint() {
    let tool = ParallelOpTool::new();
    assert_eq!(tool.concurrency_hint(), ToolConcurrencyHint::Shared);
}

// ── Test 5: zero-parameter function ──────────────────────────────────────────

#[skg_tool(name = "ping", description = "Ping the tool")]
async fn ping() -> Result<Value, ToolError> {
    Ok(json!({"pong": true}))
}

#[tokio::test]
async fn test_zero_param_schema_is_empty_object() {
    let tool = PingTool::new();
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    let props = schema["properties"]
        .as_object()
        .expect("properties must be an object");
    assert!(props.is_empty());
    let required = schema["required"]
        .as_array()
        .expect("required must be an array");
    assert!(required.is_empty());
}

#[tokio::test]
async fn test_zero_param_call() {
    let tool = PingTool::new();
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-agent"));
    let result = tool.call(json!({}), &ctx).await.expect("call must succeed");
    assert_eq!(result["pong"], true);
}

// ── Test 6: Default trait ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_default_trait_is_equivalent_to_new() {
    let a = GetWeatherTool::new();
    let b = GetWeatherTool;
    // Both instances are unit structs so they are behaviorally identical
    assert_eq!(a.name(), b.name());
}

// ── Test 7: object safety (ToolDyn must be usable as dyn ToolDyn) ─────────────

#[test]
fn test_generated_struct_usable_as_dyn_tool_dyn() {
    use std::sync::Arc;
    let _: Arc<dyn ToolDyn> = Arc::new(GetWeatherTool::new());
}
