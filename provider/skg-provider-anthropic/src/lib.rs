#![deny(missing_docs)]
//! Anthropic API provider for skg-turn.
//!
//! Implements the [`skg_turn::Provider`] trait for Anthropic's Messages API.

mod pricing;
mod types;

use futures_util::StreamExt;
use layer0::content::{Content, ContentBlock, ContentSource as L0ContentSource};
use skg_auth::{AuthProvider, AuthRequest};
use skg_turn::infer::{InferRequest, InferResponse, ToolCall};
use skg_turn::provider::{Provider, ProviderError};
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
use skg_turn::types::*;
use std::sync::Arc;
use tracing::Instrument;
use types::*;

/// Credential source — resolved per request.
#[derive(Clone)]
enum ApiKeySource {
    /// Static key provided at construction time.
    Static(String),
    /// Environment variable name resolved via [`std::env::var`] at each call.
    EnvVar(String),
    /// [`AuthProvider`] called at each request; supports token refresh.
    Auth {
        /// The provider to call.
        provider: Arc<dyn AuthProvider>,
        /// Audience string forwarded to [`AuthProvider::provide`].
        audience: String,
    },
}

/// Anthropic API provider.
#[derive(Clone)]
pub struct AnthropicProvider {
    api_key_source: ApiKeySource,
    client: reqwest::Client,
    api_url: String,
    api_version: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key_source: ApiKeySource::Static(api_key.into()),
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    /// Create a provider that reads its API key from an environment variable at each request.
    ///
    /// The variable is resolved via `std::env::var` at every call to `complete()`.  
    /// Returns `ProviderError::AuthFailed` if the variable is unset or empty — the error
    /// message contains the variable *name* only, never its value.
    pub fn from_env_var(var_name: impl Into<String>) -> Self {
        Self {
            api_key_source: ApiKeySource::EnvVar(var_name.into()),
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    /// Create a provider that authenticates via a [`skg_auth::AuthProvider`].
    ///
    /// The provider is called at **every** request, so token refresh is
    /// transparent. Use this with `PiAuthProvider` or `OmpAuthProvider`
    /// from `skelegent-extras`.
    ///
    /// OAuth tokens (`sk-ant-oat*`) returned by the provider are sent as
    /// `Authorization: Bearer` with Claude Code identity headers (`anthropic-beta:
    /// claude-code-20250219,oauth-2025-04-20`, `user-agent: claude-cli/2.1.62`,
    /// `x-app: cli`). Regular API keys use `x-api-key`.
    pub fn with_auth(provider: Arc<dyn AuthProvider>) -> Self {
        Self {
            api_key_source: ApiKeySource::Auth {
                provider,
                audience: "anthropic".into(),
            },
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    #[cfg(test)]
    async fn resolve_api_key(&self) -> Result<String, ProviderError> {
        resolve_key(&self.api_key_source).await
    }

    /// Override the API URL (for testing or proxies).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    fn build_infer_request(&self, request: &InferRequest) -> AnthropicRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "claude-haiku-4-5-20251001".into());
        let max_tokens = request.max_tokens.unwrap_or(4096);

        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .map(|m| AnthropicMessage {
                role: match &m.role {
                    layer0::context::Role::User => "user".into(),
                    layer0::context::Role::Assistant => "assistant".into(),
                    layer0::context::Role::System => "user".into(),
                    layer0::context::Role::Tool { .. } => "user".into(),
                    _ => "user".into(),
                },
                content: message_content_to_anthropic(&m.content),
            })
            .collect();

        let tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| {
                // Pass cache_control from ToolSchema.extra, if present.
                let cache_control = t
                    .extra
                    .as_ref()
                    .and_then(|e| e.get("cache_control"))
                    .cloned();
                AnthropicTool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t.input_schema.clone(),
                    cache_control,
                }
            })
            .collect();

        AnthropicRequest {
            model,
            max_tokens,
            messages,
            system: build_anthropic_system(&request.system, &request.extra),
            tools,
            stream: false,
        }
    }
}

/// Build the Anthropic `system` field from the request's system prompt and extra config.
///
/// If `extra["system_cache_control"]` is present, the system prompt is wrapped in a
/// content-block array so Anthropic's caching API can attach the directive. Otherwise
/// it falls through to a plain string, which avoids any wire-format regression.
fn build_anthropic_system(
    system: &Option<String>,
    extra: &serde_json::Value,
) -> Option<AnthropicSystemContent> {
    let text = system.as_ref()?.clone();
    if let Some(cache_control) = extra.get("system_cache_control").cloned() {
        Some(AnthropicSystemContent::Blocks(vec![AnthropicSystemBlock {
            block_type: "text",
            text,
            cache_control: Some(cache_control),
        }]))
    } else {
        Some(AnthropicSystemContent::Text(text))
    }
}

/// Convert an `anthropic_beta` extra value to a comma-joined header string.
///
/// Accepts a JSON string or an array of JSON strings. Returns an empty string
/// if the value is neither — callers must skip setting the header in that case.
fn extra_to_beta_header(beta: &serde_json::Value) -> String {
    match beta {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    }
}

/// Convert layer0 [`Content`] to Anthropic wire-format content.
fn message_content_to_anthropic(content: &Content) -> AnthropicContent {
    match content {
        Content::Text(text) => AnthropicContent::Text(text.clone()),
        Content::Blocks(blocks) => {
            let anthropic_blocks: Vec<AnthropicContentBlock> = blocks
                .iter()
                .filter_map(content_block_to_anthropic)
                .collect();
            if anthropic_blocks.len() == 1
                && let AnthropicContentBlock::Text { text } = &anthropic_blocks[0]
            {
                return AnthropicContent::Text(text.clone());
            }
            AnthropicContent::Blocks(anthropic_blocks)
        }
        // Future Content variants — treat as empty text
        _ => AnthropicContent::Text(String::new()),
    }
}

/// Convert a single layer0 [`ContentBlock`] to an Anthropic content block.
fn content_block_to_anthropic(block: &ContentBlock) -> Option<AnthropicContentBlock> {
    match block {
        ContentBlock::Text { text } => Some(AnthropicContentBlock::Text { text: text.clone() }),
        ContentBlock::ToolUse { id, name, input } => Some(AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        }),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        }),
        ContentBlock::Image { source, media_type } => Some(AnthropicContentBlock::Image {
            source: match source {
                L0ContentSource::Base64 { data } => {
                    AnthropicImageSource::Base64 { data: data.clone() }
                }
                L0ContentSource::Url { url } => AnthropicImageSource::Url { url: url.clone() },
                _ => return None,
            },
            media_type: media_type.clone(),
        }),
        // Skip custom blocks — not supported by Anthropic API
        _ => None,
    }
}

