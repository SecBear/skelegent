//! Ollama `/api/chat` request/response types.
//!
//! Key differences from OpenAI-compatible APIs:
//! - Endpoint is POST `/api/chat` (not `/v1/chat/completions`)
//! - No auth headers required
//! - Tool call arguments are JSON objects (not strings)
//! - No tool_use IDs from Ollama -- the provider must synthesize UUIDs
//! - Response includes nanosecond timing fields

use serde::{Deserialize, Serialize};

/// Ollama `/api/chat` request body.
#[derive(Debug, Serialize)]
pub struct OllamaRequest {
    /// Model identifier (e.g. "llama3.2:1b").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<OllamaMessage>,
    /// Whether to stream the response. Always `false` for this provider.
    pub stream: bool,
    /// Tools available to the model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OllamaTool>,
    /// How long to keep the model loaded in memory (e.g. "5m", "0").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,
    /// Hardware tuning and generation options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
}

/// A message in the Ollama `/api/chat` format.
#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaMessage {
    /// Role: "system", "user", "assistant", or "tool".
    pub role: String,
    /// Message text content.
    pub content: String,
    /// Tool calls requested by the assistant (present only in assistant messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
}

/// A tool call in the Ollama response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolCall {
    /// The function being called.
    pub function: OllamaFunctionCall,
}

/// A function call within a tool call.
///
/// Unlike OpenAI, Ollama returns `arguments` as a JSON object, not a string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaFunctionCall {
    /// Name of the function to call.
    pub name: String,
    /// Arguments as a JSON object (NOT a string like OpenAI).
    pub arguments: serde_json::Value,
}

/// Tool definition for the Ollama API.
#[derive(Debug, Serialize)]
pub struct OllamaTool {
    /// The type of tool (always "function").
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function definition.
    pub function: OllamaFunction,
}

/// Function definition within a tool.
#[derive(Debug, Serialize)]
pub struct OllamaFunction {
    /// Function name.
    pub name: String,
    /// Function description.
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}

/// Hardware tuning and generation options for Ollama.
#[derive(Debug, Default, Serialize)]
pub struct OllamaOptions {
    /// Sampling temperature (0.0 - 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Maximum number of tokens to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    /// Number of tokens to keep from the prompt (context window).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
    /// Top-p (nucleus sampling).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    /// Top-k sampling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Random seed for reproducibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

/// Ollama `/api/chat` response body.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OllamaResponse {
    /// Model that generated the response.
    pub model: String,
    /// The assistant's response message.
    pub message: OllamaMessage,
    /// Whether the response is complete.
    #[serde(default)]
    pub done: bool,
    /// Why generation stopped (e.g. "stop").
    #[serde(default)]
    pub done_reason: Option<String>,
    /// Total time spent generating the response in nanoseconds.
    #[serde(default)]
    pub total_duration: Option<u64>,
    /// Time spent loading the model in nanoseconds.
    #[serde(default)]
    pub load_duration: Option<u64>,
    /// Number of tokens in the prompt.
    #[serde(default)]
    pub prompt_eval_count: Option<u64>,
    /// Time spent evaluating the prompt in nanoseconds.
    #[serde(default)]
    pub prompt_eval_duration: Option<u64>,
    /// Number of tokens generated.
    #[serde(default)]
    pub eval_count: Option<u64>,
    /// Time spent generating the response in nanoseconds.
    #[serde(default)]
    pub eval_duration: Option<u64>,
}
