//! Tool dispatch operation.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use skg_tool::{ToolCallContext, ToolRegistry};
use skg_turn::infer::ToolCall;

/// Convert a tool result JSON value to a string for the model.
///
/// String values are returned as-is. All other JSON types are serialized
/// with `to_string()`. This is the default formatting used by [`ExecuteTool`].
///
/// Override this by calling the tool directly and formatting the result yourself.
pub fn format_tool_result(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Execute a single tool call via the [`ToolRegistry`].
///
/// This is a context operation so rules can observe/intercept tool dispatch.
/// A budget guard can halt before an expensive tool. An overwatch agent can
/// block dangerous tool calls. Telemetry can record execution time.
pub struct ExecuteTool {
    /// The tool call to execute.
    pub call: ToolCall,
    /// The tool registry to dispatch against.
    pub registry: ToolRegistry,
    /// The tool call context for dependency injection.
    pub tool_ctx: ToolCallContext,
}

impl ExecuteTool {
    /// Create a new tool dispatch operation.
    pub fn new(call: ToolCall, registry: ToolRegistry, tool_ctx: ToolCallContext) -> Self {
        Self {
            call,
            registry,
            tool_ctx,
        }
    }
}

#[async_trait]
impl ContextOp for ExecuteTool {
    /// Returns the tool result as a JSON string.
    type Output = String;

    async fn execute(&self, ctx: &mut Context) -> Result<String, EngineError> {
        let tool = self
            .registry
            .get(&self.call.name)
            .ok_or_else(|| EngineError::Halted {
                reason: format!("unknown tool: {}", self.call.name),
            })?;

        let result_json = tool.call(self.call.input.clone(), &self.tool_ctx).await?;

        // Update metrics
        ctx.metrics.tool_calls_total += 1;

        // Convert JSON value to string representation for the model
        let result_str = format_tool_result(&result_json);

        Ok(result_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_tool_result_string() {
        let value = serde_json::Value::String("hello world".into());
        assert_eq!(format_tool_result(&value), "hello world");
    }

    #[test]
    fn test_format_tool_result_json() {
        let value = json!({ "key": "value" });
        let result = format_tool_result(&value);
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }

    #[test]
    fn test_format_tool_result_null() {
        let value = serde_json::Value::Null;
        assert_eq!(format_tool_result(&value), "null");
    }
}
