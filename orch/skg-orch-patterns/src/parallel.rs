//! [`ParallelOperator`] — fan-out to N operators with a configurable reducer.
//!
//! All branches are dispatched concurrently via [`tokio::task::JoinSet`]. The
//! `reducer` function combines the collected outputs into a single
//! [`OperatorOutput`]. A default reducer concatenates messages and aggregates
//! effects.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::error::OperatorError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput};

static PAR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = PAR_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("par-{n}"))
}

/// A type-erased reducer: combines a `Vec<OperatorOutput>` into one.
///
/// Provided at construction time so callers control how branch results merge.
/// The default reducer concatenates text messages and aggregates effects.
pub type ReducerFn = Box<dyn Fn(Vec<OperatorOutput>) -> OperatorOutput + Send + Sync>;

/// Default reducer: concatenate all branch messages, aggregate all effects,
/// and carry the last non-`Complete` exit reason (or `Complete` if all completed).
pub(crate) fn default_reducer(outputs: Vec<OperatorOutput>) -> OperatorOutput {
    let mut parts: Vec<String> = Vec::with_capacity(outputs.len());
    let mut all_effects = Vec::new();
    let mut exit_reason = ExitReason::Complete;

    for output in outputs {
        if let Some(text) = output.message.as_text() {
            parts.push(text.to_string());
        }
        all_effects.extend(output.effects);
        // Surface the last non-complete exit reason so callers notice failures.
        if output.exit_reason != ExitReason::Complete {
            exit_reason = output.exit_reason;
        }
    }

    let mut result = OperatorOutput::new(Content::text(parts.join("\n")), exit_reason);
    result.effects = all_effects;
    result
}

/// Fan-out to N operators running concurrently; results are combined by `reducer`.
///
/// All branches receive the same input. If any branch dispatch or execution
/// fails the whole operator returns that error (remaining tasks are cancelled
/// implicitly when `JoinSet` is dropped).
pub struct ParallelOperator {
    /// Operator IDs to dispatch in parallel.
    branches: Vec<OperatorId>,
    /// Dispatcher used to invoke each branch.
    dispatcher: Arc<dyn Dispatcher>,
    /// Combines branch outputs into a single result.
    reducer: ReducerFn,
}

impl ParallelOperator {
    /// Create a parallel operator with the default concatenating reducer.
    pub fn with_default_reducer(
        branches: Vec<OperatorId>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        Self::new(branches, dispatcher, Box::new(default_reducer))
    }

    /// Create a parallel operator with a custom reducer.
    pub fn new(
        branches: Vec<OperatorId>,
        dispatcher: Arc<dyn Dispatcher>,
        reducer: ReducerFn,
    ) -> Self {
        Self {
            branches,
            dispatcher,
            reducer,
        }
    }
}

#[async_trait]
impl Operator for ParallelOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut join_set: tokio::task::JoinSet<Result<OperatorOutput, OperatorError>> =
            tokio::task::JoinSet::new();

        for branch_id in &self.branches {
            let child_ctx = ctx.child(next_dispatch_id(), branch_id.clone());
            let dispatcher = Arc::clone(&self.dispatcher);
            // Each branch receives a clone of the original input.
            let branch_input = input.clone();

            join_set.spawn(async move {
                dispatcher
                    .dispatch(&child_ctx, branch_input)
                    .await
                    .map_err(|e| OperatorError::non_retryable(e.to_string()))?
                    .collect()
                    .await
                    .map_err(|e| OperatorError::non_retryable(e.to_string()))
            });
        }

        // Collect branch results in completion order.
        let mut outputs: Vec<OperatorOutput> = Vec::with_capacity(self.branches.len());
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(output)) => outputs.push(output),
                Ok(Err(e)) => return Err(e),
                Err(join_err) => {
                    return Err(OperatorError::non_retryable(format!(
                        "branch task panicked: {join_err}"
                    )));
                }
            }
        }

        Ok((self.reducer)(outputs))
    }
}

/// Splitter function: given one input, produces N inputs to fan out to the same operator.
///
/// An empty return is valid — the reducer receives an empty `Vec` and the
/// operator returns whatever the reducer produces from nothing.
pub type SplitterFn = Box<dyn Fn(&OperatorInput) -> Vec<OperatorInput> + Send + Sync>;

/// Fan-out to the **same** operator with N different inputs produced by `splitter`.
///
/// Contrast with [`ParallelOperator`], which fans out to N *different* operators
/// with the same input. `FanOutOperator` fans out to one operator with N inputs
/// determined at runtime from the incoming data.
///
/// All branches are dispatched concurrently via [`tokio::task::JoinSet`]. The
/// `reducer` combines the collected outputs into a single [`OperatorOutput`].
/// If any branch dispatch or execution fails the whole operator returns that
/// error (remaining tasks are cancelled when `JoinSet` is dropped).
pub struct FanOutOperator {
    /// The single operator ID all split inputs are dispatched to.
    operator: OperatorId,
    /// Dispatcher used to invoke each branch.
    dispatcher: Arc<dyn Dispatcher>,
    /// Produces N inputs from the original input.
    splitter: SplitterFn,
    /// Combines branch outputs into a single result.
    reducer: ReducerFn,
}

impl FanOutOperator {
    /// Create a `FanOutOperator` with a custom splitter and the default concatenating reducer.
    pub fn with_default_reducer(
        operator: OperatorId,
        dispatcher: Arc<dyn Dispatcher>,
        splitter: SplitterFn,
    ) -> Self {
        Self::new(operator, dispatcher, splitter, Box::new(default_reducer))
    }

    /// Create a `FanOutOperator` with a custom splitter and a custom reducer.
    pub fn new(
        operator: OperatorId,
        dispatcher: Arc<dyn Dispatcher>,
        splitter: SplitterFn,
        reducer: ReducerFn,
    ) -> Self {
        Self {
            operator,
            dispatcher,
            splitter,
            reducer,
        }
    }
}

#[async_trait]
impl Operator for FanOutOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let split_inputs = (self.splitter)(&input);
        let capacity = split_inputs.len();

        let mut join_set: tokio::task::JoinSet<Result<OperatorOutput, OperatorError>> =
            tokio::task::JoinSet::new();

        for branch_input in split_inputs {
            let child_ctx = ctx.child(next_dispatch_id(), self.operator.clone());
            let dispatcher = Arc::clone(&self.dispatcher);

            join_set.spawn(async move {
                dispatcher
                    .dispatch(&child_ctx, branch_input)
                    .await
                    .map_err(|e| OperatorError::non_retryable(e.to_string()))?
                    .collect()
                    .await
                    .map_err(|e| OperatorError::non_retryable(e.to_string()))
            });
        }

        let mut outputs: Vec<OperatorOutput> = Vec::with_capacity(capacity);
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(output)) => outputs.push(output),
                Ok(Err(e)) => return Err(e),
                Err(join_err) => {
                    return Err(OperatorError::non_retryable(format!(
                        "fan-out branch task panicked: {join_err}"
                    )));
                }
            }
        }

        Ok((self.reducer)(outputs))
    }
}
