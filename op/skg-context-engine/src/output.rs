//! Structured output — validated, typed responses from inference.
//!
//! [`OutputSchema`] defines what the model should return and how to validate it.
//! [`OutputSchema::extract()`] checks an inference response against the schema.
//! [`react_loop_structured()`](crate::runtime::react_loop_structured) composes
//! this with the ReAct loop for automatic retry on validation failure.
//!
//! ## Two modes
//!
//! - [`OutputMode::ToolCall`] — the model calls a `return_result` tool with
//!   structured input. This is the most reliable mode: providers constrain
//!   the model's output to valid JSON matching the tool schema.
//!
//! - [`OutputMode::TextJson`] — the model returns JSON in its text response.
//!   Useful when the provider doesn't support tool-based structured output
//!   or when the output is simpler.
//!
//! ## Construction
//!
//! ```rust,ignore
//! // From a raw JSON Schema + validator closure:
//! let schema = OutputSchema::tool_call(json_schema, |v| {
//!     if v.get("name").is_none() { Err("missing name".into()) }
//!     else { Ok(v.clone()) }
//! });
//!
//! // From a Rust type (requires `typed-output` feature):
//! let schema = OutputSchema::from_type::<CityInfo>();
//! ```

use serde_json::Value;
use skg_turn::infer::InferResponse;
use skg_turn::types::ToolSchema;
use std::fmt;

/// The canonical tool name for returning structured results.
///
/// Matches `skg_turn_kit::RETURN_RESULT_TOOL` by convention.
pub const RETURN_RESULT_TOOL: &str = "return_result";

/// How the model should return structured output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Model calls a `return_result` tool with structured input.
    ///
    /// This is the most reliable mode. The tool's input schema constrains
    /// the model's output, and providers like Anthropic and OpenAI enforce
    /// schema conformance at the API level.
    ToolCall,

    /// Model returns JSON in its text response.
    ///
    /// The response text is parsed as JSON (with fallback to markdown
    /// code-fence extraction). Useful when tool-based output isn't
    /// available or when the output format is simple.
    TextJson,
}

/// Why structured output extraction failed.
#[derive(Debug)]
pub enum OutputError {
    /// The model didn't produce any output matching the expected mode.
    ///
    /// In `ToolCall` mode: no `return_result` tool call was present.
    /// In `TextJson` mode: no text content was present.
    NoOutput,

    /// The model produced output but it failed validation.
    ValidationFailed {
        /// Human-readable validation error message.
        message: String,
        /// The raw value that failed validation (for diagnostics).
        raw: Option<Value>,
    },
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputError::NoOutput => write!(f, "model did not produce structured output"),
            OutputError::ValidationFailed { message, .. } => {
                write!(f, "output validation failed: {message}")
            }
        }
    }
}

impl std::error::Error for OutputError {}

/// Type-erased output schema for structured responses.
///
/// Defines what the model should return (JSON Schema), how it returns it
/// ([`OutputMode`]), and how to validate it (validator closure). Constructed
/// via [`OutputSchema::tool_call()`] or [`OutputSchema::text_json()`].
///
/// With the `typed-output` feature, use [`OutputSchema::from_type::<T>()`]
/// to derive the schema and validator from a Rust type automatically.
pub struct OutputSchema {
    /// JSON Schema describing the expected output structure.
    pub schema: Value,
    /// How the model returns the output.
    pub mode: OutputMode,
    /// Maximum validation retries before giving up.
    pub max_retries: u32,
    /// Tool name used in `ToolCall` mode.
    pub tool_name: String,
    /// Tool description used in `ToolCall` mode.
    pub tool_description: String,
    /// Validator: takes raw JSON, returns `Ok(validated)` or `Err(message)`.
    #[allow(clippy::type_complexity)]
    validator: Box<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>,
}

