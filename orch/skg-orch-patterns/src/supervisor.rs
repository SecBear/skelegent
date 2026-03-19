//! [`SupervisorOperator`] — LLM-driven routing via a pluggable [`SpeakerSelector`].
//!
//! The supervisor dispatches to one sub-agent at a time, chosen by the selector.
//! After each round it checks the exit reason:
//!
//! * [`ExitReason::Complete`] — task done, return the output.
//! * [`ExitReason::HandedOff`] — another agent should continue; the selector picks
//!   the next speaker and the round proceeds.
//! * Any other exit — propagated immediately (budget, error, timeout, …).
//!
//! If `max_rounds` is reached without a `Complete` exit the operator returns
//! [`ExitReason::MaxTurns`]. Effects from all rounds are accumulated.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::error::OperatorError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};

use crate::selector::SpeakerSelector;

static SUP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = SUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("sup-{n}"))
}

/// An operator that routes work to sub-agents using a [`SpeakerSelector`].
///
/// The supervisor picks the first speaker, dispatches the input, and after
/// each `HandedOff` exit uses the selector again to pick the next speaker.
/// Routing policy (round-robin, random, LLM-ranked, …) lives entirely in the
/// selector — the supervisor itself has no routing opinion.
///
/// # Termination
/// * Returns immediately when any agent exits [`ExitReason::Complete`].
/// * Returns [`ExitReason::MaxTurns`] if `max_rounds` is exhausted.
/// * Propagates any other exit reason from the sub-agent unchanged.
pub struct SupervisorOperator {
    /// The sub-operators this supervisor can delegate to.
    agents: Vec<OperatorId>,
    /// Dispatcher to dispatch sub-operators.
    dispatcher: Arc<dyn Dispatcher>,
    /// Selects which agent speaks next.
    selector: Arc<dyn SpeakerSelector>,
    /// Maximum delegation rounds before forcing completion.
    max_rounds: u32,
}

impl SupervisorOperator {
    /// Create a new supervisor with a fixed agent pool, dispatcher, selector,
    /// and round cap.
    ///
    /// `max_rounds` is a hard safety cap; the operator exits with
    /// [`ExitReason::MaxTurns`] if no agent completes within that many rounds.
    pub fn new(
        agents: Vec<OperatorId>,
        dispatcher: Arc<dyn Dispatcher>,
        selector: Arc<dyn SpeakerSelector>,
        max_rounds: u32,
    ) -> Self {
        Self {
            agents,
            dispatcher,
            selector,
            max_rounds,
        }
    }
}

