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
    /// Optional provider-specific per-tool metadata.
    ///
    /// Providers that support per-tool configuration (e.g., Anthropic `cache_control`,
    /// OpenAI `strict`) can read this field during schema conversion.
    /// Providers that don't need it ignore it.
    ///
    /// Defaults to `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl ToolSchema {
    /// Create a new [`ToolSchema`] with the given name, description, and input schema.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            extra: None,
        }
    }

    /// Attach provider-specific per-tool metadata.
    ///
    /// The value is passed through to the provider during schema conversion.
    /// Providers that don't support per-tool metadata ignore this field.
    pub fn with_extra(mut self, extra: serde_json::Value) -> Self {
        self.extra = Some(extra);
        self
    }
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

/// Controls which tools the model can use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    /// Model decides whether to use tools.
    Auto,
    /// Model must use at least one tool.
    Any,
    /// Model must use this specific tool.
    Tool {
        /// Name of the tool the model must call.
        name: String,
    },
    /// Model must not use tools (or tool calls are suppressed).
    None,
}

/// Requested response format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Plain text (default).
    Text,
    /// JSON object (model constrained to valid JSON).
    Json,
    /// JSON conforming to a specific schema (OpenAI structured output).
    JsonSchema {
        /// Schema name identifier.
        name: String,
        /// The JSON schema.
        schema: serde_json::Value,
        /// Whether to enforce strict schema compliance (OpenAI-specific).
        strict: bool,
    },
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

    #[test]
    fn tool_schema_extra_default_none() {
        let schema = ToolSchema::new("my_tool", "does a thing", serde_json::json!({}));
        assert!(schema.extra.is_none());
    }

    #[test]
    fn tool_schema_with_extra() {
        let extra = serde_json::json!({ "cache_control": { "type": "ephemeral" } });
        let schema = ToolSchema::new("my_tool", "does a thing", serde_json::json!({}))
            .with_extra(extra.clone());
        assert_eq!(schema.extra.as_ref().unwrap(), &extra);
    }

    #[test]
    fn tool_schema_extra_serde_round_trip() {
        let extra = serde_json::json!({ "strict": true });
        let schema =
            ToolSchema::new("my_tool", "does a thing", serde_json::json!({})).with_extra(extra);
        let json = serde_json::to_value(&schema).unwrap();
        let back: ToolSchema = serde_json::from_value(json).unwrap();
        assert_eq!(back.extra.unwrap(), serde_json::json!({ "strict": true }));
    }
}
