use layer0::DispatchContext;
use layer0::dispatch::Dispatcher;
use layer0::effect::Effect;
use layer0::error::OrchError;
use layer0::id::DispatchId;
use layer0::id::{OperatorId, WorkflowId};
use layer0::operator::{OperatorInput, OperatorOutput};
use skg_effects_core::{EffectHandler, EffectOutcome};
use std::sync::Arc;
use thiserror::Error;

/// Errors returned by `skg-orch-kit`.
#[derive(Debug, Error)]
pub enum KitError {
    /// Dispatch error.
    #[error("orchestrator error: {0}")]
    Dispatch(#[from] OrchError),
    /// Effect execution failed.
    #[error("effect execution failed: {0}")]
    Effect(String),
    /// The runner detected a loop or exceeded a safety bound.
    #[error("execution exceeded safety bounds: {0}")]
    Safety(String),
}

/// An observable event emitted by the runner while interpreting effects.
#[derive(Debug, Clone)]
pub enum ExecutionEvent {
    /// An agent was dispatched.
    Dispatched {
        /// Operator id that was dispatched.
        operator: OperatorId,
    },
    /// A memory write was executed.
    MemoryWritten {
        /// State key written.
        key: String,
    },
    /// A memory delete was executed.
    MemoryDeleted {
        /// State key deleted.
        key: String,
    },
    /// A delegate task was enqueued.
    DelegateEnqueued {
        /// Operator id enqueued for follow-up dispatch.
        operator: OperatorId,
    },
    /// A handoff task was enqueued.
    HandoffEnqueued {
        /// Operator id enqueued for follow-up dispatch.
        operator: OperatorId,
    },
    /// A signal was sent.
    Signaled {
        /// Workflow id signaled.
        target: WorkflowId,
        /// Signal type sent.
        signal_type: String,
    },
}

/// Trace of a single orchestrated run (initial dispatch plus any followups).
#[derive(Debug, Clone)]
pub struct ExecutionTrace {
    /// Outputs in dispatch order (first element is the initial dispatch output).
    pub outputs: Vec<OperatorOutput>,
    /// Events recorded while interpreting effects.
    pub events: Vec<ExecutionEvent>,
}

impl ExecutionTrace {
    /// Create an empty trace.
    pub fn new() -> Self {
        Self {
            outputs: vec![],
            events: vec![],
        }
    }
}

impl Default for ExecutionTrace {
    fn default() -> Self {
        Self::new()
    }
}

/// A small runner that executes an initial dispatch, then interprets effects
/// into follow-up dispatches until the queue is empty.
///
/// This is the core "glue" promised by `skg-orch-kit`: it proves that the
/// effect vocabulary is executable without forcing a DSL.
pub struct OrchestratedRunner {
    dispatcher: Arc<dyn Dispatcher>,
    effects: Arc<dyn EffectHandler>,
    max_followups: usize,
}

impl OrchestratedRunner {
    /// Create a new orchestrated runner.
    pub fn new(dispatcher: Arc<dyn Dispatcher>, effects: Arc<dyn EffectHandler>) -> Self {
        Self {
            dispatcher,
            effects,
            max_followups: 128,
        }
    }

    /// Set a safety bound on the number of follow-up dispatches.
    pub fn with_max_followups(mut self, max_followups: usize) -> Self {
        self.max_followups = max_followups;
        self
    }

    /// Dispatch an operator and interpret its effects until completion.
    pub async fn run(
        &self,
        operator: OperatorId,
        input: OperatorInput,
    ) -> Result<ExecutionTrace, KitError> {
        let mut trace = ExecutionTrace::new();
        let mut queue: Vec<(OperatorId, OperatorInput)> = vec![(operator, input)];
        let mut followups_executed = 0usize;

        while let Some((op_id, op_input)) = queue.pop() {
            trace.events.push(ExecutionEvent::Dispatched {
                operator: op_id.clone(),
            });
            let ctx = DispatchContext::new(DispatchId::new(op_id.as_str()), op_id.clone());
            let output = self
                .dispatcher
                .dispatch(&ctx, op_input)
                .await?
                .collect()
                .await?;

            // Interpret effects into state updates + followups.
            let mut followups: Vec<(OperatorId, OperatorInput)> = vec![];
            for effect in &output.effects {
                match self
                    .effects
                    .handle(effect, &ctx)
                    .await
                    .map_err(|e| KitError::Effect(e.to_string()))?
                {
                    EffectOutcome::Applied => match effect {
                        Effect::WriteMemory { key, .. } => {
                            trace
                                .events
                                .push(ExecutionEvent::MemoryWritten { key: key.clone() });
                        }
                        Effect::DeleteMemory { key, .. } => {
                            trace
                                .events
                                .push(ExecutionEvent::MemoryDeleted { key: key.clone() });
                        }
                        Effect::Signal { target, payload } => {
                            trace.events.push(ExecutionEvent::Signaled {
                                target: target.clone(),
                                signal_type: payload.signal_type.clone(),
                            });
                        }
                        _ => {}
                    },
                    EffectOutcome::Skipped => { /* no trace event */ }
                    EffectOutcome::Delegate { operator, input } => {
                        followups.push((operator.clone(), input));
                        trace.events.push(ExecutionEvent::DelegateEnqueued {
                            operator: operator.clone(),
                        });
                    }
                    EffectOutcome::Handoff { operator, input } => {
                        followups.push((operator.clone(), input));
                        trace.events.push(ExecutionEvent::HandoffEnqueued {
                            operator: operator.clone(),
                        });
                    }
                    _ => { /* forward-compat: EffectOutcome is non_exhaustive */ }
                }
            }

            trace.outputs.push(output);

            // Depth-first: push followups onto the queue.
            if !followups.is_empty() {
                followups_executed = followups_executed.saturating_add(followups.len());
                if followups_executed > self.max_followups {
                    return Err(KitError::Safety(format!(
                        "followup dispatch count exceeded max_followups={}",
                        self.max_followups
                    )));
                }
                queue.extend(followups);
            }
        }

        Ok(trace)
    }
}
