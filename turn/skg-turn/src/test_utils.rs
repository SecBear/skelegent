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
        Self::new(|| ProviderError::RateLimited)
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
    ErrorProvider::new(|| ProviderError::RateLimited)
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
        assert!(matches!(err, ProviderError::RateLimited));
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
}
