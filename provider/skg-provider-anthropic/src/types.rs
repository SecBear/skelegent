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
    /// Optional system prompt (plain string or content blocks for prompt caching).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<AnthropicSystemContent>,
    /// Tools available to the model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AnthropicTool>,
    /// Whether to stream the response via SSE.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    /// Sampling temperature (must be None when thinking is enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Extended thinking configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>,
    /// Tool choice constraint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
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
    /// Thinking/reasoning block.
    #[serde(rename = "thinking")]
    Thinking {
        /// The thinking text.
        thinking: String,
        /// Opaque signature for multi-turn forwarding.
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
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

/// System prompt: a plain string or an array of content blocks (for prompt caching).
///
/// When `cache_control` is needed, the system prompt must be serialised as an array
/// of content blocks. The `#[serde(untagged)]` attribute handles both wire shapes.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AnthropicSystemContent {
    /// Plain text — no cache control.
    Text(String),
    /// One or more content blocks, each with optional `cache_control`.
    Blocks(Vec<AnthropicSystemBlock>),
}

/// A single block in the `system` array used when prompt caching is required.
#[derive(Debug, Serialize)]
pub struct AnthropicSystemBlock {
    /// Block type — always `"text"` for system prompts.
    #[serde(rename = "type")]
    pub block_type: &'static str,
    /// Text content.
    pub text: String,
    /// Cache control directive, e.g. `{"type": "ephemeral"}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,
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
    /// Optional cache control for prompt caching (e.g. `{"type": "ephemeral"}`).
    ///
    /// Populated from [`ToolSchema::extra`]`["cache_control"]` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,
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

// ── Streaming SSE types ─────────────────────────────────────────────────

/// Top-level SSE event wrapper from the Anthropic streaming API.
///
/// Each SSE frame has an `event:` type and a JSON `data:` payload.
/// We parse the `data:` JSON into the appropriate variant.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEventData {
    /// `message_start` — first event, contains the outer message shell.
    #[serde(rename = "message_start")]
    MessageStart {
        /// The initial message skeleton (content is empty at this point).
        message: AnthropicStreamMessage,
    },
    /// `content_block_start` — a new content block (text or tool_use) begins.
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        /// Zero-based index of the block.
        index: usize,
        /// The block shell.
        content_block: AnthropicContentBlock,
    },
    /// `content_block_delta` — incremental data for an open block.
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        /// Zero-based index of the block.
        index: usize,
        /// The delta payload.
        delta: StreamDelta,
    },
    /// `content_block_stop` — the block at `index` is complete.
    #[serde(rename = "content_block_stop")]
    ContentBlockStop {
        /// Zero-based index of the block.
        index: usize,
    },
    /// `message_delta` — message-level updates (stop_reason, usage).
    #[serde(rename = "message_delta")]
    MessageDelta {
        /// Updated fields.
        delta: MessageDeltaPayload,
        /// Updated usage (output tokens so far).
        #[serde(default)]
        usage: Option<StreamUsage>,
    },
    /// `message_stop` — the message is fully complete.
    #[serde(rename = "message_stop")]
    MessageStop,
    /// `ping` — keepalive.
    #[serde(rename = "ping")]
    Ping,
    /// `error` — streaming error.
    #[serde(rename = "error")]
    Error {
        /// Error details.
        error: StreamError,
    },
}

/// Incremental delta within a content block.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)] // All variants are deltas by design
pub enum StreamDelta {
    /// Text chunk.
    #[serde(rename = "text_delta")]
    TextDelta {
        /// The text fragment.
        text: String,
    },
    /// Partial JSON for a tool call's `input`.
    #[serde(rename = "input_json_delta")]
    InputJsonDelta {
        /// JSON fragment to append.
        partial_json: String,
    },
    /// Thinking text chunk.
    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        /// The thinking text fragment.
        thinking: String,
    },
    /// Signature chunk for a thinking block.
    #[serde(rename = "signature_delta")]
    SignatureDelta {
        /// The signature fragment.
        signature: String,
    },
}

/// Message shell from `message_start`.
#[derive(Debug, Deserialize)]
pub struct AnthropicStreamMessage {
    /// Model identifier.
    pub model: String,
    /// Token usage at start (input_tokens populated, output 0).
    pub usage: AnthropicUsage,
}

/// Payload of `message_delta`.
#[derive(Debug, Deserialize)]
pub struct MessageDeltaPayload {
    /// Stop reason (set on final delta).
    #[serde(default)]
    pub stop_reason: Option<String>,
}

/// Usage fragment from `message_delta`.
#[derive(Debug, Deserialize)]
pub struct StreamUsage {
    /// Output tokens generated so far.
    pub output_tokens: u64,
}

/// Error from the streaming API.
#[derive(Debug, Deserialize)]
pub struct StreamError {
    /// Error type.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable message.
    pub message: String,
}
