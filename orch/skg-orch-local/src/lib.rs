#![deny(missing_docs)]
//! In-process implementation of layer0's Dispatcher trait.
//!
//! Dispatches to registered operators via `HashMap<OperatorId, Arc<dyn Operator>>`.
//! Concurrent dispatch uses `tokio::spawn`. No durability — operators that fail
//! are not retried and state is not persisted. Workflow `signal` semantics and a
//! minimal `query` are implemented via an in-memory, per-workflow signal journal.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::{DispatchEvent, DispatchHandle, Dispatcher, EffectEmitter};
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
        let emitter = EffectEmitter::new(sender.clone());
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match op.execute(input, &ctx, &emitter).await {
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
