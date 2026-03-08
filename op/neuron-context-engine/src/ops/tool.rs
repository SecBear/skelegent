//! Tool dispatch operation.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use neuron_tool::{ToolCallContext, ToolRegistry};
use neuron_turn::infer::ToolCall;

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
        let result_str = match &result_json {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        Ok(result_str)
    }
}
