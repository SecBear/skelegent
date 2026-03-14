//! LocalOrchestrator — in-process orchestrator with a HashMap of operators.

use crate::dispatch::{DispatchEvent, DispatchHandle, DispatchSender, EffectEmitter};
use crate::dispatch_context::DispatchContext;
use crate::error::OrchError;
use crate::id::{DispatchId, OperatorId};
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
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let op = self
            .operators
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?
            .clone();

        let dispatch_id = DispatchId::new(format!("test-{}", uuid_v4()));
        let ctx = DispatchContext::new(dispatch_id.clone(), operator.clone());
        let (handle, sender) = DispatchHandle::channel(dispatch_id);

        tokio::spawn(run_dispatch(op, input, ctx, sender));

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
            // Emit progress/artifact effects as events before the terminal event.
            for effect in &output.effects {
                match effect {
                    crate::effect::Effect::Progress { content } => {
                        let _ = sender
                            .send(DispatchEvent::Progress {
                                content: content.clone(),
                            })
                            .await;
                    }
                    crate::effect::Effect::Artifact { artifact } => {
                        let _ = sender
                            .send(DispatchEvent::ArtifactProduced {
                                artifact: artifact.clone(),
                            })
                            .await;
                    }
                    _ => {}
                }
            }
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

/// Simple pseudo-UUID v4 for test dispatch IDs.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:032x}")
}