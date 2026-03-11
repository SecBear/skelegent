//! Bridge between [`ToolDyn`]/[`ToolRegistry`] and the operator protocol.
//!
//! [`ToolOperator`] lets any existing tool participate in orchestrator dispatch
//! without rewriting it. [`ToolRegistryOrchestrator`] makes a whole registry
//! available as an [`Orchestrator`], allowing operators to use tools via
//! the standard dispatch protocol.

use std::sync::Arc;

use async_trait::async_trait;
use layer0::dispatch::Capabilities;
use layer0::effect::SignalPayload;
use layer0::operator::Operator;
use layer0::orchestrator::QueryPayload;
use layer0::{
    Content, DurationMs, ExitReason, OperatorError, OperatorId, OperatorInput, OperatorOutput,
    OrchError, Orchestrator, SubDispatchRecord, ToolMetadata, WorkflowId,
};

use crate::{ToolCallContext, ToolConcurrencyHint, ToolDyn, ToolRegistry};

/// Wraps an `Arc<dyn ToolDyn>` as an `Operator`, bridging the tool abstraction
/// to the operator protocol. This allows existing tools to participate in
/// orchestrator dispatch without rewriting them.
pub struct ToolOperator {
    tool: Arc<dyn ToolDyn>,
}

impl ToolOperator {
    /// Create a new `ToolOperator` wrapping the given tool.
    pub fn new(tool: Arc<dyn ToolDyn>) -> Self {
        Self { tool }
    }

    /// Extract [`ToolMetadata`] from the wrapped [`ToolDyn`].
    ///
    /// `parallel_safe` mirrors the concurrency hint: `Shared` → `true`,
    /// `Exclusive` → `false`.
    pub fn metadata(&self) -> ToolMetadata {
        let parallel_safe = matches!(self.tool.concurrency_hint(), ToolConcurrencyHint::Shared);
        ToolMetadata::new(
            self.tool.name(),
            self.tool.description(),
            self.tool.input_schema(),
            parallel_safe,
        )
    }
}

#[async_trait]
impl Operator for ToolOperator {
    /// Execute the wrapped tool.
    ///
    /// `input.message` must be valid JSON text representing the tool's input.
    /// Any parse failure is surfaced as `OperatorError::NonRetryable`.
    async fn execute(&self, input: OperatorInput, _caps: &Capabilities) -> Result<OperatorOutput, OperatorError> {
        let text = input.message.as_text().unwrap_or("null");
        let tool_input: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| OperatorError::NonRetryable(format!("invalid tool input JSON: {e}")))?;

        let ctx = ToolCallContext::new(OperatorId::new("agent"));
        match self.tool.call(tool_input, &ctx).await {
            Ok(result) => {
                let mut output =
                    OperatorOutput::new(Content::text(result.to_string()), ExitReason::Complete);
                output.metadata.sub_dispatches.push(SubDispatchRecord::new(
                    self.tool.name(),
                    DurationMs::ZERO,
                    true,
                ));
                Ok(output)
            }
            Err(err) => Err(OperatorError::SubDispatch {
                operator: self.tool.name().to_string(),
                message: err.to_string(),
            }),
        }
    }
}

/// Wraps a [`ToolRegistry`] and implements [`Orchestrator`].
///
/// This allows any operator that speaks the orchestration
/// protocol) to use existing tools via `dispatch()` by name, without touching
/// individual tool implementations.
///
/// Dispatch is sequential: tools may not be `Send`-safe for parallel
/// execution, and the concurrency hint should be respected by the planner
/// above this layer.
pub struct ToolRegistryOrchestrator {
    registry: ToolRegistry,
}

impl ToolRegistryOrchestrator {
    /// Create a new orchestrator backed by `registry`.
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Orchestrator for ToolRegistryOrchestrator {
    /// Dispatch by looking up `operator` as a tool name in the registry.
    ///
    /// Returns `OrchError::OperatorNotFound` when the name is not registered.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let tool = self
            .registry
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?;

        let operator = ToolOperator::new(Arc::clone(tool));
        operator.execute(input, &Capabilities::none()).await.map_err(OrchError::from)
    }