/// Parse a raw [`AnthropicResponse`] into an [`InferResponse`].
fn parse_anthropic_infer_response(
    response: AnthropicResponse,
) -> Result<InferResponse, ProviderError> {
    let mut text_parts: Vec<ContentBlock> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &response.content {
        match block {
            AnthropicContentBlock::Text { text } => {
                text_parts.push(ContentBlock::Text { text: text.clone() });
            }
            AnthropicContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            AnthropicContentBlock::ToolResult { .. } => {
                // Unexpected in a response — ignore
            }
            AnthropicContentBlock::Image { source, media_type } => {
                text_parts.push(ContentBlock::Image {
                    source: match source {
                        AnthropicImageSource::Base64 { data } => {
                            L0ContentSource::Base64 { data: data.clone() }
                        }
                        AnthropicImageSource::Url { url } => {
                            L0ContentSource::Url { url: url.clone() }
                        }
                    },
                    media_type: media_type.clone(),
                });
            }
        }
    }

    let content = if text_parts.len() == 1 {
        if let ContentBlock::Text { text } = &text_parts[0] {
            Content::Text(text.clone())
        } else {
            Content::Blocks(text_parts)
        }
    } else if text_parts.is_empty() {
        Content::text("")
    } else {
        Content::Blocks(text_parts)
    };

    let stop_reason = match response.stop_reason.as_str() {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::ContentFilter,
        _ => StopReason::EndTurn,
    };

    let usage = TokenUsage {
        input_tokens: response.usage.input_tokens,
        output_tokens: response.usage.output_tokens,
        cache_read_tokens: response.usage.cache_read_input_tokens,
        cache_creation_tokens: response.usage.cache_creation_input_tokens,
        reasoning_tokens: None,
    };

    let pricing = pricing::lookup(&response.model);
    let cost = pricing::compute_cost(&pricing, &usage);

    Ok(InferResponse {
        content,
        tool_calls,
        stop_reason,
        usage,
        model: response.model,
        cost: Some(cost),
        truncated: None,
    })
}

