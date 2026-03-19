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
use layer0::effect::{EffectKind, HandoffContext};
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

            // Extract the handoff context from this round's effects BEFORE
            // draining them, so we can use context.task as the next input.
            let handoff_ctx: Option<HandoffContext> = output.effects.iter().rev().find_map(|e| {
                if let EffectKind::Handoff { ref context, .. } = e.kind {
                    Some(context.clone())
                } else {
                    None
                }
            });

            // Absorb effects before examining the exit reason.
            all_effects.append(&mut output.effects);

            match output.exit_reason {
                ExitReason::Complete => {
                    output.effects = all_effects;
                    return Ok(output);
                }
                ExitReason::HandedOff => {
                    // Use context.task as the next input — it is the task the
                    // handing-off operator wants the next speaker to act on.
                    // Fall back to output.message if no Handoff effect was emitted.
                    let next_task = handoff_ctx
                        .as_ref()
                        .map(|c| c.task.clone())
                        .unwrap_or_else(|| output.message.clone());
                    history.push(next_task.clone());
                    let mut next_input = OperatorInput::new(next_task, TriggerType::Task);
                    // Forward conversation history if the handing-off operator supplied it.
                    if let Some(hist) = handoff_ctx.and_then(|c| c.history) {
                        next_input.context = Some(hist);
                    }
                    current_input = next_input;
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
    use layer0::effect::{Effect, EffectKind, HandoffContext};
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

    /// Emits `Effect::Handoff` with a `HandoffContext` and exits with `HandedOff`.
    ///
    /// `task` is the explicit next-step message the handing-off operator supplies.
    /// Carries a `target` ID so the swarm tests can inspect transition logic,
    /// but the supervisor ignores the handoff target — it uses the selector.
    struct HandoffOp {
        target: OperatorId,
        reply: String,
        /// Task to pass in HandoffContext (defaults to reply if empty).
        task: String,
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
            let next_task = if self.task.is_empty() {
                self.reply.clone()
            } else {
                self.task.clone()
            };
            out.effects.push(Effect::new(0, EffectKind::Handoff {
                operator: self.target.clone(),
                context: HandoffContext {
                    task: Content::text(next_task),
                    history: None,
                    metadata: None,
                },
            }));
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
                task: "".into(),
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
                task: "".into(),
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

    /// Verifies that `context.task` from `HandoffContext` flows through as the
    /// next operator's input — NOT `output.message`.
    #[tokio::test]
    async fn handoff_context_preserves_task() {
        /// Operator that hands off with an explicit `task` distinct from its reply.
        struct ExplicitTaskHandoffOp;

        #[async_trait]
        impl Operator for ExplicitTaskHandoffOp {
            async fn execute(
                &self,
                _input: OperatorInput,
                _ctx: &DispatchContext,
            ) -> Result<OperatorOutput, OperatorError> {
                let mut out = OperatorOutput::new(
                    Content::text("output-message-not-the-task"),
                    ExitReason::HandedOff,
                );
                out.effects.push(Effect::new(0, EffectKind::Handoff {
                    operator: OperatorId::new("receiver"),
                    context: HandoffContext {
                        task: Content::text("explicit-task-for-receiver"),
                        history: None,
                        metadata: None,
                    },
                }));
                Ok(out)
            }
        }

        /// Operator that records the input it received.
        struct RecordingOp {
            received: std::sync::Arc<std::sync::Mutex<String>>,
        }

        #[async_trait]
        impl Operator for RecordingOp {
            async fn execute(
                &self,
                input: OperatorInput,
                _ctx: &DispatchContext,
            ) -> Result<OperatorOutput, OperatorError> {
                *self.received.lock().unwrap() =
                    input.message.as_text().unwrap_or("").to_string();
                Ok(OperatorOutput::new(
                    Content::text("done"),
                    ExitReason::Complete,
                ))
            }
        }

        let received = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("sender"), Arc::new(ExplicitTaskHandoffOp));
        orch.register(
            OperatorId::new("receiver"),
            Arc::new(RecordingOp { received: received.clone() }),
        );
        let orch = Arc::new(orch);

        // Round-robin: sender first, receiver second.
        let supervisor = SupervisorOperator::new(
            vec![OperatorId::new("sender"), OperatorId::new("receiver")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Arc::new(RoundRobinSelector::new()),
            3,
        );

        let output = supervisor
            .execute(simple_input("start"), &test_ctx("sup-task"))
            .await
            .expect("supervisor must complete");

        assert_eq!(output.exit_reason, ER::Complete);
        assert_eq!(
            *received.lock().unwrap(),
            "explicit-task-for-receiver",
            "receiver must get context.task, not output.message"
        );
    }
}
