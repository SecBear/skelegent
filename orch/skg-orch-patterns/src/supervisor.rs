//! [`SupervisorOperator`] — LLM-driven routing via a pluggable [`SpeakerSelector`].
//!
//! The supervisor dispatches to one sub-agent at a time, chosen by the selector.
//! After each round it checks the outcome:
//!
//! * `Outcome::Terminal { Completed }` — task done, return the output.
//! * `Outcome::Transfer { HandedOff }` — another agent should continue; the selector picks
//!   the next speaker and the round proceeds.
//! * Any other outcome — propagated immediately (budget, error, timeout, …).
//!
//! If `max_rounds` is reached without a completed terminal outcome the operator returns
//! `Outcome::Limited { MaxTurns }`. Effects from all rounds are accumulated.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::context::{Message, Role};
use layer0::dispatch::Dispatcher;
use layer0::effect::{EffectKind, HandoffContext};
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{
    LimitReason, Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome,
    TransferOutcome, TriggerType,
};

use crate::selector::SpeakerSelector;

static SUP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = SUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("sup-{n}"))
}

/// An operator that routes work to sub-agents using a [`SpeakerSelector`].
///
/// The supervisor picks the first speaker via the selector, then dispatches
/// the input. After each `HandedOff` exit it determines the next speaker
/// using the following hybrid rule:
///
/// 1. If the last [`EffectKind::Handoff`] effect names an `operator` that is
///    present in `agents`, that operator is dispatched directly —
///    bypassing the selector.
/// 2. Otherwise the selector is consulted to choose the next speaker.
///
/// Routing policy (round-robin, random, LLM-ranked, …) lives in the
/// selector; explicit targets in `Handoff` effects take precedence.
///
/// # Termination
/// * Returns immediately when any agent exits with `Outcome::Terminal { Completed }`.
/// * Returns `Outcome::Limited { MaxTurns }` if `max_rounds` is exhausted.
/// * Propagates any other outcome from the sub-agent unchanged.
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
    /// `Outcome::Limited { MaxTurns }` if no agent completes within that many rounds.
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
    ) -> Result<OperatorOutput, ProtocolError> {
        let mut current_input = input;
        // Accumulated effects from every round.
        let mut all_effects: Vec<layer0::Effect> = Vec::new();
        // Conversation history passed to the selector for context-aware routing.
        // Messages carry role + content so selectors can implement attribution-based
        // routing (e.g., "don't repeat last speaker", "route after user turn").
        let mut history: Vec<Message> = Vec::new();
        // Explicit handoff target from the previous round, if any.
        // Set in the HandedOff arm; consumed (and reset) at the top of each loop.
        let mut explicit_next: Option<OperatorId> = None;

        for _ in 0..self.max_rounds {
            // Hybrid routing: honour an explicit registered target from the last
            // Handoff effect; fall back to the selector otherwise.
            let agent_id = match explicit_next.take().filter(|t| self.agents.contains(t)) {
                Some(target) => target,
                None => self
                    .selector
                    .select(&self.agents, &history, ctx)
                    .await
                    .map_err(|e| ProtocolError::internal(e.to_string()))?,
            };
            let child_ctx = ctx.child(next_dispatch_id(), agent_id);

            let mut output = self
                .dispatcher
                .dispatch(&child_ctx, current_input)
                .await?
                .collect()
                .await?;

            // Extract the target operator and structured context from the last Handoff
            // effect BEFORE draining them, so we can use context.task as next input.
            let handoff: Option<(OperatorId, HandoffContext)> =
                output.effects.iter().rev().find_map(|e| {
                    if let EffectKind::Handoff {
                        ref operator,
                        ref context,
                    } = e.kind
                    {
                        Some((operator.clone(), context.clone()))
                    } else {
                        None
                    }
                });
            let handoff_operator = handoff.as_ref().map(|(op, _)| op.clone());
            let handoff_ctx = handoff.map(|(_, hctx)| hctx);

            // Absorb effects before examining the outcome.
            all_effects.append(&mut output.effects);

            match &output.outcome {
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed,
                } => {
                    output.effects = all_effects;
                    return Ok(output);
                }
                Outcome::Transfer {
                    transfer: TransferOutcome::HandedOff,
                } => {
                    // Use context.task as the next input — it is the task the
                    // handing-off operator wants the next speaker to act on.
                    // Fall back to output.message if no Handoff effect was emitted.
                    let next_task = handoff_ctx
                        .as_ref()
                        .map(|c| c.task.clone())
                        .unwrap_or_else(|| output.message.clone());
                    // Record as a User-role message so selectors see role + content.
                    history.push(Message::new(Role::User, next_task.clone()));
                    let mut next_input = OperatorInput::new(next_task, TriggerType::Task);
                    // Forward conversation history if the handing-off operator supplied it.
                    if let Some(hist) = handoff_ctx.and_then(|c| c.history) {
                        next_input.context = Some(hist);
                    }
                    current_input = next_input;
                    // Carry the explicit target into the next iteration.
                    // It is consumed (and reset) at the top of the loop.
                    explicit_next = handoff_operator;
                }
                // Unexpected exit (budget, timeout, error, …) — surface immediately.
                _ => {
                    output.effects = all_effects;
                    return Ok(output);
                }
            }
        }

        // Exceeded max_rounds without completing.
        let mut timeout_output = OperatorOutput::new(
            current_input.message.clone(),
            Outcome::Limited {
                limit: LimitReason::MaxTurns,
            },
        );
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
    use layer0::DispatchContext;
    use layer0::content::Content;
    use layer0::effect::{Effect, EffectKind, HandoffContext};
    use layer0::error::ProtocolError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{
        Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TransferOutcome,
        TriggerType,
    };
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
        ) -> Result<OperatorOutput, ProtocolError> {
            Ok(OperatorOutput::new(
                Content::text(self.reply.clone()),
                Outcome::Terminal {
                    terminal: TerminalOutcome::Completed,
                },
            ))
        }
    }

    /// Emits `Effect::Handoff` with a `HandoffContext` and exits with `HandedOff`.
    ///
    /// `task` is the explicit next-step message the handing-off operator supplies.
    /// Carries a `target` ID so the swarm tests can inspect transition logic,
    /// but the supervisor honours the target when it names a registered agent.
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
        ) -> Result<OperatorOutput, ProtocolError> {
            let mut out = OperatorOutput::new(
                Content::text(self.reply.clone()),
                Outcome::Transfer {
                    transfer: TransferOutcome::HandedOff,
                },
            );
            let next_task = if self.task.is_empty() {
                self.reply.clone()
            } else {
                self.task.clone()
            };
            out.effects.push(Effect::new(EffectKind::Handoff {
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

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
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
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::MaxTurns
            },
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
            ) -> Result<OperatorOutput, ProtocolError> {
                let mut out = OperatorOutput::new(
                    Content::text("output-message-not-the-task"),
                    Outcome::Transfer {
                        transfer: TransferOutcome::HandedOff,
                    },
                );
                out.effects.push(Effect::new(EffectKind::Handoff {
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
            ) -> Result<OperatorOutput, ProtocolError> {
                *self.received.lock().unwrap() = input.message.as_text().unwrap_or("").to_string();
                Ok(OperatorOutput::new(
                    Content::text("done"),
                    Outcome::Terminal {
                        terminal: TerminalOutcome::Completed,
                    },
                ))
            }
        }

        let received = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("sender"), Arc::new(ExplicitTaskHandoffOp));
        orch.register(
            OperatorId::new("receiver"),
            Arc::new(RecordingOp {
                received: received.clone(),
            }),
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

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(
            *received.lock().unwrap(),
            "explicit-task-for-receiver",
            "receiver must get context.task, not output.message"
        );
    }
    /// When a `Handoff` effect names a registered agent, the supervisor routes
    /// to that agent directly — bypassing the selector.
    ///
    /// Setup: `FixedSelector` always returns `agent-a`. `agent-a` emits a
    /// `Handoff` targeting `agent-b`. If the selector were consulted again it
    /// would keep returning `agent-a` and the run would exhaust `max_rounds`.
    /// Correct behaviour: `agent-b` is dispatched on round 2 and completes.
    #[tokio::test]
    async fn supervisor_honors_explicit_handoff_target() {
        struct FixedSelector(OperatorId);
        #[async_trait]
        impl SpeakerSelector for FixedSelector {
            async fn select(
                &self,
                _candidates: &[OperatorId],
                _history: &[Message],
                _ctx: &DispatchContext,
            ) -> Result<OperatorId, crate::selector::SelectorError> {
                Ok(self.0.clone())
            }
        }

        let mut orch = LocalOrch::new();
        // agent-a hands off to agent-b explicitly.
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

        // Selector always returns agent-a — if it were consulted after the
        // handoff the run would never complete within max_rounds.
        let supervisor = SupervisorOperator::new(
            vec![OperatorId::new("agent-a"), OperatorId::new("agent-b")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Arc::new(FixedSelector(OperatorId::new("agent-a"))),
            3,
        );

        let output = supervisor
            .execute(simple_input("start"), &test_ctx("sup-explicit"))
            .await
            .expect("supervisor should complete");

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            },
            "must complete, not exhaust rounds"
        );
        assert_eq!(
            output.message.as_text().unwrap_or(""),
            "from-b",
            "output must come from agent-b (explicit handoff target)"
        );
    }

    /// When a `Handoff` effect names an operator that is NOT in the registered
    /// agent pool, the supervisor falls back to the selector.
    ///
    /// Setup: `agent-a` emits a `Handoff` targeting `ghost-agent` (unregistered).
    /// Round-robin selector gives `agent-a` first, then `agent-b`. Because
    /// `ghost-agent` is not in agents, the selector is consulted for round 2
    /// and picks `agent-b`, which completes.
    #[tokio::test]
    async fn supervisor_uses_selector_when_no_target() {
        let mut orch = LocalOrch::new();
        // agent-a hands off to an unregistered target.
        orch.register(
            OperatorId::new("agent-a"),
            Arc::new(HandoffOp {
                target: OperatorId::new("ghost-agent"), // not in agents list
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

        // Round-robin: first call → agent-a, second call → agent-b.
        let supervisor = SupervisorOperator::new(
            vec![OperatorId::new("agent-a"), OperatorId::new("agent-b")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Arc::new(RoundRobinSelector::new()),
            3,
        );

        let output = supervisor
            .execute(simple_input("start"), &test_ctx("sup-selector-fallback"))
            .await
            .expect("supervisor should complete");

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(
            output.message.as_text().unwrap_or(""),
            "from-b",
            "selector fallback must route to agent-b"
        );
    }
}
