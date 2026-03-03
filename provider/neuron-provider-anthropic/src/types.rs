//! Anthropic Messages API request/response types.

use serde::{Deserialize, Serialize};

/// Anthropic API request body.
#[derive(Debug, Serialize)]
pub struct AnthropicRequest {
    /// Model identifier.
    pub model: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Conversation messages.
    pub messages: Vec<AnthropicMessage>,
    /// Optional system prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Tools available to the model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AnthropicTool>,
}

/// A message in the Anthropic API format.
#[derive(Debug, Serialize, Deserialize)]
pub struct AnthropicMessage {
    /// Role: "user" or "assistant".
    pub role: String,
    /// Message content.
    pub content: AnthropicContent,
}

/// Content can be a string or array of content blocks.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    /// Simple text string.
    Text(String),
    /// Array of content blocks.
    Blocks(Vec<AnthropicContentBlock>),
}

/// A content block in the Anthropic API format.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    /// Text content.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// Tool use request.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Tool use identifier.
        id: String,
        /// Tool name.
        name: String,
        /// Tool input parameters.
        input: serde_json::Value,
    },
    /// Tool result.
    #[serde(rename = "tool_result")]
    ToolResult {
        /// The tool use ID this result is for.
        tool_use_id: String,
        /// The result content.
        content: String,
        /// Whether this result represents an error.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    /// Image content.
    #[serde(rename = "image")]
    Image {
        /// Image source.
        source: AnthropicImageSource,
        /// MIME type.
        media_type: String,
    },
}

/// Image source in Anthropic API format.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicImageSource {
    /// Base64-encoded image.
    #[serde(rename = "base64")]
    Base64 {
        /// Base64 data.
        data: String,
    },
    /// URL-referenced image.
    #[serde(rename = "url")]
    Url {
        /// Image URL.
        url: String,
    },
}

/// Tool definition for the Anthropic API.
#[derive(Debug, Serialize)]
pub struct AnthropicTool {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for the tool input.
    pub input_schema: serde_json::Value,
}

/// Anthropic API response body.
#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    /// Response content blocks.
    pub content: Vec<AnthropicContentBlock>,
    /// Model that generated the response.
    pub model: String,
    /// Stop reason.
    pub stop_reason: String,
    /// Token usage.
    pub usage: AnthropicUsage,
}

/// Token usage from the Anthropic API.
#[derive(Debug, Deserialize)]
pub struct AnthropicUsage {
    /// Input tokens used.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Cache read tokens (prompt caching).
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    /// Cache creation tokens (prompt caching).
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
}