#[async_trait]
impl Operator for SupervisorOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut current_input = input;
        // Accumulated effects from every round.
        let mut all_effects: Vec<layer0::Effect> = Vec::new();
        // Conversation history passed to the selector so it can make
        // context-aware routing decisions.
        let mut history: Vec<Content> = Vec::new();

        for _ in 0..self.max_rounds {
            let agent_id = self
                .selector
                .select(&self.agents, &history, ctx)
                .await
                .map_err(|e| OperatorError::non_retryable(e.to_string()))?;

            let child_ctx = ctx.child(next_dispatch_id(), agent_id);

            let mut output = self
                .dispatcher
                .dispatch(&child_ctx, current_input)
                .await
                .map_err(|e| OperatorError::non_retryable(e.to_string()))?
                .collect()
                .await
                .map_err(|e| OperatorError::non_retryable(e.to_string()))?;

            // Absorb effects before examining the exit reason.
            all_effects.append(&mut output.effects);

            match output.exit_reason {
                ExitReason::Complete => {
                    output.effects = all_effects;
                    return Ok(output);
                }
                ExitReason::HandedOff => {
                    // Record this round's output in the history so the selector
                    // can factor it into the next routing decision.
                    history.push(output.message.clone());
                    current_input =
                        OperatorInput::new(output.message.clone(), TriggerType::Task);
                }
                // Unexpected exit (budget, timeout, error, …) — surface immediately.
                _ => {
                    output.effects = all_effects;
                    return Ok(output);
                }
            }
        }

        // Exceeded max_rounds without completing.
        let mut timeout_output =
            OperatorOutput::new(current_input.message.clone(), ExitReason::MaxTurns);
        timeout_output.effects = all_effects;
        Ok(timeout_output)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::effect::Effect;
    use layer0::error::OperatorError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};
    use layer0::{DispatchContext, ExitReason as ER};
    use skg_orch_local::LocalOrch;

    use super::*;
    use crate::selector::RoundRobinSelector;

    fn test_ctx(name: &str) -> DispatchContext {
        DispatchContext::new(DispatchId::new(name), OperatorId::new(name))
    }

    fn simple_input(msg: &str) -> OperatorInput {
        OperatorInput::new(Content::text(msg), TriggerType::User)
    }

    /// Completes immediately with a fixed message.
    struct CompleteOp {
        reply: String,
    }

    #[async_trait]
    impl Operator for CompleteOp {
        async fn execute(
            &self,
            _input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            Ok(OperatorOutput::new(
                Content::text(self.reply.clone()),
                ExitReason::Complete,
            ))
        }
    }

    /// Emits `Effect::Handoff` and exits with `HandedOff`.
    ///
    /// Carries a `target` ID so the swarm tests can inspect transition logic,
    /// but the supervisor ignores the handoff target — it uses the selector.
    struct HandoffOp {
        target: OperatorId,
        reply: String,
    }

    #[async_trait]
    impl Operator for HandoffOp {
        async fn execute(
            &self,
            _input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            let mut out = OperatorOutput::new(
                Content::text(self.reply.clone()),
                ExitReason::HandedOff,
            );
            out.effects.push(Effect::Handoff {
                operator: self.target.clone(),
                state: serde_json::Value::Null,
            });
            Ok(out)
        }
    }

    /// RoundRobinSelector picks agents A then B; A hands off, B completes.
    /// Final output must come from B.
    #[tokio::test]
    async fn supervisor_delegates_via_selector() {
        let mut orch = LocalOrch::new();
        // Agent A does a handoff; agent B completes.
        orch.register(
            OperatorId::new("agent-a"),
            Arc::new(HandoffOp {
                target: OperatorId::new("agent-b"),
                reply: "from-a".into(),
            }),
        );
        orch.register(
            OperatorId::new("agent-b"),
            Arc::new(CompleteOp {
                reply: "from-b".into(),
            }),
        );
        let orch = Arc::new(orch);

        let supervisor = SupervisorOperator::new(
            vec![OperatorId::new("agent-a"), OperatorId::new("agent-b")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Arc::new(RoundRobinSelector::new()),
            5,
        );

        let output = supervisor
            .execute(simple_input("start"), &test_ctx("sup"))
            .await
            .expect("supervisor should complete");

        assert_eq!(output.exit_reason, ER::Complete);
        assert_eq!(
            output.message.as_text().unwrap_or(""),
            "from-b",
            "final output must come from agent B"
        );
        // Effects from both rounds should be present: A emits one Handoff effect.
        assert_eq!(
            output.effects.len(),
            1,
            "one Effect::Handoff from agent A should be accumulated"
        );
    }

    /// Agent always hands off; supervisor must stop after max_rounds.
    #[tokio::test]
    async fn supervisor_respects_max_rounds() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("looping-agent"),
            Arc::new(HandoffOp {
                target: OperatorId::new("looping-agent"),
                reply: "loop".into(),
            }),
        );
        let orch = Arc::new(orch);

        let supervisor = SupervisorOperator::new(
            vec![OperatorId::new("looping-agent")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Arc::new(RoundRobinSelector::new()),
            3, // must stop here
        );

        let output = supervisor
            .execute(simple_input("go"), &test_ctx("sup-limit"))
            .await
            .expect("supervisor should return max turns");

        assert_eq!(
            output.exit_reason,
            ER::MaxTurns,
            "must exit with MaxTurns after 3 rounds"
        );
        // 3 rounds × 1 Handoff effect each.
        assert_eq!(output.effects.len(), 3);
    }
}
