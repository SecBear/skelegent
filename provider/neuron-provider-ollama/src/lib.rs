#![deny(missing_docs)]
//! Ollama local model provider for neuron-turn.
//!
//! Implements the [`neuron_turn::Provider`] trait for Ollama's `/api/chat` endpoint.
//! Ollama runs models locally, so there are no auth headers and cost is always zero.

mod types;

use layer0::content::{Content, ContentBlock};
use layer0::context::Role as Layer0Role;
use neuron_turn::infer::{InferRequest, InferResponse, ToolCall};
use neuron_turn::provider::{Provider, ProviderError};
use neuron_turn::types::*;
use rust_decimal::Decimal;
use tracing::Instrument;
use types::*;
use uuid::Uuid;

/// Ollama local model provider.
pub struct OllamaProvider {
    client: reqwest::Client,
    api_url: String,
    keep_alive: Option<String>,
}

impl OllamaProvider {
    /// Create a new Ollama provider pointed at the default local endpoint.
    ///
    /// Defaults to `http://localhost:11434/api/chat`.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_url: "http://localhost:11434/api/chat".into(),
            keep_alive: None,
        }
    }

    /// Override the API URL (for remote Ollama instances or custom ports).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    /// Set the `keep_alive` duration for how long Ollama keeps the model loaded.
    ///
    /// Examples: `"5m"`, `"0"` (unload immediately), `"-1"` (keep forever).
    pub fn with_keep_alive(mut self, duration: impl Into<String>) -> Self {
        self.keep_alive = Some(duration.into());
        self
    }

    /// Build an [`OllamaRequest`] from an [`InferRequest`] (layer0 `Message` types).
    fn build_infer_request(&self, request: &InferRequest) -> OllamaRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "llama3.2:1b".into());

        let mut messages: Vec<OllamaMessage> = Vec::new();

        // System prompt becomes a system message.
        if let Some(ref system) = request.system {
            messages.push(OllamaMessage {
                role: "system".into(),
                content: system.clone(),
                tool_calls: None,
            });
        }

        // Map layer0 Messages to Ollama messages.
        for m in &request.messages {
            match &m.role {
                Layer0Role::System => {
                    let text = content_text(&m.content);
                    messages.push(OllamaMessage {
                        role: "system".into(),
                        content: text,
                        tool_calls: None,
                    });
                }
                Layer0Role::User => {
                    // Separate tool results from other content.
                    let mut tool_results = Vec::new();
                    let mut other_text = Vec::new();
                    match &m.content {
                        Content::Text(s) => {
                            other_text.push(s.clone());
                        }
                        Content::Blocks(blocks) => {
                            for block in blocks {
                                match block {
                                    ContentBlock::ToolResult { content, .. } => {
                                        tool_results.push(content.clone());
                                    }
                                    ContentBlock::Text { text } => {
                                        other_text.push(text.clone());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }

                    // Emit tool result messages as role="tool".
                    for content in tool_results {
                        messages.push(OllamaMessage {
                            role: "tool".into(),
                            content,
                            tool_calls: None,
                        });
                    }

                    // Emit user message if there is text content.
                    if !other_text.is_empty() {
                        messages.push(OllamaMessage {
                            role: "user".into(),
                            content: other_text.join("\n"),
                            tool_calls: None,
                        });
                    }
                }
                Layer0Role::Assistant => {
                    let mut tool_calls = Vec::new();
                    let mut text_parts = Vec::new();
                    match &m.content {
                        Content::Text(s) => {
                            text_parts.push(s.clone());
                        }
                        Content::Blocks(blocks) => {
                            for block in blocks {
                                match block {
                                    ContentBlock::ToolUse { name, input, .. } => {
                                        tool_calls.push(OllamaToolCall {
                                            function: OllamaFunctionCall {
                                                name: name.clone(),
                                                arguments: input.clone(),
                                            },
                                        });
                                    }
                                    ContentBlock::Text { text } => {
                                        text_parts.push(text.clone());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }

                    let content = text_parts.join("\n");
                    let tool_calls_field = if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    };

                    messages.push(OllamaMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls: tool_calls_field,
                    });
                }
                Layer0Role::Tool { .. } => {
                    // Tool role messages contain tool results.
                    let text = content_text(&m.content);
                    messages.push(OllamaMessage {
                        role: "tool".into(),
                        content: text,
                        tool_calls: None,
                    });
                }
                _ => {}
            }
        }

        let tools: Vec<OllamaTool> = request
            .tools
            .iter()
            .map(|t| OllamaTool {
                tool_type: "function".into(),
                function: OllamaFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        let options = if request.temperature.is_some() || request.max_tokens.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                ..Default::default()
            })
        } else {
            None
        };

        OllamaRequest {
            model,
            messages,
            stream: false,
            tools,
            keep_alive: self.keep_alive.clone(),
            options,
        }
    }

    /// Parse an [`OllamaResponse`] into an [`InferResponse`].
    fn parse_infer_response(&self, response: OllamaResponse) -> InferResponse {
        // Extract text content.
        let content = if response.message.content.is_empty() {
            Content::text(String::new())
        } else {
            Content::text(response.message.content.clone())
        };

        // Extract tool calls, synthesizing UUIDs since Ollama does not provide IDs.
        let has_tool_calls = response
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty());

        let tool_calls: Vec<ToolCall> = response
            .message
            .tool_calls
            .as_ref()
            .map(|tcs| {
                tcs.iter()
                    .map(|tc| ToolCall {
                        id: Uuid::new_v4().to_string(),
                        name: tc.function.name.clone(),
                        input: tc.function.arguments.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Determine stop reason.
        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            match response.done_reason.as_deref() {
                Some("stop") => StopReason::EndTurn,
                Some("length") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            }
        };

        let usage = TokenUsage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
            cache_read_tokens: None,
            cache_creation_tokens: None,
        };

        InferResponse {
            content,
            tool_calls,
            stop_reason,
            usage,
            model: response.model,
            cost: Some(Decimal::ZERO),
            truncated: None,
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OllamaProvider {
    fn infer(
        &self,
        request: InferRequest,
    ) -> impl std::future::Future<Output = Result<InferResponse, ProviderError>> + Send {
        let api_request = self.build_infer_request(&request);
        let http_request = self
            .client
            .post(&self.api_url)
            .header("content-type", "application/json")
            .json(&api_request);

        let model = request.model.as_deref().unwrap_or("unknown");
        let span = tracing::info_span!("provider.infer", provider = "ollama", model);

        async move {
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

            let api_response: OllamaResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            let response = self.parse_infer_response(api_response);
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

/// Extract plain text from layer0 [`Content`].
fn content_text(content: &Content) -> String {
    match content {
        Content::Text(s) => s.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Map a non-success HTTP response to an appropriate [`ProviderError`].
///
/// Ollama has no content-safety filter, so all non-success, non-auth, non-rate-limit
/// responses are treated as transient errors.
fn map_error_response(status: reqwest::StatusCode, body: &str) -> ProviderError {
    let status_u16 = status.as_u16();
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
        let tool = OllamaTool {
            tool_type: "function".into(),
            function: OllamaFunction {
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
        assert!(json["function"]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn with_url_overrides_api_url() {
        let provider = OllamaProvider::new().with_url("http://remote:11434/api/chat");
        assert_eq!(provider.api_url, "http://remote:11434/api/chat");
    }

    #[test]
    fn ollama_default_impl() {
        let provider = OllamaProvider::default();
        assert_eq!(provider.api_url, "http://localhost:11434/api/chat");
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
        let err = map_error_response(status, "model not available");
        assert!(matches!(
            err,
            ProviderError::TransientError {
                status: Some(503),
                ..
            }
        ));
        assert!(err.is_retryable());
    }
}
