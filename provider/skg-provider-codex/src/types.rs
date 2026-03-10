//! OpenAI Responses API wire types for Codex.

use serde::{Deserialize, Serialize};

// ── Request types ────────────────────────────────────────────────────────────

/// Codex Responses API request body.
#[derive(Debug, Serialize)]
pub struct CodexRequest {
    /// Model identifier (e.g. "gpt-5", "gpt-5-codex").
    pub model: String,
    /// Input items (flat list, not messages).
    pub input: Vec<serde_json::Value>,
    /// Always true for streaming.
    pub stream: bool,
    /// System instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Available tools.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<CodexTool>,
    /// Tool choice constraint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    /// Sampling temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Maximum output tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Reasoning configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    /// Prompt cache key (session ID for caching).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// Whether to store the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
}

/// Tool definition for the Responses API.
#[derive(Debug, Serialize)]
pub struct CodexTool {
    /// Always "function".
    #[serde(rename = "type")]
    pub tool_type: String,
    /// Function name.
    pub name: String,
    /// Function description.
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters: serde_json::Value,
}

/// Reasoning effort configuration.
#[derive(Debug, Clone, Serialize)]
pub struct ReasoningConfig {
    /// Reasoning effort level.
    pub effort: String,
    /// Summary style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ── SSE Event types ──────────────────────────────────────────────────────────

/// A raw SSE event from the Codex streaming response.
///
/// We parse each `data:` line as a generic JSON object and dispatch
/// on the `type` field rather than using a tagged enum, because the
/// Responses API has many event types with varying shapes.
#[derive(Debug, Deserialize)]
pub struct SseEvent {
    /// Event type string.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The full raw JSON payload (we extract fields as needed).
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// Usage information from `response.completed`.
#[derive(Debug, Default)]
pub struct ResponseUsage {
    /// Total input tokens.
    pub input_tokens: u64,
    /// Total output tokens.
    pub output_tokens: u64,
    /// Cached input tokens.
    pub cached_tokens: u64,
}

impl ResponseUsage {
    /// Parse from the `response.usage` field in a completed event.
    pub fn from_value(v: &serde_json::Value) -> Self {
        let input = v.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output = v.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let cached = v
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Self {
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
        }
    }
}
