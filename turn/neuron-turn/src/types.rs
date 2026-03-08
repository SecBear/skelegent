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

// ── Conversions between internal types and layer0 types ─────────────

use layer0::content::{Content, ContentBlock};
use layer0::context::{Message, Role as L0Role};

/// Convert an internal `Role` to a layer0 `Role`.
///
/// Note: `turn::Role` has no `Tool` variant — tool results are content-level
/// in the provider wire format, not role-level.
pub fn role_to_layer0(role: &Role) -> L0Role {
    match role {
        Role::System => L0Role::System,
        Role::User => L0Role::User,
        Role::Assistant => L0Role::Assistant,
    }
}

/// Convert a layer0 `Role` to an internal `Role`.
///
/// The `Tool` variant maps to `User` because provider wire format
/// encodes tool results as user-role messages with `ToolResult` content.
pub fn role_from_layer0(role: &L0Role) -> Role {
    match role {
        L0Role::System => Role::System,
        L0Role::User => Role::User,
        L0Role::Assistant => Role::Assistant,
        L0Role::Tool { .. } => Role::User,
        // Handle non_exhaustive
        _ => Role::User,
    }
}

/// Convert a layer0 `ContentBlock` to an internal `ContentPart`.
pub fn content_block_to_part(block: &ContentBlock) -> ContentPart {
    match block {
        ContentBlock::Text { text } => ContentPart::Text { text: text.clone() },
        ContentBlock::Image { source, media_type } => ContentPart::Image {
            source: image_source_to_internal(source),
            media_type: media_type.clone(),
        },
        ContentBlock::ToolUse { id, name, input } => ContentPart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentPart::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ContentBlock::Custom { content_type, data } => ContentPart::Text {
            text: format!(
                "[custom:{}] {}",
                content_type,
                serde_json::to_string(data).unwrap_or_default()
            ),
        },
        // Handle non_exhaustive future variants
        _ => ContentPart::Text {
            text: "[unknown content block]".into(),
        },
    }
}

/// Convert an internal `ContentPart` to a layer0 `ContentBlock`.
pub fn content_part_to_block(part: &ContentPart) -> ContentBlock {
    match part {
        ContentPart::Text { text } => ContentBlock::Text { text: text.clone() },
        ContentPart::Image { source, media_type } => ContentBlock::Image {
            source: image_source_to_layer0(source),
            media_type: media_type.clone(),
        },
        ContentPart::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
    }
}

/// Convert layer0 `Content` to a list of internal `ContentPart`s.
pub fn content_to_parts(content: &Content) -> Vec<ContentPart> {
    match content {
        Content::Text(text) => vec![ContentPart::Text { text: text.clone() }],
        Content::Blocks(blocks) => blocks.iter().map(content_block_to_part).collect(),
        // Handle non_exhaustive
        _ => vec![ContentPart::Text {
            text: "[unknown content]".into(),
        }],
    }
}

/// Convert internal `ContentPart`s to a layer0 `Content`.
pub fn parts_to_content(parts: &[ContentPart]) -> Content {
    if parts.len() == 1
        && let ContentPart::Text { text } = &parts[0]
    {
        return Content::Text(text.clone());
    }
    Content::Blocks(parts.iter().map(content_part_to_block).collect())
}

fn image_source_to_internal(source: &layer0::content::ImageSource) -> ImageSource {
    match source {
        layer0::content::ImageSource::Base64 { data } => ImageSource::Base64 { data: data.clone() },
        layer0::content::ImageSource::Url { url } => ImageSource::Url { url: url.clone() },
        // Handle non_exhaustive
        _ => ImageSource::Url { url: String::new() },
    }
}

fn image_source_to_layer0(source: &ImageSource) -> layer0::content::ImageSource {
    match source {
        ImageSource::Base64 { data } => layer0::content::ImageSource::Base64 { data: data.clone() },
        ImageSource::Url { url } => layer0::content::ImageSource::Url { url: url.clone() },
    }
}

/// Convert a `ProviderMessage` to a layer0 `Message` with default metadata.
impl From<ProviderMessage> for Message {
    fn from(pm: ProviderMessage) -> Self {
        Message::new(role_to_layer0(&pm.role), parts_to_content(&pm.content))
    }
}

/// Convert a layer0 `Message` to a `ProviderMessage`.
///
/// Metadata (policy, salience, source) is discarded — the provider wire
/// format does not carry it.
impl From<Message> for ProviderMessage {
    fn from(msg: Message) -> Self {
        ProviderMessage {
            role: role_from_layer0(&msg.role),
            content: content_to_parts(&msg.content),
        }
    }
}

/// Convert a `&Message` to a `ProviderMessage` (cloning).
pub fn message_to_provider(msg: &Message) -> ProviderMessage {
    ProviderMessage {
        role: role_from_layer0(&msg.role),
        content: content_to_parts(&msg.content),
    }
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
