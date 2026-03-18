//! Tool dispatch operation.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::DispatchContext;
use skg_tool::ToolRegistry;
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
    /// The dispatch context for tool execution.
    pub dispatch_ctx: DispatchContext,
}

impl ExecuteTool {
    /// Create a new tool dispatch operation.
    pub fn new(call: ToolCall, registry: ToolRegistry, dispatch_ctx: DispatchContext) -> Self {
        Self {
            call,
            registry,
            dispatch_ctx,
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

        ctx.metrics.tool_calls_total += 1;
        match tool.call(self.call.input.clone(), &self.dispatch_ctx).await {
            Ok(result_json) => Ok(format_tool_result(&result_json)),
            Err(e) => {
                ctx.metrics.tool_calls_failed += 1;
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::DispatchContext;
    use layer0::id::{DispatchId, OperatorId};
    use serde_json::json;
    use skg_tool::{ToolDyn, ToolError};
    use std::pin::Pin;
    use std::sync::Arc;

    struct SucceedingTool;

    impl ToolDyn for SucceedingTool {
        fn name(&self) -> &str {
            "ok"
        }
        fn description(&self) -> &str {
            "succeeds"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Ok(json!("ok")) })
        }
    }

    struct FailingTool;

    impl ToolDyn for FailingTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "always fails"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Err(ToolError::ExecutionFailed("boom".into())) })
        }
    }

    fn test_dispatch_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::new("test-op"))
    }

    fn make_call(name: &str) -> skg_turn::infer::ToolCall {
        skg_turn::infer::ToolCall {
            id: "call_1".into(),
            name: name.into(),
            input: json!({}),
        }
    }

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

    #[tokio::test]
    async fn tool_success_increments_total() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SucceedingTool));
        let op = ExecuteTool::new(make_call("ok"), registry, test_dispatch_ctx());
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        assert!(result.is_ok());
        assert_eq!(ctx.metrics.tool_calls_total, 1);
        assert_eq!(ctx.metrics.tool_calls_failed, 0);
    }

    #[tokio::test]
    async fn tool_failure_increments_failed() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FailingTool));
        let op = ExecuteTool::new(make_call("fail"), registry, test_dispatch_ctx());
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        assert!(result.is_err());
        assert_eq!(ctx.metrics.tool_calls_failed, 1);
        assert_eq!(ctx.metrics.tool_calls_total, 1);
    }

    #[tokio::test]
    async fn unknown_tool_halts_without_metric_change() {
        let registry = ToolRegistry::new();
        let op = ExecuteTool::new(make_call("nonexistent"), registry, test_dispatch_ctx());
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        assert!(matches!(result, Err(EngineError::Halted { .. })));
        assert_eq!(ctx.metrics.tool_calls_total, 0);
        assert_eq!(ctx.metrics.tool_calls_failed, 0);
    }
}
