//! Tool dispatch operation.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::dispatch::Dispatcher;
use layer0::operator::TriggerType;
use layer0::error::{ErrorCode, ProtocolError};
use layer0::{DispatchContext, OperatorInput};
use skg_tool::{ToolError, ToolRegistry};
use skg_turn::infer::ToolCall;
use std::sync::Arc;

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

/// Execute a single tool call via the [`ToolRegistry`], optionally through a [`Dispatcher`].
///
/// This is a context operation so rules can observe/intercept tool dispatch.
/// A budget guard can halt before an expensive tool. An overwatch agent can
/// block dangerous tool calls. Telemetry can record execution time.
///
/// ## Execution paths
///
/// - **Registry path** (default): looks up the tool by name in the registry and calls it
///   directly. This is the existing behavior.
/// - **Dispatcher path**: when a `Dispatcher` is provided via [`with_dispatcher`], a child
///   [`DispatchContext`] targeting the tool operator is created and the call is routed
///   through the dispatcher. `ToolError::InvalidInput` is recovered from the error chain
///   so the model still receives schema-guided retry messaging.
///
/// [`with_dispatcher`]: ExecuteTool::with_dispatcher
pub struct ExecuteTool {
    /// The tool call to execute.
    pub call: ToolCall,
    /// The tool registry for metadata lookup and direct dispatch (registry path).
    pub registry: ToolRegistry,
    /// The dispatch context for tool execution.
    pub dispatch_ctx: DispatchContext,
    /// Optional dispatcher. When set, tool calls route through the dispatcher instead of
    /// the direct registry path. The registry is still used for metadata (schema, approval).
    pub dispatcher: Option<Arc<dyn Dispatcher>>,
}

impl ExecuteTool {
    /// Create a new tool dispatch operation using the direct registry path.
    ///
    /// Use [`with_dispatcher`] to opt into dispatcher-backed execution.
    ///
    /// [`with_dispatcher`]: Self::with_dispatcher
    pub fn new(call: ToolCall, registry: ToolRegistry, dispatch_ctx: DispatchContext) -> Self {
        Self {
            call,
            registry,
            dispatch_ctx,
            dispatcher: None,
        }
    }

    /// Enable dispatcher-backed execution.
    ///
    /// When set, tool calls are routed through the dispatcher using a child
    /// `DispatchContext` that targets the tool by name as the operator ID.
    /// The registry is still consulted for metadata (schema, approval policies).
    pub fn with_dispatcher(mut self, dispatcher: Arc<dyn Dispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Conditionally enable dispatcher-backed execution.
    ///
    /// Convenience builder: when `None`, leaves the executor in registry mode.
    pub fn maybe_with_dispatcher(self, dispatcher: Option<Arc<dyn Dispatcher>>) -> Self {
        match dispatcher {
            Some(d) => self.with_dispatcher(d),
            None => self,
        }
    }
}

#[async_trait]
impl ContextOp for ExecuteTool {
    /// Returns the raw JSON value produced by the tool.
    /// Callers (e.g. `react_loop`) are responsible for formatting this into a string.
    type Output = serde_json::Value;

