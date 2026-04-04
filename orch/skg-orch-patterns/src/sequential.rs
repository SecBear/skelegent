//! [`SequentialOperator`] — pipeline of operators where each output feeds the next.
//!
//! Dispatches operators in order. After each step the output content becomes
//! the input message for the next step. Effects from every step are aggregated.
//! If any step exits with a non-`Complete` reason the pipeline stops early and
//! returns that step's output with all accumulated effects attached.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{
    Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TriggerType,
};

static SEQ_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = SEQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("seq-{n}"))
}

/// A pipeline of operators executed in order, each output feeding the next input.
///
/// All effects are accumulated across steps. The `Outcome` of the last step
/// that ran is used as the pipeline's outcome. If any step exits with
/// something other than `Outcome::Terminal { terminal: TerminalOutcome::Completed }`
/// the pipeline halts immediately and returns that step's output (with all
/// prior + current effects attached).
pub struct SequentialOperator {
    /// Ordered operator IDs forming the pipeline.
    steps: Vec<OperatorId>,
    /// Dispatcher used to invoke each step.
    dispatcher: Arc<dyn Dispatcher>,
}

impl SequentialOperator {
    /// Create a sequential pipeline from an ordered list of operator IDs.
    pub fn new(steps: Vec<OperatorId>, dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self { steps, dispatcher }
    }
}

#[async_trait]
impl Operator for SequentialOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let mut current_input = input;
        // Accumulate effects from all steps as we go.
        let mut all_effects: Vec<layer0::Intent> = Vec::new();
        let mut last_output: Option<OperatorOutput> = None;

        for step_id in &self.steps {
            let child_ctx = ctx.child(next_dispatch_id(), step_id.clone());

            let mut output = self
                .dispatcher
                .dispatch(&child_ctx, current_input)
                .await?
                .collect()
                .await?;

            // Absorb this step's effects into the running total.
            all_effects.append(&mut output.intents);

            let is_completed = matches!(
                output.outcome,
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed
                }
            );
            if !is_completed {
                // Stop and surface the failure; attach all accumulated effects.
                output.intents = all_effects;
                return Ok(output);
            }

            // Advance: next step gets this step's message as input.
            current_input = OperatorInput::new(output.message.clone(), TriggerType::Task);
            last_output = Some(output);
        }

        // All steps completed normally.
        match last_output {
            Some(mut output) => {
                output.intents = all_effects;
                Ok(output)
            }
            // Empty pipeline: return a no-op completion.
            None => Ok(OperatorOutput::new(
                Content::text(""),
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed,
                },
            )),
        }
    }
}
