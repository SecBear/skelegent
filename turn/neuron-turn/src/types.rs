//! Internal types for the neuron-turn ReAct loop.
//!
//! These are the internal lingua franca — not layer0 types, not
//! provider-specific types. Providers convert to/from these.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System message (instructions).
    System,
    /// User message.
    User,
    /// Assistant (model) message.
    Assistant,
}

/// Source for image content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    /// Base64-encoded image data.
    Base64 {
        /// The base64-encoded data.
        data: String,
    },
    /// URL pointing to an image.
    Url {
        /// The image URL.
        url: String,
    },
}

/// A single content part within a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Plain text.
    Text {
        /// The text content.
        text: String,
    },
    /// A tool use request from the model.
    ToolUse {
        /// Unique identifier for this tool use.
        id: String,
        /// Name of the tool to invoke.
        name: String,
        /// Tool input parameters.
        input: serde_json::Value,
    },
    /// Result from a tool execution.
    ToolResult {
        /// The tool_use id this result corresponds to.
        tool_use_id: String,
        /// The result content.
        content: String,
        /// Whether the tool execution errored.
        is_error: bool,
    },
    /// Image content.
    Image {
        /// The image source.
        source: ImageSource,
        /// MIME type of the image.
        media_type: String,
    },
}

/// A message in the provider conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderMessage {
    /// The role of the message author.
    pub role: Role,
    /// Content parts of the message.
    pub content: Vec<ContentPart>,
}

/// JSON Schema description of a tool for the provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input.
    pub input_schema: serde_json::Value,
}

/// Request sent to a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    /// Model to use (None = provider default).
    pub model: Option<String>,
    /// Conversation messages.
    pub messages: Vec<ProviderMessage>,
    /// Available tools.
    pub tools: Vec<ToolSchema>,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// System prompt.
    pub system: Option<String>,
    /// Provider-specific config passthrough.
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// Why the provider stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model produced a final response.
    EndTurn,
    /// Model wants to use a tool.
    ToolUse,
    /// Hit the max_tokens limit.
    MaxTokens,
    /// Content was filtered by safety.
    ContentFilter,
}

/// Token usage from a single provider call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Tokens read from cache (if supported).
    pub cache_read_tokens: Option<u64>,
    /// Tokens written to cache (if supported).
    pub cache_creation_tokens: Option<u64>,
}

/// Response from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    /// Response content parts.
    pub content: Vec<ContentPart>,
    /// Why the provider stopped.
    pub stop_reason: StopReason,
    /// Token usage.
    pub usage: TokenUsage,
    /// Actual model used.
    pub model: String,
    /// Cost calculated by the provider (None if unknown).
    pub cost: Option<Decimal>,
    /// Whether the provider truncated input (telemetry only).
    pub truncated: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn role_serde_roundtrip() {
        for role in [Role::System, Role::User, Role::Assistant] {
            let json = serde_json::to_string(&role).unwrap();
            let back: Role = serde_json::from_str(&json).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn content_part_text_roundtrip() {
        let part = ContentPart::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "text");
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }

    #[test]
    fn content_part_tool_use_roundtrip() {
        let part = ContentPart::ToolUse {
            id: "tu_1".into(),
            name: "bash".into(),
            input: json!({"command": "ls"}),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "tool_use");
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }

    #[test]
    fn content_part_tool_result_roundtrip() {
        let part = ContentPart::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "file.txt".into(),
            is_error: false,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "tool_result");
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }

    #[test]
    fn content_part_image_roundtrip() {
        let part = ContentPart::Image {
            source: ImageSource::Url {
                url: "https://example.com/img.png".into(),
            },
            media_type: "image/png".into(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "image");
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }

    #[test]
    fn stop_reason_roundtrip() {
        for reason in [
            StopReason::EndTurn,
            StopReason::ToolUse,
            StopReason::MaxTokens,
            StopReason::ContentFilter,
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(reason, back);
        }
    }

    #[test]
    fn provider_message_roundtrip() {
        let msg = ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "hello".into(),
            }],
        };
        let json = serde_json::to_value(&msg).unwrap();
        let back: ProviderMessage = serde_json::from_value(json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert!(usage.cache_read_tokens.is_none());
    }

    #[test]
    fn token_usage_serde_roundtrip() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: Some(10),
            cache_creation_tokens: Some(5),
        };
        let json = serde_json::to_value(&usage).unwrap();
        let back: TokenUsage = serde_json::from_value(json).unwrap();
        assert_eq!(usage, back);
    }

    #[test]
    fn image_source_base64_roundtrip() {
        let source = ImageSource::Base64 {
            data: "aGVsbG8=".into(),
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["type"], "base64");
        let back: ImageSource = serde_json::from_value(json).unwrap();
        assert_eq!(source, back);
    }

    #[test]
    fn image_source_url_roundtrip() {
        let source = ImageSource::Url {
            url: "https://example.com/img.png".into(),
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["type"], "url");
        let back: ImageSource = serde_json::from_value(json).unwrap();
        assert_eq!(source, back);
    }

    #[test]
    fn provider_request_serde_roundtrip() {
        let request = ProviderRequest {
            model: Some("test-model".into()),
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "hello".into(),
                }],
            }],
            tools: vec![ToolSchema {
                name: "bash".into(),
                description: "Run a command".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: Some("Be helpful".into()),
            extra: json!({"key": "value"}),
        };
        let json = serde_json::to_value(&request).unwrap();
        let back: ProviderRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back.model, Some("test-model".into()));
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.tools.len(), 1);
        assert_eq!(back.max_tokens, Some(1024));
        assert_eq!(back.system, Some("Be helpful".into()));
    }

    #[test]
    fn provider_response_serde_roundtrip() {
        let response = ProviderResponse {
            content: vec![ContentPart::Text {
                text: "hello".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            },
            model: "test-model".into(),
            cost: Some(rust_decimal::Decimal::new(1, 4)),
            truncated: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        let back: ProviderResponse = serde_json::from_value(json).unwrap();
        assert_eq!(back.model, "test-model");
        assert_eq!(back.stop_reason, StopReason::EndTurn);
        assert_eq!(back.content.len(), 1);
    }

    #[test]
    fn content_part_image_base64_roundtrip() {
        let part = ContentPart::Image {
            source: ImageSource::Base64 {
                data: "aGVsbG8=".into(),
            },
            media_type: "image/jpeg".into(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "image");
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }

    #[test]
    fn provider_message_multi_content_roundtrip() {
        let msg = ProviderMessage {
            role: Role::Assistant,
            content: vec![
                ContentPart::Text {
                    text: "Let me help.".into(),
                },
                ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "bash".into(),
                    input: json!({"cmd": "ls"}),
                },
            ],
        };
        let json = serde_json::to_value(&msg).unwrap();
        let back: ProviderMessage = serde_json::from_value(json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn tool_result_with_error_roundtrip() {
        let part = ContentPart::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "command failed".into(),
            is_error: true,
        };
        let json = serde_json::to_value(&part).unwrap();
        let back: ContentPart = serde_json::from_value(json).unwrap();
        assert_eq!(part, back);
    }
}