/// Returns `true` if `key` is an Anthropic OAuth token.
///
/// OAuth tokens use prefix `sk-ant-oat` and require `Authorization: Bearer`
/// plus Claude Code identity headers (`anthropic-beta: claude-code-20250219,
/// oauth-2025-04-20`, `user-agent: claude-cli/2.1.62`, `x-app: cli`).
/// Standard API keys use the `x-api-key` header.
fn is_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}

/// Resolve the credential from any [`ApiKeySource`] variant.
async fn resolve_key(source: &ApiKeySource) -> Result<String, ProviderError> {
    match source {
        ApiKeySource::Static(key) => Ok(key.clone()),
        ApiKeySource::EnvVar(var_name) => {
            let key = std::env::var(var_name).map_err(|_| {
                ProviderError::AuthFailed(format!("env var '{}' not set or not unicode", var_name))
            })?;
            if key.is_empty() {
                return Err(ProviderError::AuthFailed(format!(
                    "env var '{}' is empty",
                    var_name
                )));
            }
            Ok(key)
        }
        ApiKeySource::Auth { provider, audience } => {
            let req = AuthRequest::new().with_audience(audience.as_str());
            let token = provider
                .provide(&req)
                .await
                .map_err(|e| ProviderError::AuthFailed(format!("auth provider: {e}")))?;
            Ok(token.with_bytes(|b| String::from_utf8_lossy(b).into_owned()))
        }
    }
}

impl Provider for AnthropicProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        skg_turn::assert_real_requests_allowed();
        let source = self.api_key_source.clone();
        let api_request = self.build_infer_request(&request);
        let client = self.client.clone();
        let api_url = self.api_url.clone();
        let api_version = self.api_version.clone();
        let extra = request.extra.clone();

        let model = request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer", provider = "anthropic", model);

        async move {
            let key = resolve_key(&source).await?;

            let mut builder = client.post(&api_url);
            if is_oauth_token(&key) {
                builder = builder
                    .header("Authorization", format!("Bearer {key}"))
                    .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                    .header("user-agent", "claude-cli/2.1.62")
                    .header("x-app", "cli");
            } else {
                builder = builder.header("x-api-key", key);
                if let Some(beta) = extra.get("anthropic_beta") {
                    let hdr = extra_to_beta_header(beta);
                    if !hdr.is_empty() {
                        builder = builder.header("anthropic-beta", hdr);
                    }
                }
            }
            let http_request = builder
                .header("anthropic-version", &api_version)
                .header("content-type", "application/json")
                .json(&api_request);

            let http_response =
                http_request
                    .send()
                    .await
                    .map_err(|e| ProviderError::TransientError {
                        message: e.to_string(),
                        status: None,
                    })?;

            let status = http_response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry_after = http_response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(std::time::Duration::from_secs);
                return Err(ProviderError::RateLimited { retry_after });
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = http_response.text().await.unwrap_or_default();
                return Err(ProviderError::AuthFailed(body));
            }
            if !status.is_success() {
                let body = http_response.text().await.unwrap_or_default();
                return Err(map_error_response(status, &body));
            }

            let api_response: AnthropicResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            let response = parse_anthropic_infer_response(api_response)?;
            tracing::info!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                "inference finished"
            );
            Ok(response)
        }
        .instrument(span)
    }
}

