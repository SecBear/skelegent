//! Wire types for provider boundaries.
//!
//! Only types that cross the operator→provider boundary and have no
//! layer0 equivalent live here. All message types use `layer0::context::Message`
//! directly via `InferRequest` / `InferResponse`.

use serde::{Deserialize, Serialize};

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

/// Why the provider stopped generating.
#[non_exhaustive]
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
    /// Reasoning tokens consumed (if the model supports extended thinking).
    /// Only populated by providers that report this (e.g., OpenAI o-series).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            reasoning_tokens: Some(42),
        };
        let json = serde_json::to_value(&usage).unwrap();
        let back: TokenUsage = serde_json::from_value(json).unwrap();
        assert_eq!(usage, back);
    }
}
