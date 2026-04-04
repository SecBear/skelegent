//! [`WorkflowBuilder`] — fluent API that compiles to pattern operators.
//!
//! Lets callers describe a workflow declaratively and receive an
//! [`Arc<dyn Operator>`] that executes it. Steps are compiled into a chain of
//! pattern operators; a single-step workflow short-circuits to that pattern
//! directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::Dispatcher;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{
    Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TriggerType,
};

use crate::loop_op::LoopOperator;
use crate::parallel::{ParallelOperator, ReducerFn};

static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("pipe-{n}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal step descriptor
// ─────────────────────────────────────────────────────────────────────────────

/// Internal representation of one workflow step before compilation.
enum StepDesc {
    /// Single operator dispatch.
    Single(OperatorId),
    /// Fan-out to multiple operators with a custom reducer.
    Parallel {
        /// Operator IDs to fan out to.
        operators: Vec<OperatorId>,
        /// How to combine branch outputs.
        reducer: ReducerFn,
    },
    /// Loop an operator until a predicate fires.
    Loop {
        /// The body operator.
        body: OperatorId,
        /// Hard iteration cap.
        max: u32,
        /// Termination predicate.
        done: Box<dyn Fn(&OperatorOutput) -> bool + Send + Sync + 'static>,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipeline: internal chain of Arc<dyn Operator>
// ─────────────────────────────────────────────────────────────────────────────

/// A compiled sequential chain of operators held by value (not by ID).
///
/// Unlike [`SequentialOperator`][crate::sequential::SequentialOperator], which
/// routes by ID through a dispatcher, this struct holds `Arc<dyn Operator>`
/// directly. This lets the builder produce mixed pipelines (single + parallel
/// + loop) without requiring all steps to be registered in the dispatcher.
struct Pipeline {
    steps: Vec<Arc<dyn Operator>>,
}

#[async_trait]
impl Operator for Pipeline {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let mut current_input = input;
        let mut all_effects: Vec<layer0::Intent> = Vec::new();
        let mut last_output: Option<OperatorOutput> = None;

        for step in &self.steps {
            let mut output = step.execute(current_input, ctx).await?;

            all_effects.append(&mut output.intents);

            let is_completed = matches!(
                output.outcome,
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed
                }
            );
            if !is_completed {
                output.intents = all_effects;
                return Ok(output);
            }

            current_input = OperatorInput::new(output.message.clone(), TriggerType::Task);
            last_output = Some(output);
        }

        match last_output {
            Some(mut out) => {
                out.intents = all_effects;
                Ok(out)
            }
            None => Ok(OperatorOutput::new(
                layer0::Content::text(""),
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed,
                },
            )),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SingleDispatch: wraps one OperatorId as Arc<dyn Operator>
// ─────────────────────────────────────────────────────────────────────────────

/// Thin adapter that dispatches a single operator by ID.
///
/// Used by `Pipeline` to represent a `step()` call in a mixed workflow.
struct SingleDispatch {
    id: OperatorId,
    dispatcher: Arc<dyn Dispatcher>,
}

#[async_trait]
impl Operator for SingleDispatch {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let child_ctx = ctx.child(next_dispatch_id(), self.id.clone());
        self.dispatcher
            .dispatch(&child_ctx, input)
            .await?
            .collect()
            .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WorkflowBuilder
// ─────────────────────────────────────────────────────────────────────────────

/// Fluent builder that compiles a sequence of steps into a pattern operator.
///
/// # Example
/// ```rust,ignore
/// let op = WorkflowBuilder::new(Arc::clone(&dispatcher))
///     .step(OperatorId::new("fetch"))
///     .step(OperatorId::new("summarise"))
///     .parallel(vec![OperatorId::new("translate-en"), OperatorId::new("translate-fr")])
///     .build();
/// ```
pub struct WorkflowBuilder {
    dispatcher: Arc<dyn Dispatcher>,
    steps: Vec<StepDesc>,
}

impl WorkflowBuilder {
    /// Create a new builder backed by `dispatcher`.
    pub fn new(dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            dispatcher,
            steps: Vec::new(),
        }
    }

    /// Append a single sequential step.
    pub fn step(mut self, operator: OperatorId) -> Self {
        self.steps.push(StepDesc::Single(operator));
        self
    }

    /// Append a parallel fan-out step using the default reducer.
    pub fn parallel(mut self, operators: Vec<OperatorId>) -> Self {
        self.steps.push(StepDesc::Parallel {
            operators,
            reducer: Box::new(crate::parallel::default_reducer),
        });
        self
    }

    /// Append a parallel fan-out step with a custom reducer.
    pub fn parallel_with_reducer(mut self, operators: Vec<OperatorId>, reducer: ReducerFn) -> Self {
        self.steps.push(StepDesc::Parallel { operators, reducer });
        self
    }

    /// Append a loop step that repeats `body` until `done` fires or `max`
    /// iterations are exhausted.
    pub fn loop_until(
        mut self,
        body: OperatorId,
        max: u32,
        done: impl Fn(&OperatorOutput) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.steps.push(StepDesc::Loop {
            body,
            max,
            done: Box::new(done),
        });
        self
    }

    /// Compile all steps into a single [`Arc<dyn Operator>`].
    ///
    /// - 0 steps → no-op operator that returns the input unchanged.
    /// - 1 step of any kind → the compiled step directly (no wrapper).
    /// - ≥2 steps → [`Pipeline`] chaining them in order.
    pub fn build(self) -> Arc<dyn Operator> {
        let dispatcher = self.dispatcher;

        // Convert each StepDesc into Arc<dyn Operator>.
        let mut ops: Vec<Arc<dyn Operator>> =
            self.steps
                .into_iter()
                .map(|desc| -> Arc<dyn Operator> {
                    match desc {
                        StepDesc::Single(id) => Arc::new(SingleDispatch {
                            id,
                            dispatcher: Arc::clone(&dispatcher),
                        }),
                        StepDesc::Parallel { operators, reducer } => Arc::new(
                            ParallelOperator::new(operators, Arc::clone(&dispatcher), reducer),
                        ),
                        StepDesc::Loop { body, max, done } => {
                            Arc::new(LoopOperator::new(body, Arc::clone(&dispatcher), max, done))
                        }
                    }
                })
                .collect();

        match ops.len() {
            0 => Arc::new(NoOpOperator),
            1 => ops.remove(0),
            _ => Arc::new(Pipeline { steps: ops }),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NoOpOperator: zero-step fallback
// ─────────────────────────────────────────────────────────────────────────────

/// Returned by `WorkflowBuilder::build()` when no steps were added.
struct NoOpOperator;

#[async_trait]
impl Operator for NoOpOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        Ok(OperatorOutput::new(
            input.message,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            },
        ))
    }
}
