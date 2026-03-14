//! LocalOrchestrator — in-process orchestrator with a HashMap of operators.

use crate::dispatch::{DispatchEvent, DispatchHandle, DispatchSender, EffectEmitter};
use crate::dispatch_context::DispatchContext;
use crate::error::OrchError;
use crate::id::OperatorId;
use crate::operator::{Operator, OperatorInput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// In-process orchestrator that dispatches operator invocations to registered operators.
/// Uses `Arc<dyn Operator>` for true concurrent dispatch via `tokio::spawn`.
pub struct LocalOrchestrator {
    operators: HashMap<String, Arc<dyn Operator>>,
}

impl LocalOrchestrator {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            operators: HashMap::new(),
        }
    }

    /// Register an operator with the orchestrator.
    pub fn register(&mut self, id: OperatorId, operator: Arc<dyn Operator>) {
        self.operators.insert(id.0, operator);
    }
}

impl Default for LocalOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl crate::dispatch::Dispatcher for LocalOrchestrator {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let op = self
            .operators
            .get(ctx.operator_id.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(ctx.operator_id.to_string()))?
            .clone();

        let (handle, sender) = DispatchHandle::channel(ctx.dispatch_id.clone());

        tokio::spawn(run_dispatch(op, input, ctx.clone(), sender));

        Ok(handle)
    }
}

/// Run an operator and send events through the dispatch channel.
async fn run_dispatch(
    op: Arc<dyn Operator>,
    input: OperatorInput,
    ctx: DispatchContext,
    sender: DispatchSender,
) {
    if sender.is_cancelled() {
        return;
    }
    let emitter = EffectEmitter::new(sender.clone());
    match op.execute(input, &ctx, &emitter).await {
        Ok(output) => {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        }
        Err(op_err) => {
            let _ = sender
                .send(DispatchEvent::Failed {
                    error: OrchError::OperatorError(op_err),
                })
                .await;
        }
    }
}
