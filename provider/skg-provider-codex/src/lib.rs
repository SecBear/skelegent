#![deny(missing_docs)]
//! OpenAI Codex (Responses API) provider for skg-turn.
//!
//! Implements [`Provider`] and [`StreamProvider`] for the OpenAI Responses API,
//! supporting both the standard `api.openai.com/v1/responses` endpoint and the
//! Codex backend at `chatgpt.com/backend-api/codex/responses`.
//!
//! # Authentication
//!
//! Codex uses OAuth JWT tokens. The provider extracts the `chatgpt_account_id`
//! from the JWT payload and sends it as the `chatgpt-account-id` header.
//!
//! ```ignore
//! use skg_provider_codex::CodexProvider;
//!
//! let provider = CodexProvider::new("eyJ...");  // JWT from OMP
//! ```

mod auth;
mod convert;
mod types;

use convert::{messages_to_input, tools_to_codex};
use futures_util::StreamExt;
use layer0::content::{Content, ContentBlock};
use rust_decimal::Decimal;
use skg_turn::infer::{InferRequest, InferResponse, ToolCall};
use skg_turn::provider::{Provider, ProviderError};
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
use skg_turn::types::*;
use tracing::Instrument;
use types::*;

/// Default base URL for the Codex backend.
const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// SSE path for responses.
const CODEX_RESPONSES_PATH: &str = "/codex/responses";

/// OpenAI Codex (Responses API) provider.
#[derive(Clone)]
pub struct CodexProvider {
    access_token: String,
    account_id: String,
    client: reqwest::Client,
    base_url: String,
}

impl CodexProvider {
    /// Create a new Codex provider with a JWT access token.
    ///
    /// The account ID is automatically extracted from the JWT payload.
    /// Returns an error if the token is not a valid Codex JWT.
    pub fn new(access_token: impl Into<String>) -> Result<Self, ProviderError> {
        let token = access_token.into();
        let account_id = auth::extract_account_id(&token).ok_or_else(|| {
            ProviderError::AuthFailed("failed to extract account ID from JWT".into())
        })?;
        Ok(Self {
            access_token: token,
            account_id,
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.into(),
        })
    }

