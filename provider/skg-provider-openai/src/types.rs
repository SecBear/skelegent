//! OpenAI Chat Completions API request/response types.

use serde::{Deserialize, Serialize};

/// OpenAI Chat Completions API request body.
#[derive(Debug, Serialize)]
pub struct OpenAIRequest {
    /// Model identifier (e.g. "gpt-4o-mini").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<OpenAIMessage>,
    /// Maximum tokens to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Tools available to the model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAITool>,
    /// Whether the model may issue multiple tool calls in parallel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Service tier for the request (e.g. "auto", "default").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Reasoning effort level (e.g. "low", "medium", "high").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Whether to stream the response.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    /// Stream options (e.g. include_usage).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<serde_json::Value>,
}

/// A message in the OpenAI Chat Completions API format.
#[derive(Debug, Serialize, Deserialize)]
pub struct OpenAIMessage {
    /// Role: "system", "user", "assistant", "developer", or "tool".
    pub role: String,
    /// Message content (string or array of content parts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAIContent>,
    /// Tool calls requested by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIToolCall>>,
    /// The tool_call_id this message is a response to (role="tool" only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Content can be a plain string or an array of content parts.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenAIContent {
    /// Simple text string.
    Text(String),
    /// Array of content parts (text, image_url, etc.).
    Parts(Vec<OpenAIContentPart>),
}

/// A single content part within a message's content array.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OpenAIContentPart {
    /// Text content part.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// Image URL content part.
    #[serde(rename = "image_url")]
    ImageUrl {
        /// The image URL object.
        image_url: OpenAIImageUrl,
    },
}

/// Image URL reference in OpenAI API format.
#[derive(Debug, Serialize, Deserialize)]
pub struct OpenAIImageUrl {
    /// The URL of the image (can be a data: URI for base64).
    pub url: String,
}

/// A tool call requested by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// The type of tool call (always "function").
    #[serde(rename = "type")]
    pub call_type: String,
    /// The function to call.
    pub function: OpenAIFunctionCall,
}

/// A function call within a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIFunctionCall {
    /// Name of the function to call.
    pub name: String,
    /// Arguments as a JSON string (must be parsed by the consumer).
    pub arguments: String,
}

/// Tool definition for the OpenAI API.
#[derive(Debug, Serialize)]
pub struct OpenAITool {
    /// The type of tool (always "function").
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function definition.
    pub function: OpenAIFunction,
}

/// Function definition within a tool.
#[derive(Debug, Serialize)]
pub struct OpenAIFunction {
    /// Function name.
    pub name: String,
    /// Function description.
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}

/// OpenAI Chat Completions API response body.
#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    /// Unique identifier for the completion.
    #[allow(dead_code)]
    pub id: String,
    /// Response choices.
    pub choices: Vec<OpenAIChoice>,
    /// Model that generated the response.
    pub model: String,
    /// Token usage statistics.
    pub usage: OpenAIUsage,
    /// Service tier used for the request.
    #[serde(default)]
    #[allow(dead_code)]
    pub service_tier: Option<String>,
}

/// A single choice in the response.
#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    /// The generated message.
    pub message: OpenAIMessage,
    /// Why generation stopped.
    pub finish_reason: String,
    /// Index of this choice.
    #[allow(dead_code)]
    pub index: u32,
}

/// Token usage statistics from the OpenAI API.
#[derive(Debug, Deserialize)]
pub struct OpenAIUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u64,
    /// Number of tokens in the completion.
    pub completion_tokens: u64,
    /// Total tokens used (prompt + completion).
    #[allow(dead_code)]
    pub total_tokens: u64,
    /// Detailed breakdown of prompt token usage.
    #[serde(default)]
    pub prompt_tokens_details: Option<OpenAIPromptTokensDetails>,
    /// Detailed breakdown of completion token usage.
    #[serde(default)]
    pub completion_tokens_details: Option<OpenAICompletionTokensDetails>,
}

/// Detailed breakdown of prompt token usage.
#[derive(Debug, Deserialize)]
pub struct OpenAIPromptTokensDetails {
    /// Number of cached tokens used.
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

/// Detailed breakdown of completion token usage.
#[derive(Debug, Deserialize)]
pub struct OpenAICompletionTokensDetails {
    /// Number of reasoning tokens used.
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

/// A streaming chunk from OpenAI's Chat Completions API.
#[derive(Debug, Deserialize)]
pub struct OpenAIStreamChunk {
    /// Unique identifier for the chunk.
    #[allow(dead_code)]
    pub id: String,
    /// Choices within this chunk.
    pub choices: Vec<OpenAIStreamChoice>,
    /// Usage statistics (present in the final chunk when stream_options.include_usage is set).
    #[serde(default)]
    pub usage: Option<OpenAIUsage>,
    /// Model that generated the chunk.
    pub model: String,
}

/// A choice within a streaming chunk.
#[derive(Debug, Deserialize)]
pub struct OpenAIStreamChoice {
    /// The delta payload.
    pub delta: OpenAIStreamDelta,
    /// Why generation stopped (present in the final choice).
    #[serde(default)]
    pub finish_reason: Option<String>,
    /// Index of this choice.
    #[allow(dead_code)]
    pub index: u32,
}

/// The delta payload within a streaming choice.
#[derive(Debug, Deserialize)]
pub struct OpenAIStreamDelta {
    /// Role of the message author (present in the first chunk).
    #[serde(default)]
    #[allow(dead_code)]
    pub role: Option<String>,
    /// Text content delta.
    #[serde(default)]
    pub content: Option<String>,
    /// Tool call deltas.
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIStreamToolCall>>,
}

/// A tool call delta in streaming.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIStreamToolCall {
    /// Index of the tool call being accumulated.
    pub index: u32,
    /// Tool call ID (present in the first delta for this index).
    #[serde(default)]
    pub id: Option<String>,
    /// Function delta.
    #[serde(default)]
    pub function: Option<OpenAIStreamFunction>,
}

/// Function delta in a streaming tool call.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIStreamFunction {
    /// Function name (present in the first delta for this tool call).
    #[serde(default)]
    pub name: Option<String>,
    /// Partial JSON arguments.
    #[serde(default)]
    pub arguments: Option<String>,
}