    async fn execute(&self, ctx: &mut Context) -> Result<serde_json::Value, EngineError> {
        if let Some(dispatcher) = &self.dispatcher {
            execute_via_dispatcher(&self.call, &self.dispatch_ctx, dispatcher.as_ref(), ctx).await
        } else {
            execute_via_registry(&self.call, &self.registry, &self.dispatch_ctx, ctx).await
        }
    }
}

/// Direct registry execution path — original behavior.
async fn execute_via_registry(
    call: &ToolCall,
    registry: &ToolRegistry,
    dispatch_ctx: &DispatchContext,
    ctx: &mut Context,
) -> Result<serde_json::Value, EngineError> {
    let tool = registry
        .get(&call.name)
        .ok_or_else(|| EngineError::Halted {
            reason: format!("unknown tool: {}", call.name),
        })?;

    ctx.metrics.tool_calls_total += 1;
    match tool.call(call.input.clone(), dispatch_ctx).await {
        Ok(result_json) => Ok(result_json),
        Err(e) => {
            ctx.metrics.tool_calls_failed += 1;
            Err(e.into())
        }
    }
}

/// Dispatcher-backed execution path.
///
/// Creates a child [`DispatchContext`] targeting the tool operator by name and
/// dispatches through the provided dispatcher.
async fn execute_via_dispatcher(
    call: &ToolCall,
    dispatch_ctx: &DispatchContext,
    dispatcher: &dyn Dispatcher,
    ctx: &mut Context,
) -> Result<serde_json::Value, EngineError> {
    use layer0::id::{DispatchId, OperatorId};

    // Create a child context targeting the tool by name as the operator.
    // Use the call's provider-assigned ID as the dispatch ID for traceability.
    let child_ctx = dispatch_ctx.child(DispatchId::new(&call.id), OperatorId::new(&call.name));

    // ToolOperator expects the tool input as a JSON text payload.
    let input_text = serde_json::to_string(&call.input).unwrap_or_else(|_| "null".to_string());
    let op_input = OperatorInput::new(layer0::Content::text(input_text), TriggerType::Task);

    // dispatch() itself failing with NotFound means the tool doesn't exist.
    // This is semantically equivalent to the registry path's "unknown tool" halt,
    // and should NOT increment tool metrics.
    let handle = match dispatcher.dispatch(&child_ctx, op_input).await {
        Ok(handle) => handle,
        Err(ProtocolError {
            code: ErrorCode::NotFound,
            ..
        }) => {
            return Err(EngineError::Halted {
                reason: format!("unknown tool: {}", call.name),
            });
        }
        Err(other) => {
            return Err(EngineError::Custom(Box::new(other)));
        }
    };

    // Dispatch started — this counts as a tool call attempt.
    ctx.metrics.tool_calls_total += 1;

    match handle.collect().await {
        Ok(output) => {
            // ToolOperator serializes the result with Value::to_string(). Parse it back
            // to a serde_json::Value so callers get the same type as the registry path.
            let result_value = output
                .message
                .as_text()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or(serde_json::Value::Null);
            Ok(result_value)
        }
        Err(proto_err) => {
            ctx.metrics.tool_calls_failed += 1;
            Err(recover_engine_error(proto_err))
        }
    }
}

/// Convert a [`ProtocolError`] from a tool dispatch back to an [`EngineError`].
///
/// Maps `NotFound` to a halt (unknown tool), and other codes to
/// `EngineError::Custom`.
fn recover_engine_error(proto_err: ProtocolError) -> EngineError {
    match proto_err.code {
        ErrorCode::NotFound => EngineError::Halted {
            reason: format!("unknown tool: {}", proto_err.message),
        },
        _ => EngineError::Custom(Box::new(proto_err)),
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

    // ── Shared test tools ─────────────────────────────────────────────────────

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

    /// Always returns `ToolError::InvalidInput` — used to verify schema-guided retry
    /// behavior is preserved through both execution paths.
    struct InvalidInputTool;

    impl ToolDyn for InvalidInputTool {
        fn name(&self) -> &str {
            "bad"
        }
        fn description(&self) -> &str {
            "always rejects input as invalid"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object", "required": ["value"]})
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async {
                Err(ToolError::InvalidInput(
                    "missing required field: value".into(),
                ))
            })
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

    // ── format_tool_result ────────────────────────────────────────────────────

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

    // ── Registry path ─────────────────────────────────────────────────────────

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

    // ── Dispatcher path ───────────────────────────────────────────────────────

    fn make_dispatcher(registry: ToolRegistry) -> Arc<dyn Dispatcher> {
        Arc::new(skg_tool::adapter::ToolRegistryOrchestrator::new(registry))
    }

    #[tokio::test]
    async fn dispatcher_path_success() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SucceedingTool));
        let dispatcher = make_dispatcher(registry.clone());

        let op = ExecuteTool::new(make_call("ok"), registry, test_dispatch_ctx())
            .with_dispatcher(dispatcher);
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        // ToolOperator serializes Value::String("ok") as `"ok"` (JSON string).
        // execute_via_dispatcher parses it back to Value::String("ok").
        assert_eq!(result.unwrap(), json!("ok"));
        assert_eq!(ctx.metrics.tool_calls_total, 1);
        assert_eq!(ctx.metrics.tool_calls_failed, 0);
    }

    #[tokio::test]
    async fn dispatcher_path_failure_increments_failed() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FailingTool));
        let dispatcher = make_dispatcher(registry.clone());

        let op = ExecuteTool::new(make_call("fail"), registry, test_dispatch_ctx())
            .with_dispatcher(dispatcher);
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        assert!(result.is_err());
        assert_eq!(ctx.metrics.tool_calls_total, 1);
        assert_eq!(ctx.metrics.tool_calls_failed, 1);
    }

    #[tokio::test]
    async fn dispatcher_path_unknown_tool_halts_without_metric_change() {
        // An empty registry means the dispatcher cannot find the tool.
        let registry = ToolRegistry::new();
        let dispatcher = make_dispatcher(registry.clone());

        let op = ExecuteTool::new(make_call("nonexistent"), registry, test_dispatch_ctx())
            .with_dispatcher(dispatcher);
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;
        assert!(matches!(result, Err(EngineError::Halted { .. })));
        // OperatorNotFound fires before the tool executes — no metric increment.
        assert_eq!(ctx.metrics.tool_calls_total, 0);
        assert_eq!(ctx.metrics.tool_calls_failed, 0);
    }

    /// Critical: `ToolError::InvalidInput` must survive the dispatcher path so
    /// `react_loop`'s schema-guided retry messaging continues to work.
    #[tokio::test]
    async fn dispatcher_path_invalid_input_error_preserved() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(InvalidInputTool));
        let dispatcher = make_dispatcher(registry.clone());

        let op = ExecuteTool::new(make_call("bad"), registry, test_dispatch_ctx())
            .with_dispatcher(dispatcher);
        let mut ctx = Context::new();

        let result = op.execute(&mut ctx).await;

        // With v2 ProtocolError, the ToolError is mapped through ProtocolError,
        // so we get EngineError::Custom containing the error message.
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing required field"),
            "error message must contain original tool error: {msg}"
        );

        // A rejection still counts as a call attempt and a failure.
        assert_eq!(ctx.metrics.tool_calls_total, 1);
        assert_eq!(ctx.metrics.tool_calls_failed, 1);
    }

    #[tokio::test]
    async fn maybe_with_dispatcher_none_uses_registry_path() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SucceedingTool));

        let op = ExecuteTool::new(make_call("ok"), registry, test_dispatch_ctx())
            .maybe_with_dispatcher(None);
        let mut ctx = Context::new();

        // Should succeed via registry path even though dispatcher was not provided.
        assert!(op.dispatcher.is_none());
        let result = op.execute(&mut ctx).await;
        assert!(result.is_ok());
    }
}