    /// Create a provider with explicit token and account ID.
    ///
    /// Use this when you have the account ID from another source
    /// (e.g., stored separately from the JWT).
    pub fn with_account_id(access_token: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            account_id: account_id.into(),
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.into(),
        }
    }

    /// Override the base URL (for testing or custom endpoints).
    ///
    /// Default: `https://chatgpt.com/backend-api`
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Build the full URL for the Responses API endpoint.
    fn endpoint_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}{CODEX_RESPONSES_PATH}")
    }

    /// Build request headers for Codex API calls.
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", self.access_token)
                .parse()
                .expect("valid header"),
        );
        headers.insert(
            "chatgpt-account-id",
            self.account_id.parse().expect("valid header"),
        );
        headers.insert(
            "openai-beta",
            "responses=experimental".parse().expect("valid header"),
        );
        headers.insert("originator", "pi".parse().expect("valid header"));
        headers.insert(
            "content-type",
            "application/json".parse().expect("valid header"),
        );
        headers
    }

    /// Build a [`CodexRequest`] from an [`InferRequest`].
    fn build_codex_request(&self, request: &InferRequest) -> CodexRequest {
        let model = request.model.clone().unwrap_or_else(|| "gpt-5".into());

        let input = messages_to_input(&request.messages);
        let tools = tools_to_codex(&request.tools);

        CodexRequest {
            model,
            input,
            stream: true,
            instructions: request.system.clone(),
            tools,
            tool_choice: None,
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            reasoning: None,
            prompt_cache_key: None,
            store: Some(false),
        }
    }

    /// Build a [`CodexRequest`] from a [`StreamRequest`].
    fn build_codex_stream_request(&self, request: &StreamRequest) -> CodexRequest {
        let infer = InferRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            system: request.system.clone(),
            extra: request.extra.clone(),
        };
        self.build_codex_request(&infer)
    }

    /// Send request and process SSE stream, emitting events via callback.
    async fn stream_sse(
        &self,
        codex_request: CodexRequest,
        on_event: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<InferResponse, ProviderError> {
        let url = self.endpoint_url();
        let headers = self.build_headers();

        let http_response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&codex_request)
            .send()
            .await
            .map_err(|e| ProviderError::TransientError {
                message: e.to_string(),
                status: None,
            })?;

        let status = http_response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::RateLimited);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = http_response.text().await.unwrap_or_default();
            return Err(ProviderError::AuthFailed(body));
        }
        if !status.is_success() {
            let body = http_response.text().await.unwrap_or_default();
            return Err(map_error_response(status, &body));
        }

        // Process SSE stream
        let mut stream = http_response.bytes_stream();
        let mut buf = String::new();

        // Accumulation state
        let mut model_name = codex_request.model.clone();
        let mut usage = ResponseUsage::default();
        let mut stop_reason = StopReason::EndTurn;
        let mut text_blocks: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        // Per-block state (indexed by output item position)
        let mut current_text = String::new();
        let mut current_tool_call_id = String::new();
        let mut current_tool_item_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();
        let mut tool_call_index: usize = 0;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| ProviderError::TransientError {
                message: format!("stream read error: {e}"),
                status: None,
            })?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete SSE frames
            while let Some(frame_end) = buf.find("\n\n") {
                let frame = buf[..frame_end].to_string();
                buf = buf[frame_end + 2..].to_string();

                // Extract data from SSE frame
                let mut data = String::new();
                for line in frame.lines() {
                    if let Some(rest) = line.strip_prefix("data: ") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(rest);
                    }
                }

                if data.is_empty() {
                    continue;
                }

                let event: SseEvent = match serde_json::from_str(&data) {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to parse Codex SSE event");
                        continue;
                    }
                };

                match event.event_type.as_str() {
                    "response.output_item.added" => {
                        if let Some(item) = event.data.get("item") {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match item_type {
                                "message" => {
                                    current_text = String::new();
                                }
                                "function_call" => {
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let item_id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    current_tool_call_id = call_id.clone();
                                    current_tool_item_id = item_id;
                                    current_tool_name = name.clone();
                                    current_tool_args = String::new();

                                    // Compose the skelegent tool ID as "call_id|item_id"
                                    let skg_id = format!("{call_id}|{}", current_tool_item_id);
                                    on_event(StreamEvent::ToolCallStart {
                                        index: tool_call_index,
                                        id: skg_id,
                                        name,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                    "response.output_text.delta" => {
                        if let Some(delta) = event.data.get("delta").and_then(|v| v.as_str()) {
                            current_text.push_str(delta);
                            on_event(StreamEvent::TextDelta(delta.to_string()));
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        if let Some(delta) = event.data.get("delta").and_then(|v| v.as_str()) {
                            current_tool_args.push_str(delta);
                            on_event(StreamEvent::ToolCallDelta {
                                index: tool_call_index,
                                json_delta: delta.to_string(),
                            });
                        }
                    }
                    "response.output_item.done" => {
                        if let Some(item) = event.data.get("item") {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

                            match item_type {
                                "message" => {
                                    // Finalize text from the item itself
                                    let final_text = extract_output_text(item);
                                    if !final_text.is_empty() {
                                        current_text = final_text;
                                    }
                                    if !current_text.is_empty() {
                                        text_blocks.push(current_text.clone());
                                    }
                                    current_text = String::new();
                                }
                                "function_call" => {
                                    // Finalize tool call
                                    let final_args = item
                                        .get("arguments")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(&current_tool_args);
                                    let input: serde_json::Value = serde_json::from_str(final_args)
                                        .unwrap_or(serde_json::Value::Object(
                                            serde_json::Map::new(),
                                        ));
                                    tool_calls.push(ToolCall {
                                        id: format!(
                                            "{}|{}",
                                            current_tool_call_id, current_tool_item_id
                                        ),
                                        name: current_tool_name.clone(),
                                        input,
                                    });
                                    tool_call_index += 1;
                                    current_tool_args = String::new();
                                }
                                _ => {}
                            }
                        }
                    }
                    "response.completed" | "response.done" => {
                        if let Some(response) = event.data.get("response") {
                            if let Some(u) = response.get("usage") {
                                usage = ResponseUsage::from_value(u);
                                on_event(StreamEvent::Usage(TokenUsage {
                                    input_tokens: usage.input_tokens,
                                    output_tokens: usage.output_tokens,
                                    cache_read_tokens: if usage.cached_tokens > 0 {
                                        Some(usage.cached_tokens)
                                    } else {
                                        None
                                    },
                                    cache_creation_tokens: None,
                                }));
                            }
                            if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
                                stop_reason = match status {
                                    "completed" => StopReason::EndTurn,
                                    "incomplete" => StopReason::MaxTokens,
                                    "failed" | "cancelled" => StopReason::EndTurn,
                                    _ => StopReason::EndTurn,
                                };
                            }
                            if let Some(m) = response.get("model").and_then(|v| v.as_str()) {
                                model_name = m.to_string();
                            }
                        }
                    }
                    "error" | "response.failed" => {
                        let msg = event
                            .data
                            .get("message")
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                event
                                    .data
                                    .get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|v| v.as_str())
                            })
                            .unwrap_or("Codex stream error");
                        return Err(ProviderError::TransientError {
                            message: msg.to_string(),
                            status: None,
                        });
                    }
                    _ => {
                        // Ignore: response.created, ping, reasoning events, etc.
                    }
                }
            }
        }

        // If tool calls present but stop reason is EndTurn, fix it.
        if !tool_calls.is_empty() && stop_reason == StopReason::EndTurn {
            stop_reason = StopReason::ToolUse;
        }

        // Build final content.
        let content = if text_blocks.len() == 1 {
            Content::Text(text_blocks.into_iter().next().unwrap())
        } else if text_blocks.is_empty() {
            Content::text("")
        } else {
            Content::Blocks(
                text_blocks
                    .into_iter()
                    .map(|t| ContentBlock::Text { text: t })
                    .collect(),
            )
        };

        // Codex is free (included in subscription), cost is zero.
        let token_usage = TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: if usage.cached_tokens > 0 {
                Some(usage.cached_tokens)
            } else {
                None
            },
            cache_creation_tokens: None,
        };

        let response = InferResponse {
            content,
            tool_calls,
            stop_reason,
            usage: token_usage,
            model: model_name,
            cost: Some(Decimal::ZERO),
            truncated: None,
        };

        on_event(StreamEvent::Done(response.clone()));

        tracing::info!(
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            "codex streaming inference finished"
        );

        Ok(response)
    }
}

