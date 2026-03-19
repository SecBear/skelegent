//! [`LoopOperator`] — repeat an operator until a predicate returns true.
//!
//! Each iteration's output content becomes the next iteration's input. The
//! loop terminates when `done` returns `true` or `max_iterations` is reached
//! (returning [`ExitReason::MaxTurns`] in the latter case).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::Dispatcher;
use layer0::error::OperatorError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};

static LOOP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = LOOP_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("loop-{n}"))
}

/// Repeat `body` until `done` returns `true` or `max_iterations` is reached.
///
/// On each iteration the body receives the previous output as its input message.
/// Effects from all iterations are accumulated in the final output. If the body
/// exits with a non-`Complete` reason on any iteration that output is returned
/// immediately (with accumulated effects).
pub struct LoopOperator {
    /// The operator that runs each iteration.
    body: OperatorId,
    /// Dispatcher used to invoke the body.
    dispatcher: Arc<dyn Dispatcher>,
    /// Hard cap on iterations. When reached the output has
    /// [`ExitReason::MaxTurns`].
    max_iterations: u32,
    /// Termination predicate: receives the last output and returns `true` when
    /// the loop should stop.
    done: Box<dyn Fn(&OperatorOutput) -> bool + Send + Sync>,
}

impl LoopOperator {
    /// Create a loop operator.
    ///
    /// `max_iterations` is a hard safety cap — the loop exits with
    /// [`ExitReason::MaxTurns`] if the predicate never fires.
    pub fn new(
        body: OperatorId,
        dispatcher: Arc<dyn Dispatcher>,
        max_iterations: u32,
        done: impl Fn(&OperatorOutput) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            body,
            dispatcher,
            max_iterations,
            done: Box::new(done),
        }
    }
}

#[async_trait]
impl Operator for LoopOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut current_input = input;
        let mut all_effects: Vec<layer0::Effect> = Vec::new();

        for _ in 0..self.max_iterations {
            let child_ctx = ctx.child(next_dispatch_id(), self.body.clone());

            let mut output = self
                .dispatcher
                .dispatch(&child_ctx, current_input)
                .await
                .map_err(|e| OperatorError::non_retryable(e.to_string()))?
                .collect()
                .await
                .map_err(|e| OperatorError::non_retryable(e.to_string()))?;

            // Absorb effects before checking exit conditions.
            all_effects.append(&mut output.effects);

            // Propagate unexpected exits immediately.
            if output.exit_reason != ExitReason::Complete {
                output.effects = all_effects;
                return Ok(output);
            }

            // Check the done predicate.
            if (self.done)(&output) {
                output.effects = all_effects;
                return Ok(output);
            }

            // Feed output back as the next iteration's input.
            current_input = OperatorInput::new(output.message.clone(), TriggerType::Task);
        }

        // Exhausted iterations without the predicate firing.
        let mut timeout_output =
            OperatorOutput::new(current_input.message.clone(), ExitReason::MaxTurns);
        timeout_output.effects = all_effects;
        Ok(timeout_output)
    }
}
