#![deny(missing_docs)]
//! In-process implementation of layer0's Dispatcher trait.
//!
//! Dispatches to registered operators via `HashMap<OperatorId, Arc<dyn Operator>>`.
//! Concurrent dispatch uses `tokio::spawn`. No durability — operators that fail
//! are not retried and state is not persisted. Workflow `signal` semantics and a
//! minimal `query` are implemented via an in-memory, per-workflow signal journal.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::{DispatchEvent, DispatchHandle, Dispatcher};
use layer0::effect::SignalPayload;
use layer0::error::OrchError;
use layer0::id::{OperatorId, WorkflowId};
use layer0::middleware::{DispatchNext, DispatchStack};
use layer0::operator::{Operator, OperatorInput};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-process orchestrator that dispatches to registered operators.
///
/// Uses `Arc<dyn Operator>` for true concurrent dispatch via `tokio::spawn`.
/// No durability, but tracks workflow signals in-memory for `signal`/`query`.
/// Suitable for development, testing, and single-process deployments.
pub struct LocalOrch {
    agents: HashMap<String, Arc<dyn Operator>>,
    // Per-workflow signal journal
    workflow_signals: RwLock<HashMap<String, Vec<SignalPayload>>>,
    /// Optional middleware stack for Pre/PostDispatch interception.
    middleware: Option<DispatchStack>,
}

impl LocalOrch {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            workflow_signals: RwLock::new(HashMap::new()),
            middleware: None,
        }
    }

    /// Register an operator with the orchestrator.
    pub fn register(&mut self, id: OperatorId, op: Arc<dyn Operator>) {
        self.agents.insert(id.to_string(), op);
    }

    /// Return the number of recorded signals for a workflow.
    pub async fn signal_count(&self, target: &WorkflowId) -> usize {
        let workflows = self.workflow_signals.read().await;
        workflows.get(target.as_str()).map(|v| v.len()).unwrap_or(0)
    }

    /// Attach a middleware stack for dispatch interception.
    pub fn with_middleware(mut self, stack: DispatchStack) -> Self {
        self.middleware = Some(stack);
        self
    }
}

impl Default for LocalOrch {
    fn default() -> Self {
        Self::new()
    }
}

/// Terminal dispatch: looks up the operator and calls `execute()`.
struct OperatorDispatch<'a> {
    agents: &'a HashMap<String, Arc<dyn Operator>>,
}

#[async_trait]
impl DispatchNext for OperatorDispatch<'_> {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let op = self
            .agents
            .get(ctx.operator_id.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(ctx.operator_id.to_string()))?
            .clone();
        let (handle, sender) = DispatchHandle::channel(ctx.dispatch_id.clone());
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match op.execute(input, &ctx).await {
                Ok(output) => {
                    let _ = sender.send(DispatchEvent::Completed { output }).await;
                }
                Err(err) => {
                    let _ = sender
                        .send(DispatchEvent::Failed {
                            error: OrchError::OperatorError(err),
                        })
                        .await;
                }
            }
        });
        Ok(handle)
    }
}

#[async_trait]
impl Dispatcher for LocalOrch {
    #[tracing::instrument(skip_all, fields(operator_id = %ctx.operator_id))]
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let terminal = OperatorDispatch {
            agents: &self.agents,
        };

        if let Some(ref stack) = self.middleware {
            stack.dispatch_with(ctx, input, &terminal).await
        } else {
            terminal.dispatch(ctx, input).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::dispatch::{DispatchHandle, Dispatcher};
    use layer0::effect::SignalPayload;
    use layer0::error::{OperatorError, OrchError};
    use layer0::id::{DispatchId, OperatorId, WorkflowId};
    use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
    use layer0::operator::{Operator, OperatorInput, OperatorOutput, TriggerType};
    use layer0::{DispatchContext, ExitReason};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Simple operator that echoes input back as output.
    struct EchoOp;

    #[async_trait]
    impl Operator for EchoOp {
        async fn execute(
            &self,
            input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    fn simple_input(msg: &str) -> OperatorInput {
        OperatorInput::new(Content::text(msg), TriggerType::User)
    }

    fn test_ctx(name: &str) -> DispatchContext {
        DispatchContext::new(DispatchId::new(name), OperatorId::new(name))
    }

    #[tokio::test]
    async fn dispatch_to_registered_operator() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));

        let output = orch
            .dispatch(&test_ctx("echo"), simple_input("hello"))
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();

        assert_eq!(output.message, Content::text("hello"));
    }

    #[tokio::test]
    async fn dispatch_to_unknown_operator_returns_error() {
        let orch = LocalOrch::new();

        let result = orch
            .dispatch(&test_ctx("nonexistent"), simple_input("test"))
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("nonexistent"),
            "error should contain operator name, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn concurrent_dispatches_do_not_deadlock() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));
        let orch = Arc::new(orch);

        let mut handles = Vec::new();
        for i in 0..10 {
            let orch = Arc::clone(&orch);
            let msg = format!("msg-{i}");
            handles.push(tokio::spawn(async move {
                orch.dispatch(&test_ctx("echo"), simple_input(&msg))
                    .await
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.expect("task should not panic"));
        }

        assert_eq!(results.len(), 10, "all 10 dispatches must complete");
    }

    #[tokio::test]
    async fn signal_journal_records_and_retrieves() {
        let orch = LocalOrch::new();
        let wf_id = WorkflowId::new("wf-1");

        assert_eq!(orch.signal_count(&wf_id).await, 0);

        // Manually insert signals via the workflow_signals field
        {
            let mut signals = orch.workflow_signals.write().await;
            signals
                .entry(wf_id.to_string())
                .or_default()
                .push(SignalPayload::new(
                    "test-signal",
                    serde_json::json!({"key": "value"}),
                ));
        }
        assert_eq!(orch.signal_count(&wf_id).await, 1);

        // Insert a second signal
        {
            let mut signals = orch.workflow_signals.write().await;
            signals
                .entry(wf_id.to_string())
                .or_default()
                .push(SignalPayload::new("another-signal", serde_json::json!(42)));
        }
        assert_eq!(orch.signal_count(&wf_id).await, 2);
    }

    #[tokio::test]
    async fn dispatch_with_middleware_stack() {
        let call_count = Arc::new(AtomicUsize::new(0));

        struct CountingMiddleware {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl DispatchMiddleware for CountingMiddleware {
            async fn dispatch(
                &self,
                ctx: &DispatchContext,
                input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, OrchError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                next.dispatch(ctx, input).await
            }
        }

        let mw = Arc::new(CountingMiddleware {
            calls: Arc::clone(&call_count),
        });

        let stack = DispatchStack::builder().observe(mw).build();

        let mut orch = LocalOrch::new().with_middleware(stack);
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));

        let output = orch
            .dispatch(&test_ctx("echo"), simple_input("middleware-test"))
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();

        assert_eq!(output.message, Content::text("middleware-test"));
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "middleware must be called exactly once"
        );
    }
}