impl Provider for CodexProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let codex_request = self.build_codex_request(&request);
        let this = self.clone();
        let model = request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer", provider = "codex", model);

        async move {
            // Non-streaming: use stream_sse with a no-op callback, then return the response.
            this.stream_sse(codex_request, &|_| {}).await
        }
        .instrument(span)
    }
}

impl StreamProvider for CodexProvider {
    fn infer_stream(
        &self,
        request: StreamRequest,
        on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let codex_request = self.build_codex_stream_request(&request);
        let this = self.clone();
        let model = request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer_stream", provider = "codex", model);

        async move { this.stream_sse(codex_request, &on_event).await }.instrument(span)
    }
}

/// Extract combined text from a Responses API output message item.
fn extract_output_text(item: &serde_json::Value) -> String {
    item.get("content")
        .and_then(|c| c.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| {
                    let ptype = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match ptype {
                        "output_text" => p.get("text").and_then(|v| v.as_str()),
                        "refusal" => p.get("refusal").and_then(|v| v.as_str()),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Map a non-success HTTP response to a [`ProviderError`].
fn map_error_response(status: reqwest::StatusCode, body: &str) -> ProviderError {
    let status_u16 = status.as_u16();

    // Check for rate limit / usage limit signals.
    if body.contains("usage_limit_reached")
        || body.contains("usage_not_included")
        || body.contains("rate_limit_exceeded")
    {
        return ProviderError::RateLimited;
    }

    if body.contains("content_filter") || body.contains("content policy") {
        return ProviderError::ContentBlocked {
            message: body.to_string(),
        };
    }

    ProviderError::TransientError {
        message: format!("HTTP {status}: {body}"),
        status: Some(status_u16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_default() {
        let p = CodexProvider::with_account_id("tok", "acct");
        assert_eq!(
            p.endpoint_url(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn endpoint_url_custom() {
        let p =
            CodexProvider::with_account_id("tok", "acct").with_base_url("http://localhost:8080/");
        assert_eq!(p.endpoint_url(), "http://localhost:8080/codex/responses");
    }

    #[test]
    fn error_mapping_rate_limit() {
        let err = map_error_response(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":{"code":"rate_limit_exceeded"}}"#,
        );
        assert!(matches!(err, ProviderError::RateLimited));
    }

    #[test]
    fn error_mapping_content_filter() {
        let err = map_error_response(reqwest::StatusCode::BAD_REQUEST, "content_filter triggered");
        assert!(matches!(err, ProviderError::ContentBlocked { .. }));
    }

    #[test]
    fn extract_output_text_basic() {
        let item = serde_json::json!({
            "type": "message",
            "content": [
                {"type": "output_text", "text": "Hello "},
                {"type": "output_text", "text": "world"}
            ]
        });
        assert_eq!(extract_output_text(&item), "Hello world");
    }

    #[test]
    fn build_request_sets_stream_true() {
        let p = CodexProvider::with_account_id("tok", "acct");
        let req = InferRequest::new(vec![]);
        let codex = p.build_codex_request(&req);
        assert!(codex.stream);
        assert_eq!(codex.store, Some(false));
    }
}
