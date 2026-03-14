//! Bridge between [`ToolDyn`]/[`ToolRegistry`] and the operator protocol.
//!
//! [`ToolOperator`] lets any existing tool participate in orchestrator dispatch
//! without rewriting it. [`ToolRegistryOrchestrator`] makes a whole registry
//! available as a [`Dispatcher`], allowing operators to use tools via
//! the standard dispatch protocol.

use std::sync::Arc;

use async_trait::async_trait;
use layer0::dispatch::EffectEmitter;
use layer0::operator::Operator;
use layer0::{
    Content, DispatchContext, DispatchId, DurationMs, ExitReason, OperatorError, OperatorId,
    OperatorInput, OperatorOutput, OrchError, SubDispatchRecord, ToolMetadata,
};

use crate::{ToolConcurrencyHint, ToolDyn, ToolRegistry};

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
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
        _emitter: &EffectEmitter,
    ) -> Result<OperatorOutput, OperatorError> {
        let text = input.message.as_text().unwrap_or("null");
        let tool_input: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| OperatorError::non_retryable(format!("invalid tool input JSON: {e}")))?;

        let ctx = DispatchContext::new(DispatchId::new("tool-call"), OperatorId::new("agent"));
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
                source: Box::new(err),
            }),
        }
    }
}

/// Wraps a [`ToolRegistry`] and implements [`Dispatcher`].
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
impl layer0::dispatch::Dispatcher for ToolRegistryOrchestrator {
    /// Dispatch by looking up `operator` as a tool name in the registry.
    ///
    /// Returns `OrchError::OperatorNotFound` when the name is not registered.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<layer0::DispatchHandle, OrchError> {
        let tool = self
            .registry
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?;

        let operator_id = operator.clone();
        let operator = ToolOperator::new(Arc::clone(tool));
        let (handle, sender) =
            layer0::DispatchHandle::channel(layer0::DispatchId::new("tool-registry"));
        tokio::spawn(async move {
            let ctx = DispatchContext::new(layer0::DispatchId::new("tool-registry"), operator_id);
            match operator.execute(input, &ctx, &EffectEmitter::noop()).await {
                Ok(output) => {
                    let _ = sender
                        .send(layer0::DispatchEvent::Completed { output })
                        .await;
                }
                Err(err) => {
                    let _ = sender
                        .send(layer0::DispatchEvent::Failed {
                            error: OrchError::from(err),
                        })
                        .await;
                }
            }
        });
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolConcurrencyHint, ToolDyn, ToolError, ToolRegistry};
    use layer0::operator::TriggerType;
    use layer0::{
        Content, DispatchId, Dispatcher, ExitReason, OperatorError, OperatorId, OperatorInput,
        OrchError,
    };
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
            _ctx: &DispatchContext,
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
            _ctx: &DispatchContext,
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
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let output = op
            .execute(input, &ctx, &EffectEmitter::noop())
            .await
            .expect("should succeed");

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
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
        let err = op
            .execute(input, &ctx, &EffectEmitter::noop())
            .await
            .expect_err("should fail");

        match err {
            OperatorError::SubDispatch { operator, source } => {
                assert_eq!(operator, "fail");
                let msg = source.to_string();
                assert!(
                    msg.contains("always fails"),
                    "unexpected message: {msg}"
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
            .expect("should succeed")
            .collect()
            .await
            .expect("should complete");

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