    /// Sequential dispatch. Tools may not be `Send`-safe across parallel
    /// tasks, so each invocation runs to completion before the next starts.
    async fn dispatch_many(
        &self,
        tasks: Vec<(OperatorId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut results = Vec::with_capacity(tasks.len());
        for (operator, input) in tasks {
            results.push(self.dispatch(&operator, input).await);
        }
        results
    }

    /// No durable workflow backing — signals are accepted and discarded.
    async fn signal(&self, _target: &WorkflowId, _signal: SignalPayload) -> Result<(), OrchError> {
        Ok(())
    }

    /// No durable workflow backing — queries always return `null`.
    async fn query(
        &self,
        _target: &WorkflowId,
        _query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError> {
        Ok(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolCallContext, ToolConcurrencyHint, ToolDyn, ToolError, ToolRegistry};
    use layer0::operator::TriggerType;
    use layer0::{Content, ExitReason, OperatorError, OperatorId, OperatorInput, OrchError};
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    // ── Minimal test tools ────────────────────────────────────────────────────

    struct EchoTool;

    impl ToolDyn for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input back"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
            _ctx: &ToolCallContext,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
        fn concurrency_hint(&self) -> ToolConcurrencyHint {
            ToolConcurrencyHint::Shared
        }
    }

    struct FailTool;

    impl ToolDyn for FailTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCallContext,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>>
        {
            Box::pin(async { Err(ToolError::ExecutionFailed("always fails".into())) })
        }
    }

    fn make_input(json_text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(json_text), TriggerType::Task)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn tool_operator_metadata_extraction() {
        let tool: Arc<dyn ToolDyn> = Arc::new(EchoTool);
        let op = ToolOperator::new(Arc::clone(&tool));
        let meta = op.metadata();

        assert_eq!(meta.name, "echo");
        assert_eq!(meta.description, "Echoes input back");
        assert_eq!(meta.input_schema, json!({"type": "object"}));
        // EchoTool is Shared → parallel_safe == true
        assert!(meta.parallel_safe);
    }

    #[tokio::test]
    async fn tool_operator_execute_success() {
        let tool: Arc<dyn ToolDyn> = Arc::new(EchoTool);
        let op = ToolOperator::new(tool);

        let input = make_input(r#"{"msg": "hello"}"#);
        let output = op.execute(input, &Capabilities::none()).await.expect("should succeed");

        assert_eq!(output.exit_reason, ExitReason::Complete);

        // The text response should contain the echoed JSON.
        let text = output.message.as_text().expect("should be text");
        let parsed: serde_json::Value = serde_json::from_str(text).expect("should be valid JSON");
        assert_eq!(parsed, json!({"echoed": {"msg": "hello"}}));

        // Exactly one sub-dispatch recorded.
        assert_eq!(output.metadata.sub_dispatches.len(), 1);
        let rec = &output.metadata.sub_dispatches[0];
        assert_eq!(rec.name, "echo");
        assert!(rec.success);
    }

    #[tokio::test]
    async fn tool_operator_execute_error() {
        let tool: Arc<dyn ToolDyn> = Arc::new(FailTool);
        let op = ToolOperator::new(tool);

        let input = make_input("{}");
        let err = op.execute(input, &Capabilities::none()).await.expect_err("should fail");

        match err {
            OperatorError::SubDispatch { operator, message } => {
                assert_eq!(operator, "fail");
                assert!(
                    message.contains("always fails"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected SubDispatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn registry_orchestrator_dispatch() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let orch = ToolRegistryOrchestrator::new(reg);

        let operator = OperatorId::new("echo");
        let input = make_input(r#"{"x": 42}"#);
        let output = orch
            .dispatch(&operator, input)
            .await
            .expect("should succeed");

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let text = output.message.as_text().expect("should be text");
        let parsed: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
        assert_eq!(parsed, json!({"echoed": {"x": 42}}));
    }

    #[tokio::test]
    async fn registry_orchestrator_not_found() {
        let reg = ToolRegistry::new(); // empty
        let orch = ToolRegistryOrchestrator::new(reg);

        let operator = OperatorId::new("unknown_tool");
        let input = make_input("{}");
        let err = orch
            .dispatch(&operator, input)
            .await
            .expect_err("should fail");

        match err {
            OrchError::OperatorNotFound(name) => {
                assert_eq!(name, "unknown_tool");
            }
            other => panic!("expected OperatorNotFound, got {other:?}"),
        }
    }
}
