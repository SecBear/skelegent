//! Test utilities for provider implementations.
//!
//! Provides deterministic, queue-based [`TestProvider`], closure-based
//! [`FunctionProvider`], and error-returning [`ErrorProvider`] — all
//! implementing [`Provider`] via `infer()`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use layer0::content::Content;

use crate::infer::{InferRequest, InferResponse, ToolCall};
use crate::provider::{Provider, ProviderError};
use crate::types::{StopReason, TokenUsage};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TestProvider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Queue-based deterministic provider for tests.
///
/// Responses are popped from the front of the queue on each `infer()` call.
/// Panics if called with an empty queue — a test bug.
pub struct TestProvider {
    responses: Mutex<VecDeque<InferResponse>>,
    call_count: AtomicUsize,
    recorded_requests: Mutex<Vec<InferRequest>>,
}

impl TestProvider {
    /// Create an empty provider. Will panic on `infer()` unless responses are queued.
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(VecDeque::new()),
            call_count: AtomicUsize::new(0),
            recorded_requests: Mutex::new(Vec::new()),
        }
    }

    /// Create a provider pre-loaded with responses.
    pub fn with_responses(responses: Vec<InferResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            call_count: AtomicUsize::new(0),
            recorded_requests: Mutex::new(Vec::new()),
        }
    }

    /// Queue a simple text response with `StopReason::EndTurn`.
    pub fn respond_with_text(&self, text: &str) -> &Self {
        self.responses
            .lock()
            .unwrap()
            .push_back(make_text_response(text));
        self
    }

    /// Queue a tool-use response with `StopReason::ToolUse`.
    pub fn respond_with_tool_call(&self, name: &str, id: &str, input: serde_json::Value) -> &Self {
        self.responses
            .lock()
            .unwrap()
            .push_back(make_tool_call_response(name, id, input));
        self
    }

    /// Queue a response with multiple tool calls.
    pub fn respond_with_tool_calls(&self, calls: Vec<(&str, &str, serde_json::Value)>) -> &Self {
        let tool_calls = calls
            .into_iter()
            .map(|(name, id, input)| crate::infer::ToolCall {
                id: id.into(),
                name: name.into(),
                input,
            })
            .collect();
        let response = InferResponse {
            content: Content::text(""),
            tool_calls,
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage::default(),
            model: "test-model".into(),
            cost: None,
            truncated: None,
        };
        self.responses.lock().unwrap().push_back(response);
        self
    }

    /// How many times `infer()` has been called.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    /// The last request received, if any.
    pub fn last_request(&self) -> Option<InferRequest> {
        self.recorded_requests.lock().unwrap().last().cloned()
    }

    /// All requests received so far.
    pub fn requests(&self) -> Vec<InferRequest> {
        self.recorded_requests.lock().unwrap().clone()
    }
}

impl Default for TestProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for TestProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.recorded_requests.lock().unwrap().push(request.clone());

        let response = self.responses.lock().unwrap().pop_front().expect(
            "TestProvider: no more queued responses — queue a response before calling infer()",
        );

        async move { Ok(response) }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FunctionProvider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Closure-based provider for custom test logic.
pub struct FunctionProvider<F>
where
    F: Fn(InferRequest) -> Result<InferResponse, ProviderError> + Send + Sync,
{
    func: F,
}

