use std::collections::VecDeque;

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::Dispatcher;
use layer0::effect::{Effect, EffectKind};
use layer0::error::ProtocolError;
use layer0::id::DispatchId;
use layer0::id::{OperatorId, WorkflowId};
use layer0::operator::{OperatorInput, OperatorOutput};
use skg_effects_core::{EffectHandler, EffectOutcome};
use std::sync::Arc;
use thiserror::Error;

// ── Effect middleware (local definitions) ───────────────────────────────────

/// Action returned by effect middleware.
pub enum EffectAction {
    /// Continue processing with a (possibly modified) effect.
    Continue(Box<Effect>),
    /// Skip this effect entirely.
    Skip,
}

/// Middleware that can intercept and modify effects before execution.
#[async_trait]
pub trait EffectMiddleware: Send + Sync {
    /// Process an effect before it reaches the handler.
    async fn on_effect(&self, effect: Effect, ctx: &DispatchContext) -> EffectAction;
}

/// Stack of effect middleware layers.
pub struct EffectStack {
    layers: Vec<Box<dyn EffectMiddleware>>,
}

impl EffectStack {
    /// Create a new empty middleware stack.
    pub fn new() -> Self {
        Self { layers: vec![] }
    }

    /// Push a middleware layer onto the stack.
    pub fn push(mut self, mw: impl EffectMiddleware + 'static) -> Self {
        self.layers.push(Box::new(mw));
        self
    }

    /// Process an effect through all layers. Returns `None` if any layer skips.
    pub async fn process(&self, mut effect: Effect, ctx: &DispatchContext) -> Option<Effect> {
        for layer in &self.layers {
            match layer.on_effect(effect, ctx).await {
                EffectAction::Continue(e) => effect = *e,
                EffectAction::Skip => return None,
            }
        }
        Some(effect)
    }
}

/// Errors returned by `skg-orch-kit`.
#[derive(Debug, Error)]
pub enum KitError {
    /// Dispatch error.
    #[error("orchestrator error: {0}")]
    Dispatch(#[from] ProtocolError),
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
    /// Optional middleware stack applied to every effect before execution.
    ///
    /// Layers run in insertion order. If any layer returns
    /// [`layer0::EffectAction::Skip`], the effect is not executed and the
    /// handler never sees it. This is the composition point for
    /// [`layer0::LoggingEffectMiddleware`] and custom policies.
    effect_middleware: Option<EffectStack>,
}

impl OrchestratedRunner {
    /// Create a new orchestrated runner.
    pub fn new(dispatcher: Arc<dyn Dispatcher>, effects: Arc<dyn EffectHandler>) -> Self {
        Self {
            dispatcher,
            effects,
            max_followups: 128,
            effect_middleware: None,
        }
    }

    /// Set a safety bound on the number of follow-up dispatches.
    pub fn with_max_followups(mut self, max_followups: usize) -> Self {
        self.max_followups = max_followups;
        self
    }

    /// Attach an effect middleware stack.
    ///
    /// Every effect is passed through the stack before reaching the
    /// [`EffectHandler`]. Compose logging, validation, and audit layers here:
    ///
    /// ```rust,ignore
    /// let log = Arc::new(InMemoryEffectLog::new());
    /// let stack = EffectStack::new()
    ///     .push(LoggingEffectMiddleware::new(log.clone()));
    /// let runner = OrchestratedRunner::new(dispatcher, handler)
    ///     .with_effect_middleware(stack);
    /// ```
    pub fn with_effect_middleware(mut self, stack: EffectStack) -> Self {
        self.effect_middleware = Some(stack);
        self
    }

    /// Dispatch an operator and interpret its effects until completion.
    pub async fn run(
        &self,
        operator: OperatorId,
        input: OperatorInput,
    ) -> Result<ExecutionTrace, KitError> {
        let mut trace = ExecutionTrace::new();
        let mut queue: VecDeque<(OperatorId, OperatorInput)> = VecDeque::from([(operator, input)]);
        let mut followups_executed = 0usize;

        while let Some((op_id, op_input)) = queue.pop_front() {
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
            for raw_effect in &output.effects {
                // Pass through middleware stack if configured.
                // A Skip return suppresses this effect entirely — the handler
                // never sees it and no trace event is recorded.
                let effect = match &self.effect_middleware {
                    Some(stack) => match stack.process(raw_effect.clone(), &ctx).await {
                        Some(e) => e,
                        None => continue,
                    },
                    None => raw_effect.clone(),
                };

                match self
                    .effects
                    .handle(&effect, &ctx)
                    .await
                    .map_err(|e| KitError::Effect(e.to_string()))?
                {
                    EffectOutcome::Applied => match &effect.kind {
                        EffectKind::WriteMemory { key, .. } => {
                            trace
                                .events
                                .push(ExecutionEvent::MemoryWritten { key: key.clone() });
                        }
                        EffectKind::DeleteMemory { key, .. } => {
                            trace
                                .events
                                .push(ExecutionEvent::MemoryDeleted { key: key.clone() });
                        }
                        EffectKind::Signal { target, payload } => {
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

            // FIFO: append followups so first-emitted fires next within each batch.
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