impl fmt::Debug for OutputSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputSchema")
            .field("mode", &self.mode)
            .field("max_retries", &self.max_retries)
            .field("tool_name", &self.tool_name)
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl OutputSchema {
    /// Create an output schema in tool-call mode.
    ///
    /// The model will be given a `return_result` tool whose input schema
    /// wraps the provided `schema`. When the model calls this tool, the
    /// `validator` checks the `result` field.
    ///
    /// # Arguments
    ///
    /// * `schema` — JSON Schema for the expected output type.
    /// * `validator` — Closure that validates a raw JSON value. Return
    ///   `Ok(value)` on success or `Err(message)` with a human-readable
    ///   error that will be sent back to the model for retry.
    pub fn tool_call(
        schema: Value,
        validator: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            schema,
            mode: OutputMode::ToolCall,
            max_retries: 3,
            tool_name: RETURN_RESULT_TOOL.to_string(),
            tool_description: "Return a structured result. Call this tool with the final answer."
                .to_string(),
            validator: Box::new(validator),
        }
    }

    /// Create an output schema in text-JSON mode.
    ///
    /// The model is expected to return JSON in its text response. The text
    /// is parsed as JSON (with fallback to markdown code-fence extraction),
    /// then validated.
    pub fn text_json(
        schema: Value,
        validator: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            schema,
            mode: OutputMode::TextJson,
            max_retries: 3,
            tool_name: RETURN_RESULT_TOOL.to_string(),
            tool_description: String::new(),
            validator: Box::new(validator),
        }
    }

    /// Set the maximum number of validation retries.
    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries = max;
        self
    }

    /// Set a custom tool name (default: `return_result`).
    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = name.into();
        self
    }

    /// Generate the [`ToolSchema`] for this output.
    ///
    /// Used by [`react_loop_structured`](crate::runtime::react_loop_structured)
    /// to inject the output tool into the compile config.
    pub fn tool_schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.tool_name.clone(),
            description: self.tool_description.clone(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "result": self.schema
                },
                "required": ["result"]
            }),
            extra: None,
        }
    }

    /// Extract and validate structured output from an inference response.
    ///
    /// Returns `Ok(validated_value)` on success. On failure, returns an
    /// [`OutputError`] that the caller can use to decide whether to retry.
    pub fn extract(&self, response: &InferResponse) -> Result<Value, OutputError> {
        match self.mode {
            OutputMode::ToolCall => self.extract_tool_call(response),
            OutputMode::TextJson => self.extract_text_json(response),
        }
    }

    /// Validate a raw JSON value against this schema's validator.
    pub fn validate(&self, value: &Value) -> Result<Value, String> {
        (self.validator)(value)
    }

    fn extract_tool_call(&self, response: &InferResponse) -> Result<Value, OutputError> {
        let call = response
            .tool_calls
            .iter()
            .find(|c| c.name == self.tool_name)
            .ok_or(OutputError::NoOutput)?;

        let result = call
            .input
            .get("result")
            .ok_or_else(|| OutputError::ValidationFailed {
                message: "missing 'result' field in tool call input".to_string(),
                raw: Some(call.input.clone()),
            })?;

        (self.validator)(result).map_err(|message| OutputError::ValidationFailed {
            message,
            raw: Some(result.clone()),
        })
    }

    fn extract_text_json(&self, response: &InferResponse) -> Result<Value, OutputError> {
        let text = response.text().ok_or(OutputError::NoOutput)?;
        if text.is_empty() {
            return Err(OutputError::NoOutput);
        }

        let json_str = extract_json_block(text).unwrap_or(text);

        let value: Value =
            serde_json::from_str(json_str).map_err(|e| OutputError::ValidationFailed {
                message: format!("JSON parse error: {e}"),
                raw: None,
            })?;

        (self.validator)(&value).map_err(|message| OutputError::ValidationFailed {
            message,
            raw: Some(value),
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Feature-gated: typed-output (schemars)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "typed-output")]
impl OutputSchema {
    /// Create an output schema from a Rust type (tool-call mode).
    ///
    /// Derives the JSON Schema from `T: JsonSchema` and validates by
    /// attempting `serde_json::from_value::<T>()`. This is the most
    /// ergonomic constructor — one line for full schema + validation.
    ///
    /// ```rust,ignore
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize, JsonSchema)]
    /// struct CityInfo {
    ///     name: String,
    ///     population: u64,
    /// }
    ///
    /// let schema = OutputSchema::from_type::<CityInfo>();
    /// ```
    pub fn from_type<T>() -> Self
    where
        T: serde::de::DeserializeOwned + schemars::JsonSchema + Send + Sync + 'static,
    {
        let schema = schemars::schema_for!(T);
        let schema_value =
            serde_json::to_value(schema).unwrap_or(Value::Object(Default::default()));

        Self::tool_call(schema_value, |value| {
            serde_json::from_value::<T>(value.clone())
                .map_err(|e| format!("validation error: {e}"))?;
            Ok(value.clone())
        })
    }

    /// Create an output schema from a Rust type (text-JSON mode).
    ///
    /// Same as [`OutputSchema::from_type()`] but expects JSON in the
    /// model's text response instead of a tool call.
    pub fn from_type_text<T>() -> Self
    where
        T: serde::de::DeserializeOwned + schemars::JsonSchema + Send + Sync + 'static,
    {
        let schema = schemars::schema_for!(T);
        let schema_value =
            serde_json::to_value(schema).unwrap_or(Value::Object(Default::default()));

        Self::text_json(schema_value, |value| {
            serde_json::from_value::<T>(value.clone())
                .map_err(|e| format!("validation error: {e}"))?;
            Ok(value.clone())
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract JSON content from a markdown code fence.
///
/// Handles both ` ```json\n...\n``` ` and ` ```\n...\n``` ` patterns.
/// Returns `None` if no code fence is found.
pub fn extract_json_block(text: &str) -> Option<&str> {
    // Try ```json first, then bare ```
    let start = text
        .find("```json\n")
        .map(|i| i + 8)
        .or_else(|| text.find("```json\r\n").map(|i| i + 9))
        .or_else(|| text.find("```\n").map(|i| i + 4))
        .or_else(|| text.find("```\r\n").map(|i| i + 5))?;

    let end = text[start..].find("```").map(|i| start + i)?;

    let content = text[start..end].trim();
    if content.is_empty() {
        return None;
    }
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use skg_turn::infer::{InferResponse, ToolCall};
    use skg_turn::types::{StopReason, TokenUsage};

    fn text_response(text: &str) -> InferResponse {
        InferResponse {
            content: layer0::content::Content::text(text),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
            },
            model: "test".to_string(),
            cost: None,
            truncated: None,
        }
    }

    fn tool_call_response(name: &str, id: &str, input: Value) -> InferResponse {
        InferResponse {
            content: layer0::content::Content::text(""),
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                input,
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
            },
            model: "test".to_string(),
            cost: None,
            truncated: None,
        }
    }

    fn city_validator(value: &Value) -> Result<Value, String> {
        if value.get("name").and_then(|n| n.as_str()).is_none() {
            return Err("missing or invalid 'name' field".to_string());
        }
        if value.get("population").and_then(|p| p.as_u64()).is_none() {
            return Err("missing or invalid 'population' field".to_string());
        }
        Ok(value.clone())
    }

    // ── OutputSchema construction ─────────────────────────

    #[test]
    fn tool_call_schema_defaults() {
        let schema = OutputSchema::tool_call(json!({"type": "object"}), |v| Ok(v.clone()));
        assert_eq!(schema.mode, OutputMode::ToolCall);
        assert_eq!(schema.max_retries, 3);
        assert_eq!(schema.tool_name, RETURN_RESULT_TOOL);
    }

    #[test]
    fn text_json_schema_defaults() {
        let schema = OutputSchema::text_json(json!({"type": "object"}), |v| Ok(v.clone()));
        assert_eq!(schema.mode, OutputMode::TextJson);
        assert_eq!(schema.max_retries, 3);
    }

    #[test]
    fn builder_methods() {
        let schema = OutputSchema::tool_call(json!({}), |v| Ok(v.clone()))
            .with_max_retries(5)
            .with_tool_name("custom_result");
        assert_eq!(schema.max_retries, 5);
        assert_eq!(schema.tool_name, "custom_result");
    }

    #[test]
    fn tool_schema_generation() {
        let schema = OutputSchema::tool_call(
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } }
            }),
            |v| Ok(v.clone()),
        );
        let ts = schema.tool_schema();
        assert_eq!(ts.name, "return_result");
        assert!(ts.description.contains("structured result"));
        assert_eq!(ts.input_schema["type"], "object");
        assert!(ts.input_schema["properties"]["result"].is_object());
        assert!(
            ts.input_schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("result"))
        );
    }

    // ── ToolCall mode extraction ──────────────────────────

    #[test]
    fn extract_tool_call_success() {
        let schema = OutputSchema::tool_call(json!({}), city_validator);
        let response = tool_call_response(
            "return_result",
            "call_1",
            json!({
                "result": {
                    "name": "Tokyo",
                    "population": 13960000_u64
                }
            }),
        );
        let value = schema.extract(&response).unwrap();
        assert_eq!(value["name"], "Tokyo");
        assert_eq!(value["population"], 13960000_u64);
    }

    #[test]
    fn extract_tool_call_validation_failure() {
        let schema = OutputSchema::tool_call(json!({}), city_validator);
        let response = tool_call_response(
            "return_result",
            "call_1",
            json!({
                "result": { "name": "Tokyo" }
            }),
        );
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::ValidationFailed { .. }));
        assert!(err.to_string().contains("population"));
    }

    #[test]
    fn extract_tool_call_missing_result_field() {
        let schema = OutputSchema::tool_call(json!({}), city_validator);
        let response = tool_call_response(
            "return_result",
            "call_1",
            json!({ "name": "Tokyo", "population": 13960000_u64 }),
        );
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::ValidationFailed { .. }));
        assert!(err.to_string().contains("result"));
    }

    #[test]
    fn extract_tool_call_wrong_tool_name() {
        let schema = OutputSchema::tool_call(json!({}), city_validator);
        let response = tool_call_response("some_other_tool", "call_1", json!({}));
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::NoOutput));
    }

    #[test]
    fn extract_tool_call_no_tool_calls() {
        let schema = OutputSchema::tool_call(json!({}), city_validator);
        let response = text_response("The capital is Tokyo.");
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::NoOutput));
    }

    #[test]
    fn extract_tool_call_custom_tool_name() {
        let schema = OutputSchema::tool_call(json!({}), |v| Ok(v.clone())).with_tool_name("submit");
        let response = tool_call_response("submit", "call_1", json!({ "result": "ok" }));
        assert!(schema.extract(&response).is_ok());
    }

    // ── TextJson mode extraction ──────────────────────────

    #[test]
    fn extract_text_json_success() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response(r#"{"name": "Tokyo", "population": 13960000}"#);
        let value = schema.extract(&response).unwrap();
        assert_eq!(value["name"], "Tokyo");
    }

    #[test]
    fn extract_text_json_code_fence() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response(
            "Here is the result:\n```json\n{\"name\": \"Tokyo\", \"population\": 13960000}\n```\nDone.",
        );
        let value = schema.extract(&response).unwrap();
        assert_eq!(value["name"], "Tokyo");
    }

    #[test]
    fn extract_text_json_bare_code_fence() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response("```\n{\"name\": \"Berlin\", \"population\": 3645000}\n```");
        let value = schema.extract(&response).unwrap();
        assert_eq!(value["name"], "Berlin");
    }

    #[test]
    fn extract_text_json_validation_failure() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response(r#"{"name": "Tokyo"}"#);
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::ValidationFailed { .. }));
        assert!(err.to_string().contains("population"));
    }

    #[test]
    fn extract_text_json_not_json() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response("The capital of Japan is Tokyo.");
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::ValidationFailed { .. }));
        assert!(err.to_string().contains("JSON parse error"));
    }

    #[test]
    fn extract_text_json_no_text() {
        let schema = OutputSchema::text_json(json!({}), city_validator);
        let response = text_response("");
        let err = schema.extract(&response).unwrap_err();
        assert!(matches!(err, OutputError::NoOutput));
    }

    // ── extract_json_block helper ─────────────────────────

    #[test]
    fn extract_json_block_with_json_tag() {
        let text = "text\n```json\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json_block(text), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_json_block_bare() {
        let text = "text\n```\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json_block(text), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_json_block_none() {
        assert_eq!(extract_json_block("no code fence here"), None);
    }

    #[test]
    fn extract_json_block_empty() {
        let text = "```json\n```";
        assert_eq!(extract_json_block(text), None);
    }

    #[test]
    fn extract_json_block_trims_whitespace() {
        let text = "```json\n  {\"a\": 1}  \n```";
        assert_eq!(extract_json_block(text), Some("{\"a\": 1}"));
    }

    // ── Debug impl ────────────────────────────────────────

    #[test]
    fn debug_impl_works() {
        let schema = OutputSchema::tool_call(json!({"type": "object"}), |v| Ok(v.clone()));
        let debug = format!("{schema:?}");
        assert!(debug.contains("OutputSchema"));
        assert!(debug.contains("ToolCall"));
    }

    // ── OutputError display ───────────────────────────────

    #[test]
    fn output_error_display() {
        assert_eq!(
            OutputError::NoOutput.to_string(),
            "model did not produce structured output"
        );
        let err = OutputError::ValidationFailed {
            message: "bad field".into(),
            raw: None,
        };
        assert_eq!(err.to_string(), "output validation failed: bad field");
    }

    // ── Feature-gated typed-output ────────────────────────

    #[cfg(feature = "typed-output")]
    mod typed_output_tests {
        use super::*;
        use schemars::JsonSchema;
        use serde::Deserialize;

        #[derive(Debug, Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct CityInfo {
            name: String,
            population: u64,
        }

        #[test]
        fn from_type_tool_call_mode() {
            let schema = OutputSchema::from_type::<CityInfo>();
            assert_eq!(schema.mode, OutputMode::ToolCall);

            let response = tool_call_response(
                "return_result",
                "call_1",
                json!({
                    "result": {
                        "name": "Tokyo",
                        "population": 13960000_u64
                    }
                }),
            );
            let value = schema.extract(&response).unwrap();
            assert_eq!(value["name"], "Tokyo");
        }

        #[test]
        fn from_type_validation_failure() {
            let schema = OutputSchema::from_type::<CityInfo>();
            let response = tool_call_response(
                "return_result",
                "call_1",
                json!({
                    "result": { "name": "Tokyo" }
                }),
            );
            let err = schema.extract(&response).unwrap_err();
            assert!(matches!(err, OutputError::ValidationFailed { .. }));
        }

        #[test]
        fn from_type_text_mode() {
            let schema = OutputSchema::from_type_text::<CityInfo>();
            assert_eq!(schema.mode, OutputMode::TextJson);

            let response = text_response(r#"{"name": "Tokyo", "population": 13960000}"#);
            let value = schema.extract(&response).unwrap();
            assert_eq!(value["name"], "Tokyo");
        }
    }
}
