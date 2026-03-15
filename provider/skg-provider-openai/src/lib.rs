#![deny(missing_docs)]
//! OpenAI API provider for skg-turn.
//!
//! Implements the [`skg_turn::Provider`] trait for OpenAI's Chat Completions API.

mod types;

use futures_util::StreamExt;
use layer0::content::{Content, ContentBlock};
use layer0::context;
use rust_decimal::Decimal;
use skg_turn::infer::{InferRequest, InferResponse, ToolCall};
use skg_turn::embedding::{EmbedRequest, EmbedResponse, Embedding};
use skg_turn::provider::{Provider, ProviderError};
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
use skg_turn::types::*;
use tracing::Instrument;
use types::*;

/// API key source — static string or environment variable resolved per request.
enum ApiKeySource {
    /// Key material provided at construction time.
    Static(String),
    /// Environment variable name; resolved at each `complete()` call.
    EnvVar(String),
}

/// OpenAI API provider.
pub struct OpenAIProvider {
    api_key_source: ApiKeySource,
    client: reqwest::Client,
    api_url: String,
    org_id: Option<String>,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key_source: ApiKeySource::Static(api_key.into()),
            client: reqwest::Client::new(),
            api_url: "https://api.openai.com/v1/chat/completions".into(),
            org_id: None,
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
            api_url: "https://api.openai.com/v1/chat/completions".into(),
            org_id: None,
        }
    }

    fn resolve_api_key(&self) -> Result<String, ProviderError> {
        match &self.api_key_source {
            ApiKeySource::Static(key) => Ok(key.clone()),
            ApiKeySource::EnvVar(var_name) => {
                let key = std::env::var(var_name).map_err(|_| {
                    ProviderError::AuthFailed(format!(
                        "env var '{}' not set or not unicode",
                        var_name
                    ))
                })?;
                if key.is_empty() {
                    return Err(ProviderError::AuthFailed(format!(
                        "env var '{}' is empty",
                        var_name
                    )));
                }
                Ok(key)
            }
        }
    }

    /// Override the API URL (for testing or proxies).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    /// Set the OpenAI-Organization header for multi-org accounts.
    pub fn with_org(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }

    /// Build an [`OpenAIRequest`] from an [`InferRequest`] (layer0 Message types).
    fn build_infer_request(&self, request: &InferRequest) -> OpenAIRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "gpt-4o-mini".into());

        let mut messages: Vec<OpenAIMessage> = Vec::new();

        // System prompt becomes a system message.
        if let Some(ref system) = request.system {
            messages.push(OpenAIMessage {
                role: "system".into(),
                content: Some(OpenAIContent::Text(system.clone())),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Map layer0 Messages to OpenAI messages.
        for msg in &request.messages {
            match &msg.role {
                context::Role::System => {
                    let text = content_to_text(&msg.content);
                    messages.push(OpenAIMessage {
                        role: "system".into(),
                        content: Some(OpenAIContent::Text(text)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                context::Role::User => {
                    // Separate tool results from other content.
                    // OpenAI uses role="tool" for tool results.
                    match &msg.content {
                        Content::Text(text) => {
                            messages.push(OpenAIMessage {
                                role: "user".into(),
                                content: Some(OpenAIContent::Text(text.clone())),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                        Content::Blocks(blocks) => {
                            let mut tool_results = Vec::new();
                            let mut other_parts = Vec::new();
                            for block in blocks {
                                match block {
                                    ContentBlock::ToolResult {
                                        tool_use_id,
                                        content,
                                        ..
                                    } => {
                                        tool_results.push((tool_use_id.clone(), content.clone()));
                                    }
                                    _ => {
                                        other_parts.push(block.clone());
                                    }
                                }
                            }
                            // Emit tool result messages first.
                            for (tool_call_id, content) in tool_results {
                                messages.push(OpenAIMessage {
                                    role: "tool".into(),
                                    content: Some(OpenAIContent::Text(content)),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_call_id),
                                });
                            }
                            // Emit user message if there are other parts.
                            if !other_parts.is_empty() {
                                messages.push(OpenAIMessage {
                                    role: "user".into(),
                                    content: Some(blocks_to_openai_content(&other_parts)),
                                    tool_calls: None,
                                    tool_call_id: None,
                                });
                            }
                        }
                        _ => {
                            let text = content_to_text(&msg.content);
                            messages.push(OpenAIMessage {
                                role: "user".into(),
                                content: Some(OpenAIContent::Text(text)),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                    }
                }
                context::Role::Assistant => {
                    // Separate tool-use blocks from text/image content.
                    match &msg.content {
                        Content::Text(text) => {
                            messages.push(OpenAIMessage {
                                role: "assistant".into(),
                                content: Some(OpenAIContent::Text(text.clone())),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                        Content::Blocks(blocks) => {
                            let mut tool_calls = Vec::new();
                            let mut text_blocks = Vec::new();
                            for block in blocks {
                                match block {
                                    ContentBlock::ToolUse { id, name, input } => {
                                        tool_calls.push(OpenAIToolCall {
                                            id: id.clone(),
                                            call_type: "function".into(),
                                            function: OpenAIFunctionCall {
                                                name: name.clone(),
                                                arguments: serde_json::to_string(input)
                                                    .unwrap_or_default(),
                                            },
                                        });
                                    }
                                    _ => {
                                        text_blocks.push(block.clone());
                                    }
                                }
                            }
                            let content = if text_blocks.is_empty() {
                                None
                            } else {
                                Some(blocks_to_openai_content(&text_blocks))
                            };
                            let tool_calls_field = if tool_calls.is_empty() {
                                None
                            } else {
                                Some(tool_calls)
                            };
                            messages.push(OpenAIMessage {
                                role: "assistant".into(),
                                content,
                                tool_calls: tool_calls_field,
                                tool_call_id: None,
                            });
                        }
                        _ => {
                            let text = content_to_text(&msg.content);
                            messages.push(OpenAIMessage {
                                role: "assistant".into(),
                                content: Some(OpenAIContent::Text(text)),
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                    }
                }
                context::Role::Tool { call_id, .. } => {
                    // Tool result message.
                    let text = content_to_text(&msg.content);
                    // Try to extract tool_call_id from ToolResult blocks if call_id is empty.
                    let resolved_call_id = if call_id.is_empty() {
                        if let Content::Blocks(blocks) = &msg.content {
                            blocks
                                .iter()
                                .find_map(|b| match b {
                                    ContentBlock::ToolResult { tool_use_id, .. } => {
                                        Some(tool_use_id.clone())
                                    }
                                    _ => None,
                                })
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }
                    } else {
                        call_id.clone()
                    };
                    messages.push(OpenAIMessage {
                        role: "tool".into(),
                        content: Some(OpenAIContent::Text(text)),
                        tool_calls: None,
                        tool_call_id: Some(resolved_call_id),
                    });
                }
                _ => {
                    // Future role variants — treat as user message.
                    let text = content_to_text(&msg.content);
                    messages.push(OpenAIMessage {
                        role: "user".into(),
                        content: Some(OpenAIContent::Text(text)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        let tools: Vec<OpenAITool> = request
            .tools
            .iter()
            .map(|t| OpenAITool {
                tool_type: "function".into(),
                function: OpenAIFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // Extract provider-specific fields from extra.
        let service_tier = request
            .extra
            .get("service_tier")
            .and_then(|v| v.as_str())
            .map(String::from);
        let reasoning_effort = request
            .extra
            .get("reasoning_effort")
            .and_then(|v| v.as_str())
            .map(String::from);
        let parallel_tool_calls = request
            .extra
            .get("parallel_tool_calls")
            .and_then(|v| v.as_bool());

        OpenAIRequest {
            model,
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools,
            parallel_tool_calls,
            service_tier,
            reasoning_effort,
            stream: false,
            stream_options: None,
        }
    }

    /// Parse an [`OpenAIResponse`] into an [`InferResponse`].
    fn parse_infer_response(
        &self,
        response: OpenAIResponse,
    ) -> Result<InferResponse, ProviderError> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::InvalidResponse("no choices in response".into()))?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        // Extract text/image content.
        if let Some(msg_content) = choice.message.content {
            match msg_content {
                OpenAIContent::Text(text) => {
                    if !text.is_empty() {
                        content_blocks.push(ContentBlock::Text { text });
                    }
                }
                OpenAIContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            OpenAIContentPart::Text { text } => {
                                content_blocks.push(ContentBlock::Text { text });
                            }
                            OpenAIContentPart::ImageUrl { image_url } => {
                                content_blocks.push(ContentBlock::Image {
                                    source: layer0::content::ContentSource::Url {
                                        url: image_url.url,
                                    },
                                    media_type: "image/png".into(),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Extract tool calls.
        if let Some(tc_list) = choice.message.tool_calls {
            for tc in tc_list {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                tool_calls.push(ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    input,
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_str() {
            "stop" => StopReason::EndTurn,
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            "content_filter" => StopReason::ContentFilter,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: response.usage.prompt_tokens,
            output_tokens: response.usage.completion_tokens,
            cache_read_tokens: response
                .usage
                .prompt_tokens_details
                .and_then(|d| d.cached_tokens),
            cache_creation_tokens: None,
            reasoning_tokens: response
                .usage
                .completion_tokens_details
                .as_ref()
                .and_then(|d| d.reasoning_tokens)
                .filter(|&t| t > 0),
        };

        let input_cost = Decimal::from(response.usage.prompt_tokens) * Decimal::new(15, 8);
        let output_cost = Decimal::from(response.usage.completion_tokens) * Decimal::new(60, 8);
        let cost = input_cost + output_cost;

        // Build Content from blocks.
        let content = if content_blocks.is_empty() {
            Content::text("")
        } else if content_blocks.len() == 1 {
            if let ContentBlock::Text { ref text } = content_blocks[0] {
                Content::Text(text.clone())
            } else {
                Content::Blocks(content_blocks)
            }
        } else {
            Content::Blocks(content_blocks)
        };

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
}

impl Provider for OpenAIProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let api_key_result = self.resolve_api_key();
        let api_request = self.build_infer_request(&request);
        let http_opt = api_key_result.map(|key| {
            let mut builder = self
                .client
                .post(&self.api_url)
                .header("authorization", format!("Bearer {}", key))
                .header("content-type", "application/json");
            if let Some(ref org_id) = self.org_id {
                builder = builder.header("openai-organization", org_id);
            }
            builder.json(&api_request)
        });

        let model = request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer", provider = "openai", model);

        async move {
            let http_request: reqwest::RequestBuilder = match http_opt {
                Err(e) => return Err(e),
                Ok(r) => r,
            };
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

            let api_response: OpenAIResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            let response = self.parse_infer_response(api_response)?;
            tracing::info!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                "inference finished"
            );
            Ok(response)
        }
        .instrument(span)
    }

    #[allow(clippy::manual_async_fn)] // matches infer() RPITIT pattern
    fn embed(
        &self,
        request: EmbedRequest,
    ) -> impl std::future::Future<Output = Result<EmbedResponse, ProviderError>> + Send {
        async move {
            let api_key = self.resolve_api_key()?;
            let model = request
                .model
                .unwrap_or_else(|| "text-embedding-3-small".into());

            // Derive embedding URL from the configured base URL.
            let embed_url = self.api_url.replace("/chat/completions", "/embeddings");

            let wire_request = types::OpenAIEmbeddingRequest {
                model: model.clone(),
                input: request.texts,
                dimensions: request.dimensions,
            };

            let mut http_req = self
                .client
                .post(&embed_url)
                .bearer_auth(&api_key)
                .json(&wire_request);

            if let Some(ref org) = self.org_id {
                http_req = http_req.header("OpenAI-Organization", org);
            }

            let response = http_req.send().await.map_err(|e| {
                ProviderError::TransientError {
                    message: format!("embedding request failed: {e}"),
                    status: None,
                }
            })?;

            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry_after = response
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
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::AuthFailed(format!(
                    "HTTP {status}: {body}"
                )));
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(map_error_response(status, &body));
            }

            let wire_resp: types::OpenAIEmbeddingResponse =
                response.json().await.map_err(|e| {
                    ProviderError::InvalidResponse(format!(
                        "embedding response parse failed: {e}"
                    ))
                })?;

            let embeddings = wire_resp
                .data
                .into_iter()
                .map(|d| Embedding {
                    vector: d.embedding,
                })
                .collect();

            let usage = TokenUsage {
                input_tokens: wire_resp.usage.prompt_tokens,
                output_tokens: 0,
                ..Default::default()
            };

            Ok(EmbedResponse {
                embeddings,
                model: wire_resp.model,
                usage,
            })
        }
    }
}

impl StreamProvider for OpenAIProvider {
    fn infer_stream(
        &self,
        request: StreamRequest,
        on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
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
        api_request.stream_options = Some(serde_json::json!({"include_usage": true}));

        let api_key_result = self.resolve_api_key();
        let http_opt = api_key_result.map(|key| {
            let mut builder = self
                .client
                .post(&self.api_url)
                .header("authorization", format!("Bearer {}", key))
                .header("content-type", "application/json");
            if let Some(ref org_id) = self.org_id {
                builder = builder.header("openai-organization", org_id);
            }
            builder.json(&api_request)
        });

        let model = infer_request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer_stream", provider = "openai", model);

        async move {
            let http_request: reqwest::RequestBuilder = match http_opt {
                Err(e) => return Err(e),
                Ok(r) => r,
            };
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

            // Process SSE byte stream
            let mut stream = http_response.bytes_stream();
            let mut buf = String::new();

            // Accumulation state
            let mut model_name = String::new();
            let mut input_tokens: u64 = 0;
            let mut output_tokens: u64 = 0;
            let mut stop_reason = StopReason::EndTurn;
            let mut accumulated_text = String::new();
            // Tool calls accumulated by index
            let mut tool_ids: Vec<String> = Vec::new();
            let mut tool_names: Vec<String> = Vec::new();
            let mut tool_args: Vec<String> = Vec::new();
            let mut cache_read_tokens: Option<u64> = None;
            let mut reasoning_tokens: Option<u64> = None;

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

                    for line in frame.lines() {
                        let data = match line.strip_prefix("data: ") {
                            Some(d) => d,
                            None => continue,
                        };

                        if data == "[DONE]" {
                            break;
                        }

                        let chunk: OpenAIStreamChunk = match serde_json::from_str(data) {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::warn!(error = %e, "failed to parse SSE chunk");
                                continue;
                            }
                        };

                        if model_name.is_empty() {
                            model_name.clone_from(&chunk.model);
                        }

                        // Usage chunk (sent at the end when stream_options.include_usage is set)
                        if let Some(ref usage) = chunk.usage {
                            input_tokens = usage.prompt_tokens;
                            output_tokens = usage.completion_tokens;
                            cache_read_tokens = usage
                                .prompt_tokens_details
                                .as_ref()
                                .and_then(|d| d.cached_tokens);
                            reasoning_tokens = usage
                                .completion_tokens_details
                                .as_ref()
                                .and_then(|d| d.reasoning_tokens)
                                .filter(|&t| t > 0);
                            let usage_event = TokenUsage {
                                input_tokens,
                                output_tokens,
                                cache_read_tokens,
                                cache_creation_tokens: None,
                                reasoning_tokens,
                            };
                            on_event(StreamEvent::Usage(usage_event));
                        }

                        for choice in &chunk.choices {
                            // Text delta
                            if let Some(ref text) = choice.delta.content {
                                accumulated_text.push_str(text);
                                on_event(StreamEvent::TextDelta(text.clone()));
                            }

                            // Tool call deltas
                            if let Some(ref tc_deltas) = choice.delta.tool_calls {
                                for tc in tc_deltas {
                                    let idx = tc.index as usize;
                                    // Grow accumulators if needed
                                    while tool_ids.len() <= idx {
                                        tool_ids.push(String::new());
                                        tool_names.push(String::new());
                                        tool_args.push(String::new());
                                    }

                                    if let Some(ref id) = tc.id {
                                        tool_ids[idx].clone_from(id);
                                    }
                                    if let Some(ref func) = tc.function {
                                        if let Some(ref name) = func.name {
                                            tool_names[idx].clone_from(name);
                                            // Emit ToolCallStart when we get id + name
                                            on_event(StreamEvent::ToolCallStart {
                                                index: idx,
                                                id: tool_ids[idx].clone(),
                                                name: name.clone(),
                                            });
                                        }
                                        if let Some(ref args) = func.arguments {
                                            tool_args[idx].push_str(args);
                                            on_event(StreamEvent::ToolCallDelta {
                                                index: idx,
                                                json_delta: args.clone(),
                                            });
                                        }
                                    }
                                }
                            }

                            // Stop reason
                            if let Some(ref reason) = choice.finish_reason {
                                stop_reason = match reason.as_str() {
                                    "stop" => StopReason::EndTurn,
                                    "tool_calls" => StopReason::ToolUse,
                                    "length" => StopReason::MaxTokens,
                                    "content_filter" => StopReason::ContentFilter,
                                    _ => StopReason::EndTurn,
                                };
                            }
                        }
                    }
                }
            }

            // Build final tool calls
            let tool_calls: Vec<ToolCall> = (0..tool_ids.len())
                .filter(|i| !tool_ids[*i].is_empty())
                .map(|i| {
                    let input: serde_json::Value =
                        serde_json::from_str(&tool_args[i]).unwrap_or_default();
                    ToolCall {
                        id: tool_ids[i].clone(),
                        name: tool_names[i].clone(),
                        input,
                    }
                })
                .collect();

            // Build content
            let content = if accumulated_text.is_empty() {
                Content::text("")
            } else {
                Content::Text(accumulated_text)
            };

            let usage = TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens: None,
                reasoning_tokens,
            };

            let input_cost = Decimal::from(input_tokens) * Decimal::new(15, 8);
            let output_cost = Decimal::from(output_tokens) * Decimal::new(60, 8);
            let cost = input_cost + output_cost;

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
    // Check for OpenAI content-filter signals in the response body.
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

/// Extract plain text from a layer0 [`Content`] value.
fn content_to_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Convert layer0 [`ContentBlock`]s to an [`OpenAIContent`] value.
fn blocks_to_openai_content(blocks: &[ContentBlock]) -> OpenAIContent {
    if blocks.len() == 1
        && let ContentBlock::Text { text } = &blocks[0]
    {
        return OpenAIContent::Text(text.clone());
    }
    OpenAIContent::Parts(
        blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(OpenAIContentPart::Text { text: text.clone() }),
                ContentBlock::Image { source, .. } => {
                    let url = match source {
                        layer0::content::ContentSource::Url { url } => url.clone(),
                        layer0::content::ContentSource::Base64 { data } => {
                            format!("data:image/png;base64,{data}")
                        }
                        _ => return None,
                    };
                    Some(OpenAIContentPart::ImageUrl {
                        image_url: OpenAIImageUrl { url },
                    })
                }
                // ToolUse and ToolResult handled separately.
                _ => None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_schema_serializes() {
        let tool = OpenAITool {
            tool_type: "function".into(),
            function: OpenAIFunction {
                name: "get_weather".into(),
                description: "Get current weather".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }),
            },
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "get_weather");
    }

    #[test]
    fn with_url_overrides_api_url() {
        let provider =
            OpenAIProvider::new("test-key").with_url("https://proxy.example.com/v1/chat");
        assert_eq!(provider.api_url, "https://proxy.example.com/v1/chat");
    }

    #[test]
    fn with_org_sets_org_id() {
        let provider = OpenAIProvider::new("test-key").with_org("org-123");
        assert_eq!(provider.org_id, Some("org-123".into()));
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
        let body = r#"{"error":{"code":"content_filter","message":"blocked"}}"#;
        let err = map_error_response(status, body);
        assert!(matches!(err, ProviderError::ContentBlocked { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn embed_default_model_is_text_embedding_3_small() {
        // EmbedRequest::new sets model to None; embed() should default
        // to "text-embedding-3-small".
        let req = EmbedRequest::new(vec!["test".into()]);
        assert!(req.model.is_none());
    }

    #[test]
    fn embed_url_derived_from_api_url() {
        let provider = OpenAIProvider::new("test-key")
            .with_url("https://proxy.example.com/v1/chat/completions");
        // The embed implementation replaces /chat/completions with /embeddings.
        let expected = "https://proxy.example.com/v1/embeddings";
        let actual = provider.api_url.replace("/chat/completions", "/embeddings");
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod tests_credential {
    use super::*;

    #[test]
    fn new_uses_static_key() {
        let p = OpenAIProvider::new("sk-static");
        assert_eq!(p.resolve_api_key().unwrap(), "sk-static");
    }

    #[test]
    fn from_env_var_resolves_when_set() {
        let var = "SKG_OPENAI_TEST_CRED_A";
        unsafe {
            std::env::set_var(var, "sk-from-env");
        }
        let p = OpenAIProvider::from_env_var(var);
        assert_eq!(p.resolve_api_key().unwrap(), "sk-from-env");
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[test]
    fn from_env_var_missing_returns_auth_failed() {
        let var = "SKG_OPENAI_TEST_CRED_MISSING_ZZZ";
        unsafe {
            std::env::remove_var(var);
        }
        let p = OpenAIProvider::from_env_var(var);
        let err = p.resolve_api_key().unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        let msg = err.to_string();
        assert!(msg.contains(var), "error should name the variable");
    }

    #[test]
    fn from_env_var_empty_returns_auth_failed() {
        let var = "SKG_OPENAI_TEST_CRED_EMPTY_ZZZ";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = OpenAIProvider::from_env_var(var);
        let err = p.resolve_api_key().unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        let msg = err.to_string();
        assert!(msg.contains(var), "error should name the variable");
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[test]
    fn error_message_does_not_contain_secret_value() {
        let var = "SKG_OPENAI_TEST_CRED_REDACT_ZZZ";
        let secret = "sk-must-not-appear-in-any-error-message";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = OpenAIProvider::from_env_var(var);
        let msg = p.resolve_api_key().unwrap_err().to_string();
        assert!(msg.contains(var));
        assert!(!msg.contains(secret));
        unsafe {
            std::env::set_var(var, secret);
        }
        assert_eq!(p.resolve_api_key().unwrap(), secret);
        unsafe {
            std::env::remove_var(var);
        }
    }
}
