use async_trait::async_trait;
use layer0::effect::Effect;
use layer0::error::{OrchError, StateError};
use layer0::id::{AgentId, WorkflowId};
use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};
use layer0::orchestrator::Orchestrator;
use layer0::state::StateStore;
use std::sync::Arc;
use thiserror::Error;

/// Errors returned by `neuron-orch-kit`.
#[derive(Debug, Error)]
pub enum KitError {
    /// Orchestrator error.
    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] OrchError),
    /// State backend error.
    #[error("state error: {0}")]
    State(#[from] StateError),
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
        /// Agent id that was dispatched.
        agent: AgentId,
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
        /// Agent id enqueued for follow-up dispatch.
        agent: AgentId,
    },
    /// A handoff task was enqueued.
    HandoffEnqueued {
        /// Agent id enqueued for follow-up dispatch.
        agent: AgentId,
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

/// Effect interpretation policy.
///
/// The default `OrchestratedRunner` uses this trait as the single seam where
/// a product (like Sortie) can override semantics without adopting a DSL.
#[async_trait]
pub trait EffectInterpreter: Send + Sync {
    /// Interpret a single effect and optionally enqueue follow-up dispatches.
    async fn execute_effect(
        &self,
        effect: &Effect,
        followups: &mut Vec<(AgentId, OperatorInput)>,
        trace: &mut ExecutionTrace,
    ) -> Result<(), KitError>;
}

/// Default effect interpreter for local composition.
///
/// Interprets state effects directly against the supplied state store and
/// turns `Delegate`/`Handoff` into follow-up dispatches on the same orchestrator.
pub struct LocalEffectInterpreter<S: StateStore + ?Sized> {
    /// State backend used for memory effects.
    pub state: Arc<S>,
}

impl<S: StateStore + ?Sized> LocalEffectInterpreter<S> {
    /// Create a new local effect interpreter.
    pub fn new(state: Arc<S>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl<S: StateStore + ?Sized + 'static> EffectInterpreter for LocalEffectInterpreter<S> {
    async fn execute_effect(
        &self,
        effect: &Effect,
        followups: &mut Vec<(AgentId, OperatorInput)>,
        trace: &mut ExecutionTrace,
    ) -> Result<(), KitError> {
        match effect {
            Effect::WriteMemory { scope, key, value } => {
                self.state.write(scope, key, value.clone()).await?;
                trace
                    .events
                    .push(ExecutionEvent::MemoryWritten { key: key.clone() });
            }
            Effect::DeleteMemory { scope, key } => {
                self.state.delete(scope, key).await?;
                trace
                    .events
                    .push(ExecutionEvent::MemoryDeleted { key: key.clone() });
            }
            Effect::Signal { target, payload } => {
                trace.events.push(ExecutionEvent::Signaled {
                    target: target.clone(),
                    signal_type: payload.signal_type.clone(),
                });
                // The runner sends signals via the Orchestrator; this executor only records.
            }
            Effect::Delegate { agent, input } => {
                followups.push((agent.clone(), input.as_ref().clone()));
                trace.events.push(ExecutionEvent::DelegateEnqueued {
                    agent: agent.clone(),
                });
            }
            Effect::Handoff { agent, state } => {
                // v0 semantics: handoff state is serialized into a new task input.
                let mut input = OperatorInput::new(
                    layer0::content::Content::text(state.to_string()),
                    TriggerType::Task,
                );
                input.metadata = serde_json::Value::Null;
                followups.push((agent.clone(), input));
                trace.events.push(ExecutionEvent::HandoffEnqueued {
                    agent: agent.clone(),
                });
            }
            Effect::Log { .. } | Effect::Custom { .. } => {
                // v0: the kit ignores logs/custom effects by default.
            }
            _ => {
                // `Effect` is non_exhaustive; ignore forward-compatible variants by default.
            }
        }
        Ok(())
    }
}

/// A small runner that executes an initial dispatch, then interprets effects
/// into follow-up dispatches until the queue is empty.
///
/// This is the core “glue” promised by `neuron-orch-kit`: it proves that the
/// effect vocabulary is executable without forcing a DSL.
pub struct OrchestratedRunner<E: EffectInterpreter> {
    orch: Arc<dyn Orchestrator>,
    effects: Arc<E>,
    max_followups: usize,
}

impl<E: EffectInterpreter> OrchestratedRunner<E> {
    /// Create a new orchestrated runner.
    pub fn new(orch: Arc<dyn Orchestrator>, effects: Arc<E>) -> Self {
        Self {
            orch,
            effects,
            max_followups: 128,
        }
    }

    /// Set a safety bound on the number of follow-up dispatches.
    pub fn with_max_followups(mut self, max_followups: usize) -> Self {
        self.max_followups = max_followups;
        self
    }

    /// Dispatch an agent and interpret its effects until completion.
    pub async fn run(
        &self,
        agent: AgentId,
        input: OperatorInput,
    ) -> Result<ExecutionTrace, KitError> {
        let mut trace = ExecutionTrace::new();
        let mut queue: Vec<(AgentId, OperatorInput)> = vec![(agent, input)];
        let mut followups_executed = 0usize;

        while let Some((agent_id, agent_input)) = queue.pop() {
            trace.events.push(ExecutionEvent::Dispatched {
                agent: agent_id.clone(),
            });
            let output = self.orch.dispatch(&agent_id, agent_input).await?;

            // Interpret effects into state updates + followups.
            let mut followups: Vec<(AgentId, OperatorInput)> = vec![];
            for effect in &output.effects {
                // For signals, we want the orchestrator call to be owned here so
                // products can override executor behavior without losing transport.
                if let Effect::Signal { target, payload } = effect {
                    self.orch.signal(target, payload.clone()).await?;
                }
                self.effects
                    .execute_effect(effect, &mut followups, &mut trace)
                    .await?;
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