impl StreamProvider for AnthropicProvider {
    fn infer_stream(
        &self,
        request: StreamRequest,
        on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let source = self.api_key_source.clone();
        let infer_request = InferRequest {
            model: request.model,
            messages: request.messages,
            tools: request.tools,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            system: request.system,
            extra: request.extra,
        };
        let mut api_request = self.build_infer_request(&infer_request);
        api_request.stream = true;
        let client = self.client.clone();
        let api_url = self.api_url.clone();
        let api_version = self.api_version.clone();
        let extra = infer_request.extra.clone();

        let model = infer_request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer_stream", provider = "anthropic", model);

        async move {
            let key = resolve_key(&source).await?;

            let mut builder = client.post(&api_url);
            if is_oauth_token(&key) {
                builder = builder
                    .header("Authorization", format!("Bearer {key}"))
                    .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                    .header("user-agent", "claude-cli/2.1.62")
                    .header("x-app", "cli");
            } else {
                builder = builder.header("x-api-key", key);
                if let Some(beta) = extra.get("anthropic_beta") {
                    let hdr = extra_to_beta_header(beta);
                    if !hdr.is_empty() {
                        builder = builder.header("anthropic-beta", hdr);
                    }
                }
            }
            let http_response = builder
                .header("anthropic-version", &api_version)
                .header("content-type", "application/json")
                .json(&api_request)
                .send()
                .await
                .map_err(|e| ProviderError::TransientError {
                    message: e.to_string(),
                    status: None,
                })?;

            let status = http_response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry_after = http_response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(std::time::Duration::from_secs);
                return Err(ProviderError::RateLimited { retry_after });
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = http_response.text().await.unwrap_or_default();
                return Err(ProviderError::AuthFailed(body));
            }
            if !status.is_success() {
                let body = http_response.text().await.unwrap_or_default();
                return Err(map_error_response(status, &body));
            }

            // Process SSE byte stream
            let mut stream = http_response.bytes_stream();
            let mut buf = String::new();

            // Accumulation state
            let mut model_name = String::new();
            let mut input_tokens: u64 = 0;
            let mut output_tokens: u64 = 0;
            let mut cache_read_tokens: Option<u64> = None;
            let mut cache_creation_tokens: Option<u64> = None;
            let mut stop_reason = StopReason::EndTurn;
            let mut text_parts: Vec<ContentBlock> = Vec::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            // Per-block accumulators (indexed by content_block index)
            let mut block_texts: Vec<String> = Vec::new();
            let mut block_tool_ids: Vec<String> = Vec::new();
            let mut block_tool_names: Vec<String> = Vec::new();
            let mut block_tool_inputs: Vec<String> = Vec::new();
            // Track which indices are tool_use vs text
            let mut block_is_tool: Vec<bool> = Vec::new();

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

                    // Parse SSE: extract event type and data
                    let mut event_type = String::new();
                    let mut data = String::new();
                    for line in frame.lines() {
                        if let Some(rest) = line.strip_prefix("event: ") {
                            event_type = rest.to_string();
                        } else if let Some(rest) = line.strip_prefix("data: ") {
                            data = rest.to_string();
                        }
                    }

                    if data.is_empty() {
                        continue;
                    }

                    let parsed: StreamEventData = match serde_json::from_str(&data) {
                        Ok(ev) => ev,
                        Err(e) => {
                            tracing::warn!(
                                event_type,
                                error = %e,
                                "failed to parse SSE event"
                            );
                            continue;
                        }
                    };

                    match parsed {
                        StreamEventData::MessageStart { message } => {
                            model_name = message.model;
                            input_tokens = message.usage.input_tokens;
                            cache_read_tokens = message.usage.cache_read_input_tokens;
                            cache_creation_tokens = message.usage.cache_creation_input_tokens;
                        }
                        StreamEventData::ContentBlockStart {
                            index,
                            content_block,
                        } => {
                            // Ensure accumulators are large enough
                            while block_texts.len() <= index {
                                block_texts.push(String::new());
                                block_tool_ids.push(String::new());
                                block_tool_names.push(String::new());
                                block_tool_inputs.push(String::new());
                                block_is_tool.push(false);
                            }
                            match &content_block {
                                AnthropicContentBlock::Text { .. } => {
                                    block_is_tool[index] = false;
                                }
                                AnthropicContentBlock::ToolUse { id, name, .. } => {
                                    block_is_tool[index] = true;
                                    block_tool_ids[index] = id.clone();
                                    block_tool_names[index] = name.clone();
                                    on_event(StreamEvent::ToolCallStart {
                                        index,
                                        id: id.clone(),
                                        name: name.clone(),
                                    });
                                }
                                _ => {}
                            }
                        }
                        StreamEventData::ContentBlockDelta { index, delta } => match delta {
                            StreamDelta::TextDelta { text } => {
                                if index < block_texts.len() {
                                    block_texts[index].push_str(&text);
                                }
                                on_event(StreamEvent::TextDelta(text));
                            }
                            StreamDelta::InputJsonDelta { partial_json } => {
                                if index < block_tool_inputs.len() {
                                    block_tool_inputs[index].push_str(&partial_json);
                                }
                                on_event(StreamEvent::ToolCallDelta {
                                    index,
                                    json_delta: partial_json,
                                });
                            }
                        },
                        StreamEventData::ContentBlockStop { index } => {
                            if index < block_is_tool.len() {
                                if block_is_tool[index] {
                                    let input_json: serde_json::Value = serde_json::from_str(
                                        &block_tool_inputs[index],
                                    )
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                                    tool_calls.push(ToolCall {
                                        id: block_tool_ids[index].clone(),
                                        name: block_tool_names[index].clone(),
                                        input: input_json,
                                    });
                                } else {
                                    text_parts.push(ContentBlock::Text {
                                        text: block_texts[index].clone(),
                                    });
                                }
                            }
                        }
                        StreamEventData::MessageDelta { delta, usage } => {
                            if let Some(sr) = delta.stop_reason {
                                stop_reason = match sr.as_str() {
                                    "end_turn" => StopReason::EndTurn,
                                    "tool_use" => StopReason::ToolUse,
                                    "max_tokens" => StopReason::MaxTokens,
                                    "refusal" => StopReason::ContentFilter,
                                    _ => StopReason::EndTurn,
                                };
                            }
                            if let Some(u) = usage {
                                output_tokens = u.output_tokens;
                                let usage_event = TokenUsage {
                                    input_tokens,
                                    output_tokens,
                                    cache_read_tokens,
                                    cache_creation_tokens,
                                    reasoning_tokens: None,
                                };
                                on_event(StreamEvent::Usage(usage_event));
                            }
                        }
                        StreamEventData::Error { error } => {
                            return Err(ProviderError::TransientError {
                                message: format!(
                                    "stream error ({}): {}",
                                    error.error_type, error.message
                                ),
                                status: None,
                            });
                        }
                        StreamEventData::MessageStop | StreamEventData::Ping => {}
                    }
                }
            }

            // Build final response
            let content = if text_parts.len() == 1 {
                if let ContentBlock::Text { text } = &text_parts[0] {
                    Content::Text(text.clone())
                } else {
                    Content::Blocks(text_parts)
                }
            } else if text_parts.is_empty() {
                Content::text("")
            } else {
                Content::Blocks(text_parts)
            };

            let usage = TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                reasoning_tokens: None,
            };

            let pricing = pricing::lookup(&model_name);
            let cost = pricing::compute_cost(&pricing, &usage);

            let response = InferResponse {
                content,
                tool_calls,
                stop_reason,
                usage,
                model: model_name,
                cost: Some(cost),
                truncated: None,
            };

            on_event(StreamEvent::Done(response.clone()));

            tracing::info!(input_tokens, output_tokens, "streaming inference finished");
            Ok(response)
        }
        .instrument(span)
    }
}