impl<F> FunctionProvider<F>
where
    F: Fn(InferRequest) -> Result<InferResponse, ProviderError> + Send + Sync,
{
    /// Wrap a closure as a provider.
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> Provider for FunctionProvider<F>
where
    F: Fn(InferRequest) -> Result<InferResponse, ProviderError> + Send + Sync,
{
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let result = (self.func)(request);
        async move { result }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ErrorProvider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Provider that always returns a specific error. Useful for testing error paths.
///
/// Uses a factory closure since [`ProviderError`] is not `Clone`.
pub struct ErrorProvider<F>
where
    F: Fn() -> ProviderError + Send + Sync,
{
    factory: F,
}

impl<F> ErrorProvider<F>
where
    F: Fn() -> ProviderError + Send + Sync,
{
    /// Create from an error factory closure.
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl ErrorProvider<fn() -> ProviderError> {
    /// Provider that always returns `ProviderError::RateLimited`.
    pub fn rate_limited() -> Self {
        Self::new(|| ProviderError::RateLimited { retry_after: None })
    }

    /// Provider that always returns `ProviderError::AuthFailed`.
    pub fn auth_failed(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        // We need a concrete closure type that is fn() -> ProviderError.
        // Since fn pointers can't capture, use the generic constructor instead.
        // This is a workaround — return a different concrete type.
        let _ = msg;
        Self::new(|| ProviderError::AuthFailed("auth failed".into()))
    }
}

impl<F> Provider for ErrorProvider<F>
where
    F: Fn() -> ProviderError + Send + Sync,
{
    fn infer(
        &self,
        _request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let err = (self.factory)();
        async move { Err(err) }
    }
}

/// Create an [`ErrorProvider`] that always returns `ProviderError::RateLimited`.
pub fn error_provider_rate_limited() -> ErrorProvider<impl Fn() -> ProviderError + Send + Sync> {
    ErrorProvider::new(|| ProviderError::RateLimited { retry_after: None })
}

/// Create an [`ErrorProvider`] that always returns `ProviderError::AuthFailed`.
pub fn error_provider_auth_failed(
    msg: impl Into<String>,
) -> ErrorProvider<impl Fn() -> ProviderError + Send + Sync> {
    let msg = msg.into();
    ErrorProvider::new(move || ProviderError::AuthFailed(msg.clone()))
}

/// Create an [`ErrorProvider`] that always returns `ProviderError::TransientError`.
pub fn error_provider_transient(
    msg: impl Into<String>,
) -> ErrorProvider<impl Fn() -> ProviderError + Send + Sync> {
    let msg = msg.into();
    ErrorProvider::new(move || ProviderError::TransientError {
        message: msg.clone(),
        status: None,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build a simple text `InferResponse`.
pub fn make_text_response(text: &str) -> InferResponse {
    InferResponse {
        content: Content::text(text),
        tool_calls: vec![],
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage::default(),
        model: "test-model".into(),
        cost: None,
        truncated: None,
    }
}

/// Build a tool-call `InferResponse`.
pub fn make_tool_call_response(name: &str, id: &str, input: serde_json::Value) -> InferResponse {
    InferResponse {
        content: Content::text(""),
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: TokenUsage::default(),
        model: "test-model".into(),
        cost: None,
        truncated: None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SchemaTestProvider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Schema-driven test provider that auto-generates tool calls from tool schemas.
///
/// On each `infer()` call, if tools are available and the round limit has not
/// been reached, picks the first tool, generates minimal valid arguments from
/// its `input_schema` via [`generate_from_schema`], and returns a [`ToolCall`]
/// response. Once the round limit is reached (or no tools are present), returns
/// a text response `"done"`.
///
/// Useful for exercising the react loop without hand-crafting tool responses.
pub struct SchemaTestProvider {
    /// After this many tool call rounds, return a text response to end the loop.
    max_tool_rounds: usize,
    /// Number of tool call rounds completed so far.
    round: AtomicUsize,
}

impl SchemaTestProvider {
    /// Create a new provider that will issue up to `max_tool_rounds` tool calls
    /// before returning a final text response.
    pub fn new(max_tool_rounds: usize) -> Self {
        Self {
            max_tool_rounds,
            round: AtomicUsize::new(0),
        }
    }

    /// How many tool call rounds have been issued so far.
    pub fn rounds_completed(&self) -> usize {
        self.round.load(Ordering::SeqCst)
    }
}

impl Provider for SchemaTestProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let round = self.round.load(Ordering::SeqCst);
        let response = if !request.tools.is_empty() && round < self.max_tool_rounds {
            let tool = &request.tools[0];
            let input = generate_from_schema(&tool.input_schema);
            let call_id = format!("tc_{round}");
            self.round.fetch_add(1, Ordering::SeqCst);
            InferResponse {
                content: Content::text(""),
                tool_calls: vec![ToolCall {
                    id: call_id,
                    name: tool.name.clone(),
                    input,
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
                model: "schema-test-model".into(),
                cost: None,
                truncated: None,
            }
        } else {
            make_text_response("done")
        };
        async move { Ok(response) }
    }
}

/// Generate minimal valid JSON values from a JSON Schema object.
///
/// Walks `"type"` and `"properties"` to produce a value that satisfies the
/// schema structurally. Only fields listed in `"required"` are generated for
/// objects; optional properties are skipped. Unknown or missing type annotations
/// produce `null`.
///
/// Supported types:
/// - `"string"` → `"test_value"`
/// - `"integer"` → `42`
/// - `"number"` → `3.14`
/// - `"boolean"` → `true`
/// - `"object"` → recurses into each required property; returns `{}` when none
/// - `"array"` → single-element array of the generated item type
/// - unknown / missing → `null`
pub fn generate_from_schema(schema: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};

    let type_str = schema.get("type").and_then(Value::as_str).unwrap_or("");
    match type_str {
        "string" => json!("test_value"),
        "integer" => json!(42),
        "number" => json!(3.14),
        "boolean" => json!(true),
        "object" => {
            let mut obj = serde_json::Map::new();
            // Only generate required fields; optional properties are skipped.
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, prop_schema) in props {
                    if required.contains(&key.as_str()) {
                        obj.insert(key.clone(), generate_from_schema(prop_schema));
                    }
                }
            }
            Value::Object(obj)
        }
        "array" => {
            let item_schema = schema.get("items").unwrap_or(&Value::Null);
            serde_json::json!([generate_from_schema(item_schema)])
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::context::{Message, Role};

    fn dummy_request() -> InferRequest {
        InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
    }

    #[tokio::test]
    async fn test_provider_returns_queued_responses() {
        let provider = TestProvider::with_responses(vec![
            make_text_response("first"),
            make_text_response("second"),
        ]);

        let r1 = provider.infer(dummy_request()).await.unwrap();
        assert_eq!(r1.text(), Some("first"));

        let r2 = provider.infer(dummy_request()).await.unwrap();
        assert_eq!(r2.text(), Some("second"));
    }

    #[tokio::test]
    #[should_panic(expected = "no more queued responses")]
    async fn test_provider_panics_when_empty() {
        let provider = TestProvider::new();
        let _ = provider.infer(dummy_request()).await;
    }

    #[tokio::test]
    async fn test_provider_tracks_call_count() {
        let provider = TestProvider::new();
        provider.respond_with_text("a");
        provider.respond_with_text("b");
        provider.respond_with_text("c");

        assert_eq!(provider.call_count(), 0);
        let _ = provider.infer(dummy_request()).await;
        assert_eq!(provider.call_count(), 1);
        let _ = provider.infer(dummy_request()).await;
        assert_eq!(provider.call_count(), 2);
        let _ = provider.infer(dummy_request()).await;
        assert_eq!(provider.call_count(), 3);
    }

    #[tokio::test]
    async fn test_provider_records_requests() {
        let provider = TestProvider::new();
        provider.respond_with_text("reply");

        let req = InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
            .with_model("gpt-4");
        provider.infer(req).await.unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model.as_deref(), Some("gpt-4"));
        assert_eq!(requests[0].messages.len(), 1);

        let last = provider.last_request().unwrap();
        assert_eq!(last.model.as_deref(), Some("gpt-4"));
    }

    #[tokio::test]
    async fn test_function_provider() {
        let provider = FunctionProvider::new(|req| {
            let model = req.model.unwrap_or_else(|| "default".into());
            Ok(InferResponse {
                content: Content::text(format!("echoed with {model}")),
                tool_calls: vec![],
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
                model,
                cost: None,
                truncated: None,
            })
        });

        let resp = provider
            .infer(dummy_request().with_model("claude"))
            .await
            .unwrap();
        assert_eq!(resp.text(), Some("echoed with claude"));
        assert_eq!(resp.model, "claude");
    }

    #[tokio::test]
    async fn test_error_provider_rate_limited() {
        let provider = error_provider_rate_limited();
        let err = provider.infer(dummy_request()).await.unwrap_err();
        assert!(err.is_retryable());
        assert!(matches!(err, ProviderError::RateLimited { .. }));
    }

    #[tokio::test]
    async fn test_error_provider_auth_failed() {
        let provider = error_provider_auth_failed("bad key");
        let err = provider.infer(dummy_request()).await.unwrap_err();
        assert!(!err.is_retryable());
        assert!(matches!(err, ProviderError::AuthFailed(_)));
    }

    #[tokio::test]
    async fn test_respond_with_tool_call() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("search", "tc_1", serde_json::json!({"q": "test"}));

        let resp = provider.infer(dummy_request()).await.unwrap();
        assert!(resp.has_tool_calls());
        assert_eq!(resp.tool_calls[0].name, "search");
        assert_eq!(resp.tool_calls[0].id, "tc_1");
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
    }

    #[tokio::test]
    async fn schema_provider_generates_tool_call() {
        use crate::types::ToolSchema;
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["query", "limit"]
        });
        let tool = ToolSchema::new("search", "Search tool", schema);
        let provider = SchemaTestProvider::new(1);
        let request = dummy_request().with_tools(vec![tool]);

        let resp = provider.infer(request).await.unwrap();
        assert!(resp.has_tool_calls());
        assert_eq!(resp.tool_calls[0].name, "search");
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        let input = &resp.tool_calls[0].input;
        assert_eq!(input["query"], serde_json::json!("test_value"));
        assert_eq!(input["limit"], serde_json::json!(42));
    }

    #[tokio::test]
    async fn schema_provider_returns_text_after_max_rounds() {
        use crate::types::ToolSchema;
        let tool = ToolSchema::new("noop", "No-op", serde_json::json!({ "type": "object" }));
        let provider = SchemaTestProvider::new(2);
        let request = dummy_request().with_tools(vec![tool]);

        let r1 = provider.infer(request.clone()).await.unwrap();
        assert!(r1.has_tool_calls(), "round 1 should be a tool call");

        let r2 = provider.infer(request.clone()).await.unwrap();
        assert!(r2.has_tool_calls(), "round 2 should be a tool call");

        let r3 = provider.infer(request.clone()).await.unwrap();
        assert!(!r3.has_tool_calls(), "round 3 should be text");
        assert_eq!(r3.text(), Some("done"));
        assert_eq!(provider.rounds_completed(), 2);
    }

    #[test]
    fn generate_from_schema_handles_types() {
        use serde_json::json;

        // Scalar types
        assert_eq!(generate_from_schema(&json!({"type": "string"})), json!("test_value"));
        assert_eq!(generate_from_schema(&json!({"type": "integer"})), json!(42));
        assert_eq!(generate_from_schema(&json!({"type": "number"})), json!(3.14));
        assert_eq!(generate_from_schema(&json!({"type": "boolean"})), json!(true));

        // Unknown and missing type → null
        assert_eq!(generate_from_schema(&json!({"type": "unknown"})), json!(null));
        assert_eq!(generate_from_schema(&json!(null)), json!(null));
        assert_eq!(generate_from_schema(&json!({"description": "no type"})), json!(null));

        // Array wraps a single generated item
        let arr = generate_from_schema(&json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(arr, json!(["test_value"]));

        // Object: only required fields are generated
        let obj = generate_from_schema(&json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        }));
        assert_eq!(obj["name"], json!("test_value"));
        assert!(obj.as_object().unwrap().get("age").is_none(), "optional field should be skipped");

        // Object with no required → empty object
        let empty_obj = generate_from_schema(&json!({
            "type": "object",
            "properties": { "foo": { "type": "string" } }
        }));
        assert_eq!(empty_obj, json!({}));

        // Object with no properties → empty object
        assert_eq!(generate_from_schema(&json!({"type": "object"})), json!({}));

        // Nested objects recurse correctly
        let nested = generate_from_schema(&json!({
            "type": "object",
            "properties": {
                "inner": {
                    "type": "object",
                    "properties": { "x": { "type": "integer" } },
                    "required": ["x"]
                }
            },
            "required": ["inner"]
        }));
        assert_eq!(nested["inner"]["x"], json!(42));
    }
}