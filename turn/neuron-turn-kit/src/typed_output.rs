//! Typed output support for structured agent responses.
//!
//! When attached to an operator, `TypedOutput<T>` injects a `return_result` tool
//! whose schema is derived from `T: JsonSchema`. When the model calls this tool,
//! the operator validates the output with serde and either returns the parsed value
//! or sends a validation error back for retry.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::marker::PhantomData;

/// A typed output specification that constrains agent output to type `T`.
///
/// # Usage
///
/// ```rust,ignore
/// use neuron_turn_kit::TypedOutput;
/// use schemars::JsonSchema;
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize, JsonSchema)]
/// struct CityInfo {
///     name: String,
///     population: u64,
///     country: String,
/// }
///
/// let typed = TypedOutput::<CityInfo>::new();
/// let schema = typed.tool_schema();
/// ```
pub struct TypedOutput<T: DeserializeOwned + JsonSchema + Send + Sync + 'static> {
    _marker: PhantomData<T>,
    max_retries: u32,
}

impl<T: DeserializeOwned + JsonSchema + Send + Sync + 'static> TypedOutput<T> {
    /// Create a new typed output with default settings (3 retries).
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
            max_retries: 3,
        }
    }

    /// Set the maximum number of validation retries.
    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries = max;
        self
    }

    /// Maximum validation retries before giving up.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Generate the JSON Schema for the `return_result` tool's input.
    pub fn json_schema(&self) -> Value {
        let schema = schemars::schema_for!(T);
        serde_json::to_value(schema).unwrap_or(Value::Object(Default::default()))
    }

    /// Generate a complete tool schema entry for the `return_result` tool.
    pub fn tool_schema(&self) -> ToolSchemaEntry {
        let result_schema = self.json_schema();
        ToolSchemaEntry {
            name: RETURN_RESULT_TOOL.to_string(),
            description: "Return a structured result. Call this tool with the final answer."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "result": result_schema
                },
                "required": ["result"]
            }),
        }
    }

    /// Type-erase into a `Box<dyn OutputValidator>` for storage in operators.
    pub fn into_validator(self) -> Box<dyn OutputValidator> {
        Box::new(OutputValidatorImpl::<T> {
            _marker: PhantomData,
            max_retries: self.max_retries,
            schema_entry: self.tool_schema(),
        })
    }
}

impl<T: DeserializeOwned + JsonSchema + Send + Sync + 'static> Default for TypedOutput<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The canonical tool name for returning structured results.
pub const RETURN_RESULT_TOOL: &str = "return_result";

/// A tool schema entry (name + description + input_schema).
///
/// Mirrors the structure of `neuron_turn::types::ToolSchema` without
/// depending on that crate directly.
#[derive(Debug, Clone)]
pub struct ToolSchemaEntry {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
}

/// Type-erased output validator for use in operators.
///
/// Created via [`TypedOutput::into_validator`]. Stored as `Box<dyn OutputValidator>`
/// so operators don't need to carry the generic `T`.
pub trait OutputValidator: Send + Sync {
    /// The tool schema entry for the `return_result` tool.
    fn tool_schema(&self) -> &ToolSchemaEntry;

    /// Maximum number of validation retries.
    fn max_retries(&self) -> u32;

    /// Try to validate a tool call input. Returns `Ok(value)` as a `serde_json::Value`
    /// on success, or `Err(message)` with a human-readable error for retry.
    fn validate(&self, input: &Value) -> Result<Value, String>;
}

struct OutputValidatorImpl<T: DeserializeOwned> {
    _marker: PhantomData<T>,
    max_retries: u32,
    schema_entry: ToolSchemaEntry,
}

impl<T: DeserializeOwned + Send + Sync> OutputValidator for OutputValidatorImpl<T> {
    fn tool_schema(&self) -> &ToolSchemaEntry {
        &self.schema_entry
    }

    fn max_retries(&self) -> u32 {
        self.max_retries
    }

    fn validate(&self, input: &Value) -> Result<Value, String> {
        let result = input
            .get("result")
            .ok_or_else(|| "Missing 'result' field in return_result call".to_string())?;
        // Validate by attempting deserialization, then return the raw value
        serde_json::from_value::<T>(result.clone())
            .map_err(|e| format!("Validation error: {e}"))?;
        Ok(result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct CityInfo {
        name: String,
        population: u64,
        country: String,
    }

    #[test]
    fn schema_generation() {
        let typed = TypedOutput::<CityInfo>::new();
        let schema = typed.tool_schema();
        assert_eq!(schema.name, "return_result");
        assert!(schema.description.contains("structured result"));

        let input = &schema.input_schema;
        assert_eq!(input["type"], "object");
        assert!(input["properties"]["result"].is_object());
        assert!(
            input["required"]
                .as_array()
                .unwrap()
                .contains(&json!("result"))
        );
    }

    #[test]
    fn validator_success() {
        let validator = TypedOutput::<CityInfo>::new().into_validator();
        let input = json!({
            "result": {
                "name": "Tokyo",
                "population": 13960000_u64,
                "country": "Japan"
            }
        });
        let result = validator.validate(&input).unwrap();
        assert_eq!(result["name"], "Tokyo");
    }

    #[test]
    fn validator_missing_field() {
        let validator = TypedOutput::<CityInfo>::new().into_validator();
        let input = json!({
            "result": {
                "name": "Tokyo"
            }
        });
        let err = validator.validate(&input).unwrap_err();
        assert!(err.contains("Validation error"), "got: {err}");
    }

    #[test]
    fn validator_missing_result_key() {
        let validator = TypedOutput::<CityInfo>::new().into_validator();
        let input = json!({
            "name": "Tokyo",
            "population": 13960000_u64,
            "country": "Japan"
        });
        let err = validator.validate(&input).unwrap_err();
        assert!(err.contains("Missing 'result'"), "got: {err}");
    }

    #[test]
    fn max_retries_default_and_builder() {
        let typed = TypedOutput::<CityInfo>::new();
        assert_eq!(typed.max_retries(), 3);

        let typed = typed.with_max_retries(5);
        assert_eq!(typed.max_retries(), 5);
    }

    #[test]
    fn validator_preserves_max_retries() {
        let validator = TypedOutput::<CityInfo>::new()
            .with_max_retries(7)
            .into_validator();
        assert_eq!(validator.max_retries(), 7);
    }
}
