#![deny(missing_docs)]
//! OpenAI API provider for neuron-turn.
//!
//! Implements the [`neuron_turn::Provider`] trait for OpenAI's Chat Completions API.

mod types;

use layer0::content::{Content, ContentBlock};
use layer0::context;
use neuron_turn::infer::{InferRequest, InferResponse, ToolCall};
use neuron_turn::provider::{Provider, ProviderError};
use neuron_turn::types::*;
use rust_decimal::Decimal;
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

    fn build_request(&self, request: &ProviderRequest) -> OpenAIRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "gpt-4o-mini".into());
        let max_tokens = request.max_tokens;

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

        // Map ProviderMessages to OpenAI messages.
        for m in &request.messages {
            match m.role {
                Role::System => {
                    // Additional system messages.
                    let text = extract_text(&m.content);
                    messages.push(OpenAIMessage {
                        role: "system".into(),
                        content: Some(OpenAIContent::Text(text)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
                Role::User => {
                    // Check if any content part is a tool result.
                    // OpenAI uses role="tool" for tool results, not user messages.
                    let mut tool_results = Vec::new();
                    let mut other_parts = Vec::new();
                    for part in &m.content {
                        match part {
                            ContentPart::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                tool_results.push((tool_use_id.clone(), content.clone()));
                            }
                            _ => {
                                other_parts.push(part.clone());
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
                            content: Some(parts_to_openai_content(&other_parts)),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
                Role::Assistant => {
                    // Check for tool use parts.
                    let mut tool_calls = Vec::new();
                    let mut text_parts = Vec::new();
                    for part in &m.content {
                        match part {
                            ContentPart::ToolUse { id, name, input } => {
                                tool_calls.push(OpenAIToolCall {
                                    id: id.clone(),
                                    call_type: "function".into(),
                                    function: OpenAIFunctionCall {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input).unwrap_or_default(),
                                    },
                                });
                            }
                            _ => {
                                text_parts.push(part.clone());
                            }
                        }
                    }

                    let content = if text_parts.is_empty() {
                        None
                    } else {
                        Some(parts_to_openai_content(&text_parts))
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
            max_tokens,
            temperature: request.temperature,
            tools,
            parallel_tool_calls,
            service_tier,
            reasoning_effort,
        }
    }

    fn parse_response(&self, response: OpenAIResponse) -> Result<ProviderResponse, ProviderError> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::InvalidResponse("no choices in response".into()))?;

        let mut content: Vec<ContentPart> = Vec::new();

        // Extract text content.
        if let Some(msg_content) = choice.message.content {
            match msg_content {
                OpenAIContent::Text(text) => {
                    if !text.is_empty() {
                        content.push(ContentPart::Text { text });
                    }
                }
                OpenAIContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            OpenAIContentPart::Text { text } => {
                                content.push(ContentPart::Text { text });
                            }
                            OpenAIContentPart::ImageUrl { image_url } => {
                                content.push(ContentPart::Image {
                                    source: ImageSource::Url { url: image_url.url },
                                    media_type: "image/png".into(),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Extract tool calls.
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                content.push(ContentPart::ToolUse {
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
        };

        // Cost calculation for gpt-4o-mini: $0.15/MTok input, $0.60/MTok output
        // $0.15 per 1M tokens = $0.00000015 per token = 15e-8
        // $0.60 per 1M tokens = $0.0000006 per token = 60e-8
        let input_cost = Decimal::from(response.usage.prompt_tokens) * Decimal::new(15, 8);
        let output_cost = Decimal::from(response.usage.completion_tokens) * Decimal::new(60, 8);
        let cost = input_cost + output_cost;

        Ok(ProviderResponse {
            content,
            stop_reason,
            usage,
            model: response.model,
            cost: Some(cost),
            truncated: None,
        })
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
                                        tool_results
                                            .push((tool_use_id.clone(), content.clone()));
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
                                    ContentBlock::ToolResult {
                                        tool_use_id, ..
                                    } => Some(tool_use_id.clone()),
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
                                    source: layer0::content::ImageSource::Url { url: image_url.url },
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
        };

        let input_cost = Decimal::from(response.usage.prompt_tokens) * Decimal::new(15, 8);
        let output_cost =
            Decimal::from(response.usage.completion_tokens) * Decimal::new(60, 8);
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
            let http_response = http_request.send().await.map_err(|e| {
                ProviderError::TransientError {
                    message: e.to_string(),
                    status: None,
                }
            })?;

            let status = http_response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited);
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

    #[allow(deprecated)]
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send {
        let api_key_result = self.resolve_api_key();
        let api_request = self.build_request(&request);
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
        let span = tracing::info_span!("provider.complete", provider = "openai", model);

        async move {
            let http_request = match http_opt {
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
                return Err(ProviderError::RateLimited);
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

            let response = self.parse_response(api_response)?;
            tracing::info!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                "completion finished"
            );
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
                ContentBlock::Text { text } => {
                    Some(OpenAIContentPart::Text { text: text.clone() })
                }
                ContentBlock::Image { source, .. } => {
                    let url = match source {
                        layer0::content::ImageSource::Url { url } => url.clone(),
                        layer0::content::ImageSource::Base64 { data } => {
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

fn extract_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parts_to_openai_content(parts: &[ContentPart]) -> OpenAIContent {
    if parts.len() == 1
        && let ContentPart::Text { text } = &parts[0]
    {
        return OpenAIContent::Text(text.clone());
    }
    OpenAIContent::Parts(
        parts
            .iter()
            .filter_map(content_part_to_openai_part)
            .collect(),
    )
}

fn content_part_to_openai_part(part: &ContentPart) -> Option<OpenAIContentPart> {
    match part {
        ContentPart::Text { text } => Some(OpenAIContentPart::Text { text: text.clone() }),
        ContentPart::Image { source, .. } => {
            let url = match source {
                ImageSource::Url { url } => url.clone(),
                ImageSource::Base64 { data } => format!("data:image/png;base64,{data}"),
            };
            Some(OpenAIContentPart::ImageUrl {
                image_url: OpenAIImageUrl { url },
            })
        }
        // ToolUse and ToolResult are handled separately, not as content parts.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_simple_request() {
        let provider = OpenAIProvider::new("test-key");
        let request = ProviderRequest {
            model: Some("gpt-4o-mini".into()),
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: Some(256),
            temperature: None,
            system: Some("Be helpful.".into()),
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "gpt-4o-mini");
        assert_eq!(api_request.max_tokens, Some(256));
        // System prompt becomes the first message.
        assert_eq!(api_request.messages.len(), 2);
        assert_eq!(api_request.messages[0].role, "system");
        match &api_request.messages[0].content {
            Some(OpenAIContent::Text(t)) => assert_eq!(t, "Be helpful."),
            _ => panic!("expected system message text"),
        }
        assert_eq!(api_request.messages[1].role, "user");
    }

    #[test]
    fn parse_simple_response() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-123".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: Some(OpenAIContent::Text("Hello!".into())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: "stop".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
        assert!(response.cost.is_some());
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn parse_tool_use_response() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-456".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(vec![OpenAIToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: OpenAIFunctionCall {
                            name: "bash".into(),
                            arguments: r#"{"command": "ls"}"#.into(),
                        },
                    }]),
                    tool_call_id: None,
                },
                finish_reason: "tool_calls".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 20,
                completion_tokens: 30,
                total_tokens: 50,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::ToolUse { id, name, input } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "bash");
                assert_eq!(input, &json!({"command": "ls"}));
            }
            _ => panic!("expected ToolUse"),
        }
    }

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
    fn parse_string_tool_arguments() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-789".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(vec![OpenAIToolCall {
                        id: "call_2".into(),
                        call_type: "function".into(),
                        function: OpenAIFunctionCall {
                            name: "calculator".into(),
                            arguments: r#"{"expression": "2 + 2", "format": "decimal"}"#.into(),
                        },
                    }]),
                    tool_call_id: None,
                },
                finish_reason: "tool_calls".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 15,
                completion_tokens: 25,
                total_tokens: 40,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        match &response.content[0] {
            ContentPart::ToolUse { input, .. } => {
                assert_eq!(input["expression"], "2 + 2");
                assert_eq!(input["format"], "decimal");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn service_tier_in_request() {
        let provider = OpenAIProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!({
                "service_tier": "auto",
                "reasoning_effort": "high",
                "parallel_tool_calls": false
            }),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.service_tier, Some("auto".into()));
        assert_eq!(api_request.reasoning_effort, Some("high".into()));
        assert_eq!(api_request.parallel_tool_calls, Some(false));
    }

    #[test]
    fn tool_result_becomes_tool_role_message() {
        let provider = OpenAIProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![
                ProviderMessage {
                    role: Role::Assistant,
                    content: vec![ContentPart::ToolUse {
                        id: "call_1".into(),
                        name: "bash".into(),
                        input: json!({"command": "ls"}),
                    }],
                },
                ProviderMessage {
                    role: Role::User,
                    content: vec![ContentPart::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: "file.txt".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        // Assistant message with tool_calls.
        assert_eq!(api_request.messages[0].role, "assistant");
        assert!(api_request.messages[0].tool_calls.is_some());
        // Tool result becomes role="tool" message.
        assert_eq!(api_request.messages[1].role, "tool");
        assert_eq!(api_request.messages[1].tool_call_id, Some("call_1".into()));
    }

    #[test]
    fn default_model_is_gpt4o_mini() {
        let provider = OpenAIProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text { text: "Hi".into() }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "gpt-4o-mini");
    }

    #[test]
    fn parse_empty_choices_returns_error() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-empty".into(),
            choices: vec![],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 5,
                completion_tokens: 0,
                total_tokens: 5,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let result = provider.parse_response(api_response);
        assert!(result.is_err());
    }

    #[test]
    fn parse_cache_token_details() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-cache".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: Some(OpenAIContent::Text("Cached!".into())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: "stop".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 100,
                completion_tokens: 10,
                total_tokens: 110,
                prompt_tokens_details: Some(OpenAIPromptTokensDetails {
                    cached_tokens: Some(50),
                }),
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        assert_eq!(response.usage.cache_read_tokens, Some(50));
    }

    #[test]
    fn parse_multiple_tool_calls() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-multi".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(vec![
                        OpenAIToolCall {
                            id: "call_1".into(),
                            call_type: "function".into(),
                            function: OpenAIFunctionCall {
                                name: "bash".into(),
                                arguments: r#"{"command": "ls"}"#.into(),
                            },
                        },
                        OpenAIToolCall {
                            id: "call_2".into(),
                            call_type: "function".into(),
                            function: OpenAIFunctionCall {
                                name: "read".into(),
                                arguments: r#"{"file": "test.txt"}"#.into(),
                            },
                        },
                    ]),
                    tool_call_id: None,
                },
                finish_reason: "tool_calls".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 20,
                completion_tokens: 30,
                total_tokens: 50,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        assert_eq!(response.content.len(), 2);
        match &response.content[0] {
            ContentPart::ToolUse { id, name, .. } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "bash");
            }
            _ => panic!("expected ToolUse"),
        }
        match &response.content[1] {
            ContentPart::ToolUse { id, name, .. } => {
                assert_eq!(id, "call_2");
                assert_eq!(name, "read");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn parse_length_finish_reason() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-len".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: Some(OpenAIContent::Text("trunca...".into())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: "length".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 100,
                total_tokens: 110,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let response = provider.parse_response(api_response).unwrap();
        assert_eq!(response.stop_reason, StopReason::MaxTokens);
    }

    #[test]
    fn parse_content_filter_finish_reason_maps_to_ok() {
        let provider = OpenAIProvider::new("test-key");
        let api_response = OpenAIResponse {
            id: "chatcmpl-filter".into(),
            choices: vec![OpenAIChoice {
                message: OpenAIMessage {
                    role: "assistant".into(),
                    content: Some(OpenAIContent::Text(String::new())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: "content_filter".into(),
                index: 0,
            }],
            model: "gpt-4o-mini".into(),
            usage: OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 0,
                total_tokens: 10,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            service_tier: None,
        };

        let result = provider.parse_response(api_response);
        let resp = result.expect("content_filter finish_reason should be Ok, not Err");
        assert_eq!(resp.stop_reason, StopReason::ContentFilter);
    }

    #[test]
    fn build_request_with_tools() {
        let provider = OpenAIProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Help me".into(),
                }],
            }],
            tools: vec![ToolSchema {
                name: "bash".into(),
                description: "Run a command".into(),
                input_schema: json!({"type": "object", "properties": {"cmd": {"type": "string"}}}),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.tools.len(), 1);
        assert_eq!(api_request.tools[0].tool_type, "function");
        assert_eq!(api_request.tools[0].function.name, "bash");
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
        let var = "NEURON_OPENAI_TEST_CRED_A";
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
        let var = "NEURON_OPENAI_TEST_CRED_MISSING_ZZZ";
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
        let var = "NEURON_OPENAI_TEST_CRED_EMPTY_ZZZ";
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
        let var = "NEURON_OPENAI_TEST_CRED_REDACT_ZZZ";
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
