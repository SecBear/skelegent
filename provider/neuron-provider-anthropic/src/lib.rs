#![deny(missing_docs)]
//! Anthropic API provider for neuron-turn.
//!
//! Implements the [`neuron_turn::Provider`] trait for Anthropic's Messages API.

mod types;

use neuron_turn::provider::{Provider, ProviderError};
use neuron_turn::types::*;
use rust_decimal::Decimal;
use types::*;

/// Anthropic API provider.
pub struct AnthropicProvider {
    api_key: String,
    client: reqwest::Client,
    api_url: String,
    api_version: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    /// Override the API URL (for testing or proxies).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    fn build_request(&self, request: &ProviderRequest) -> AnthropicRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "claude-haiku-4-5-20251001".into());
        let max_tokens = request.max_tokens.unwrap_or(4096);

        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user".into(),
                    Role::Assistant => "assistant".into(),
                    Role::System => "user".into(), // System messages go in the system field
                },
                content: parts_to_anthropic_content(&m.content),
            })
            .collect();

        let tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect();

        AnthropicRequest {
            model,
            max_tokens,
            messages,
            system: request.system.clone(),
            tools,
        }
    }

    fn parse_response(&self, response: AnthropicResponse) -> ProviderResponse {
        let content: Vec<ContentPart> = response
            .content
            .iter()
            .map(anthropic_block_to_content_part)
            .collect();

        let stop_reason = match response.stop_reason.as_str() {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cache_read_tokens: response.usage.cache_read_input_tokens,
            cache_creation_tokens: response.usage.cache_creation_input_tokens,
        };

        // Simple cost calculation for Haiku
        // Haiku: $0.25/MTok input, $1.25/MTok output (as of 2025)
        let input_cost = Decimal::from(response.usage.input_tokens) * Decimal::new(25, 8);
        let output_cost = Decimal::from(response.usage.output_tokens) * Decimal::new(125, 8);
        let cost = input_cost + output_cost;

        ProviderResponse {
            content,
            stop_reason,
            usage,
            model: response.model,
            cost: Some(cost),
            truncated: None,
        }
    }
}

impl Provider for AnthropicProvider {
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send {
        let api_request = self.build_request(&request);
        let http_request = self
            .client
            .post(&self.api_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .header("content-type", "application/json")
            .json(&api_request);

        async move {
            let http_response = http_request
                .send()
                .await
                .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

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
                return Err(ProviderError::RequestFailed(format!(
                    "HTTP {status}: {body}"
                )));
            }

            let api_response: AnthropicResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            Ok(self.parse_response(api_response))
        }
    }
}

fn parts_to_anthropic_content(parts: &[ContentPart]) -> AnthropicContent {
    if parts.len() == 1
        && let ContentPart::Text { text } = &parts[0]
    {
        return AnthropicContent::Text(text.clone());
    }
    AnthropicContent::Blocks(parts.iter().map(content_part_to_anthropic_block).collect())
}

fn content_part_to_anthropic_block(part: &ContentPart) -> AnthropicContentBlock {
    match part {
        ContentPart::Text { text } => AnthropicContentBlock::Text { text: text.clone() },
        ContentPart::ToolUse { id, name, input } => AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ContentPart::Image { source, media_type } => AnthropicContentBlock::Image {
            source: match source {
                ImageSource::Base64 { data } => AnthropicImageSource::Base64 { data: data.clone() },
                ImageSource::Url { url } => AnthropicImageSource::Url { url: url.clone() },
            },
            media_type: media_type.clone(),
        },
    }
}

fn anthropic_block_to_content_part(block: &AnthropicContentBlock) -> ContentPart {
    match block {
        AnthropicContentBlock::Text { text } => ContentPart::Text { text: text.clone() },
        AnthropicContentBlock::ToolUse { id, name, input } => ContentPart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        AnthropicContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentPart::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        AnthropicContentBlock::Image { source, media_type } => ContentPart::Image {
            source: match source {
                AnthropicImageSource::Base64 { data } => ImageSource::Base64 { data: data.clone() },
                AnthropicImageSource::Url { url } => ImageSource::Url { url: url.clone() },
            },
            media_type: media_type.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_simple_request() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: Some("claude-haiku-4-5-20251001".into()),
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
        assert_eq!(api_request.model, "claude-haiku-4-5-20251001");
        assert_eq!(api_request.max_tokens, 256);
        assert_eq!(api_request.messages.len(), 1);
        assert_eq!(api_request.messages[0].role, "user");
        assert_eq!(api_request.system, Some("Be helpful.".into()));
    }

    #[test]
    fn parse_simple_response() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "Hello!".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "end_turn".into(),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
        assert!(response.cost.is_some());
        assert_eq!(response.content.len(), 1);
    }

    #[test]
    fn parse_tool_use_response() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "bash".into(),
                input: json!({"command": "ls"}),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "tool_use".into(),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 30,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::ToolUse { name, .. } => assert_eq!(name, "bash"),
            _ => panic!("expected ToolUse"),
        }
    }

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
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "get_weather");
    }

    #[test]
    fn parse_cache_tokens() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "Cached.".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "end_turn".into(),
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 10,
                cache_read_input_tokens: Some(50),
                cache_creation_input_tokens: Some(25),
            },
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.usage.cache_read_tokens, Some(50));
        assert_eq!(response.usage.cache_creation_tokens, Some(25));
    }

    #[test]
    fn default_model_is_haiku() {
        let provider = AnthropicProvider::new("test-key");
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
        assert_eq!(api_request.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn default_max_tokens_is_4096() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.max_tokens, 4096);
    }

    #[test]
    fn tool_result_in_request() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![
                ProviderMessage {
                    role: Role::Assistant,
                    content: vec![ContentPart::ToolUse {
                        id: "tu_1".into(),
                        name: "bash".into(),
                        input: json!({"cmd": "ls"}),
                    }],
                },
                ProviderMessage {
                    role: Role::User,
                    content: vec![ContentPart::ToolResult {
                        tool_use_id: "tu_1".into(),
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
        assert_eq!(api_request.messages.len(), 2);
        assert_eq!(api_request.messages[0].role, "assistant");
        assert_eq!(api_request.messages[1].role, "user");
    }

    #[test]
    fn image_content_in_request() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Image {
                    source: ImageSource::Base64 {
                        data: "aGVsbG8=".into(),
                    },
                    media_type: "image/png".into(),
                }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        // Should build without panic. Content should be blocks with image.
        assert_eq!(api_request.messages.len(), 1);
    }

    #[test]
    fn parse_tool_use_stop_reason() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "bash".into(),
                input: json!({"cmd": "ls"}),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "tool_use".into(),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn parse_max_tokens_stop_reason() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "trunca...".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "max_tokens".into(),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 100,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = provider.parse_response(api_response);
        assert_eq!(response.stop_reason, StopReason::MaxTokens);
    }

    #[test]
    fn with_url_overrides_api_url() {
        let provider =
            AnthropicProvider::new("test-key").with_url("https://proxy.example.com/v1/messages");
        assert_eq!(provider.api_url, "https://proxy.example.com/v1/messages");
    }

    #[test]
    fn cost_calculation_is_positive() {
        let provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "Hello".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "end_turn".into(),
            usage: AnthropicUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = provider.parse_response(api_response);
        let cost = response.cost.unwrap();
        assert!(cost > Decimal::ZERO);
    }
}