/// Map a non-success HTTP response to an appropriate [`ProviderError`].
///
/// - 500, 502, 503 (server errors) → [`ProviderError::TransientError`]
/// - Body containing content-filter signals → [`ProviderError::ContentBlocked`]
/// - All other non-success responses → [`ProviderError::TransientError`]
fn map_error_response(status: reqwest::StatusCode, body: &str) -> ProviderError {
    let status_u16 = status.as_u16();
    // Check for Anthropic content-filter signals in the response body.
    if body.contains("content_filter") || body.contains("content policy") {
        return ProviderError::ContentBlocked {
            message: body.to_string(),
        };
    }
    // Client errors (4xx except 429, handled earlier) are not retryable.
    if status.is_client_error() {
        return ProviderError::InvalidRequest {
            message: format!("HTTP {status}: {body}"),
            status: Some(status_u16),
        };
    }
    // Server errors and network issues are transient.
    ProviderError::TransientError {
        message: format!("HTTP {status}: {body}"),
        status: Some(status_u16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_schema_serializes() {
        let tool = AnthropicTool {
            name: "get_weather".into(),
            description: "Get current weather".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
            cache_control: None,
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "get_weather");
    }

    #[test]
    fn tool_cache_control_in_extra_is_forwarded() {
        use skg_turn::types::ToolSchema;
        let schema = ToolSchema::new(
            "search",
            "Search the web",
            json!({"type": "object", "properties": {}}),
        )
        .with_extra(json!({"cache_control": {"type": "ephemeral"}}));
        let provider = AnthropicProvider::new("sk-test");
        let request = skg_turn::infer::InferRequest::new(vec![])
            .with_tools(vec![schema]);
        let api_req = provider.build_infer_request(&request);
        let json = serde_json::to_value(&api_req.tools[0]).unwrap();
        assert_eq!(json["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn system_cache_control_in_extra_produces_block_array() {
        let provider = AnthropicProvider::new("sk-test");
        let request = skg_turn::infer::InferRequest::new(vec![])
            .with_system("You are helpful")
            .with_extra(json!({"system_cache_control": {"type": "ephemeral"}}));
        let api_req = provider.build_infer_request(&request);
        let json = serde_json::to_value(&api_req).unwrap();
        // system must be an array, not a string
        assert!(json["system"].is_array(), "system should be a block array");
        assert_eq!(json["system"][0]["type"], "text");
        assert_eq!(json["system"][0]["text"], "You are helpful");
        assert_eq!(json["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn system_without_cache_control_is_plain_string() {
        let provider = AnthropicProvider::new("sk-test");
        let request = skg_turn::infer::InferRequest::new(vec![])
            .with_system("You are helpful");
        let api_req = provider.build_infer_request(&request);
        let json = serde_json::to_value(&api_req).unwrap();
        assert_eq!(json["system"], "You are helpful");
    }

    #[test]
    fn extra_to_beta_header_handles_string_and_array() {
        let hdr = extra_to_beta_header(&json!("prompt-caching-2024-07-31"));
        assert_eq!(hdr, "prompt-caching-2024-07-31");
        let hdr = extra_to_beta_header(&json!(["a", "b", "c"]));
        assert_eq!(hdr, "a,b,c");
        let hdr = extra_to_beta_header(&json!(42));
        assert_eq!(hdr, "");
    }

    #[test]
    fn map_error_500_returns_transient() {
        let status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
        let err = map_error_response(status, "internal server error");
        assert!(matches!(
            err,
            ProviderError::TransientError {
                status: Some(500),
                ..
            }
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_error_503_returns_transient() {
        let status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
        let err = map_error_response(status, "service unavailable");
        assert!(matches!(
            err,
            ProviderError::TransientError {
                status: Some(503),
                ..
            }
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_error_content_filter_body_returns_blocked() {
        let status = reqwest::StatusCode::BAD_REQUEST;
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"content_filter triggered"}}"#;
        let err = map_error_response(status, body);
        assert!(matches!(err, ProviderError::ContentBlocked { .. }));
        assert!(!err.is_retryable());
    }
}

#[cfg(test)]
mod tests_credential {
    use super::*;
    use async_trait::async_trait;
    use skg_auth::{AuthError, AuthToken};

    // ── Static / env var ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn new_uses_static_key() {
        let p = AnthropicProvider::new("sk-static");
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-static");
    }

    #[tokio::test]
    async fn from_env_var_resolves_when_set() {
        let var = "SKG_ANTHROPIC_TEST_CRED_A";
        unsafe {
            std::env::set_var(var, "sk-from-env");
        }
        let p = AnthropicProvider::from_env_var(var);
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-from-env");
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn from_env_var_missing_returns_auth_failed() {
        let var = "SKG_ANTHROPIC_TEST_CRED_MISSING_ZZZ";
        unsafe {
            std::env::remove_var(var);
        }
        let p = AnthropicProvider::from_env_var(var);
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        assert!(
            err.to_string().contains(var),
            "error should name the variable"
        );
    }

    #[tokio::test]
    async fn from_env_var_empty_returns_auth_failed() {
        let var = "SKG_ANTHROPIC_TEST_CRED_EMPTY_ZZZ";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = AnthropicProvider::from_env_var(var);
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        assert!(
            err.to_string().contains(var),
            "error should name the variable"
        );
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn error_message_does_not_contain_secret_value() {
        let var = "SKG_ANTHROPIC_TEST_CRED_REDACT_ZZZ";
        let secret = "sk-must-not-appear-in-any-error-message";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = AnthropicProvider::from_env_var(var);
        let msg = p.resolve_api_key().await.unwrap_err().to_string();
        assert!(msg.contains(var));
        assert!(!msg.contains(secret));
        unsafe {
            std::env::set_var(var, secret);
        }
        assert_eq!(p.resolve_api_key().await.unwrap(), secret);
        unsafe {
            std::env::remove_var(var);
        }
    }

    // ── OAuth token detection ────────────────────────────────────────────────

    #[test]
    fn oauth_token_prefix_detected() {
        assert!(is_oauth_token("sk-ant-oat01-abc"));
        assert!(is_oauth_token("sk-ant-oat-xyz"));
        assert!(!is_oauth_token("sk-ant-api123"));
        assert!(!is_oauth_token("sk-ant-api03-abc"));
        assert!(!is_oauth_token(""));
    }

    // ── Auth provider variant ────────────────────────────────────────────────

    /// Minimal stub that returns a pre-set token.
    struct StubAuth {
        token: String,
        expected_audience: String,
    }

    #[async_trait]
    impl skg_auth::AuthProvider for StubAuth {
        async fn provide(&self, request: &skg_auth::AuthRequest) -> Result<AuthToken, AuthError> {
            assert_eq!(
                request.audience.as_deref().unwrap_or(""),
                self.expected_audience,
            );
            Ok(AuthToken::permanent(self.token.as_bytes().to_vec()))
        }
    }

    #[tokio::test]
    async fn with_auth_resolves_via_provider() {
        let stub = Arc::new(StubAuth {
            token: "sk-ant-test".into(),
            expected_audience: "anthropic".into(),
        });
        let p = AnthropicProvider::with_auth(stub);
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-ant-test");
    }

    #[tokio::test]
    async fn with_auth_oauth_token_detected() {
        let stub = Arc::new(StubAuth {
            token: "sk-ant-oat01-mytoken".into(),
            expected_audience: "anthropic".into(),
        });
        let p = AnthropicProvider::with_auth(stub);
        let key = p.resolve_api_key().await.unwrap();
        assert!(is_oauth_token(&key), "token should be detected as OAuth");
    }

    #[tokio::test]
    async fn with_auth_provider_error_maps_to_auth_failed() {
        struct FailAuth;
        #[async_trait]
        impl skg_auth::AuthProvider for FailAuth {
            async fn provide(&self, _: &skg_auth::AuthRequest) -> Result<AuthToken, AuthError> {
                Err(AuthError::ScopeUnavailable("no anthropic key".into()))
            }
        }
        let p = AnthropicProvider::with_auth(Arc::new(FailAuth));
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
    }
}
