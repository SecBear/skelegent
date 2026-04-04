//! [`SwarmOperator`] — peer-to-peer handoff with explicit transition constraints.
//!
//! Unlike [`SupervisorOperator`] which uses a central selector, a swarm lets
//! each operator declare its own successor via [`EffectKind::Handoff`]. The swarm
//! validates every transition against a pre-declared adjacency map and errors
//! hard if an undeclared transition is attempted.
//!
//! # Flow
//! 1. Dispatch the entry operator.
//! 2. If it exits [`Outcome::Transfer { transfer: TransferOutcome::HandedOff }`], find the [`EffectKind::Handoff`] target.
//! 3. Verify the transition `current → target` is allowed.
//! 4. Dispatch the target; repeat until [`Outcome::Terminal { terminal: TerminalOutcome::Completed }`] or `max_handoffs`.
//!
//! Effects from every hop are accumulated into the final output.
//!
//! # Example
//! ```rust,ignore
//! let op = SwarmOperator::builder(dispatcher)
//!     .entry(OperatorId::new("triage"))
//!     .transition(OperatorId::new("triage"), OperatorId::new("billing"))
//!     .transition(OperatorId::new("triage"), OperatorId::new("support"))
//!     .transition(OperatorId::new("billing"), OperatorId::new("support"))
//!     .max_handoffs(5)
//!     .build();
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::dispatch::Dispatcher;
use layer0::effect::{EffectKind, HandoffContext};
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{
    LimitReason, Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome,
    TransferOutcome, TriggerType,
};

static SWARM_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = SWARM_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("swarm-{n}"))
}

/// A peer-to-peer handoff operator with explicit transition constraints.
///
/// Each operator in the swarm may hand off to another by emitting
/// [`EffectKind::Handoff`] and returning [`Outcome::Transfer { transfer: TransferOutcome::HandedOff }`]. The swarm
/// validates the transition against the declared adjacency map before
/// dispatching the next operator.
///
/// # Invariants
/// * A `HandedOff` exit **must** have an accompanying `Effect::Handoff` in the
///   output effects, otherwise execution is an error.
/// * A transition not present in the adjacency map is an error; the swarm does
///   not silently allow unlisted routes.
/// * `max_handoffs` is a hard cap; reaching it returns `Outcome::Limited { MaxTurns }`.
pub struct SwarmOperator {
    /// Allowed transitions: operator → set of valid targets.
    transitions: HashMap<OperatorId, Vec<OperatorId>>,
    /// Starting operator.
    entry: OperatorId,
    /// Dispatcher.
    dispatcher: Arc<dyn Dispatcher>,
    /// Maximum handoff chain length.
    max_handoffs: u32,
}

impl SwarmOperator {
    /// Create a swarm directly from its component parts.
    ///
    /// Prefer [`SwarmOperator::builder`] for incremental construction.
    pub fn new(
        entry: OperatorId,
        transitions: HashMap<OperatorId, Vec<OperatorId>>,
        dispatcher: Arc<dyn Dispatcher>,
        max_handoffs: u32,
    ) -> Self {
        Self {
            transitions,
            entry,
            dispatcher,
            max_handoffs,
        }
    }

    /// Start building a [`SwarmOperator`].
    pub fn builder(dispatcher: Arc<dyn Dispatcher>) -> SwarmBuilder {
        SwarmBuilder::new(dispatcher)
    }
}

#[async_trait]
impl Operator for SwarmOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let mut current_id = self.entry.clone();
        let mut current_input = input;
        let mut all_effects: Vec<layer0::Effect> = Vec::new();
        let mut handoffs: u32 = 0;

        loop {
            let child_ctx = ctx.child(next_dispatch_id(), current_id.clone());

            let mut output = self
                .dispatcher
                .dispatch(&child_ctx, current_input)
                .await?
                .collect()
                .await?;

            // Extract the handoff target AND context from this round's effects
            // before moving them into the accumulator.
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

            // Absorb this round's effects into the running total.
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
                    // Handoff cap enforced before any routing work.
                    if handoffs >= self.max_handoffs {
                        let mut out = OperatorOutput::new(
                            output.message.clone(),
                            Outcome::Limited {
                                limit: LimitReason::MaxTurns,
                            },
                        );
                        out.effects = all_effects;
                        return Ok(out);
                    }

                    let (target, context) = handoff.ok_or_else(|| {
                        ProtocolError::internal(format!(
                            "operator '{}' exited HandedOff but emitted no EffectKind::Handoff",
                            current_id.as_str()
                        ))
                    })?;

                    // Validate that this transition is explicitly permitted.
                    let allowed = self
                        .transitions
                        .get(&current_id)
                        .map(|targets| targets.contains(&target))
                        .unwrap_or(false);

                    if !allowed {
                        return Err(ProtocolError::internal(format!(
                            "swarm: transition '{}' → '{}' is not allowed",
                            current_id.as_str(),
                            target.as_str()
                        )));
                    }

                    handoffs += 1;
                    // Use context.task as the next input — it is the explicit
                    // task the handing-off operator wants the next one to act on.
                    let mut next_input = OperatorInput::new(context.task, TriggerType::Task);
                    if let Some(hist) = context.history {
                        next_input.context = Some(hist);
                    }
                    current_input = next_input;
                    current_id = target;
                }
                // Unexpected exit — surface immediately with accumulated effects.
                _ => {
                    output.effects = all_effects;
                    return Ok(output);
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Fluent builder for [`SwarmOperator`].
///
/// ```rust,ignore
/// let swarm = SwarmOperator::builder(dispatcher)
///     .entry(OperatorId::new("a"))
///     .transition(OperatorId::new("a"), OperatorId::new("b"))
///     .max_handoffs(10)
///     .build();
/// ```
pub struct SwarmBuilder {
    dispatcher: Arc<dyn Dispatcher>,
    entry: Option<OperatorId>,
    transitions: HashMap<OperatorId, Vec<OperatorId>>,
    max_handoffs: u32,
}

impl SwarmBuilder {
    fn new(dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            dispatcher,
            entry: None,
            transitions: HashMap::new(),
            max_handoffs: 10,
        }
    }

    /// Set the entry (starting) operator.
    pub fn entry(mut self, id: OperatorId) -> Self {
        self.entry = Some(id);
        self
    }

    /// Allow operator `from` to hand off to operator `to`.
    ///
    /// Call this once per directed edge. Multiple calls with the same `from`
    /// accumulate targets.
    pub fn transition(mut self, from: OperatorId, to: OperatorId) -> Self {
        self.transitions.entry(from).or_default().push(to);
        self
    }

    /// Override the handoff cap (default: 10).
    pub fn max_handoffs(mut self, n: u32) -> Self {
        self.max_handoffs = n;
        self
    }

    /// Consume the builder and produce a [`SwarmOperator`].
    ///
    /// # Panics
    /// Panics if [`entry`](Self::entry) was never called.
    pub fn build(self) -> SwarmOperator {
        SwarmOperator {
            entry: self
                .entry
                .expect("SwarmBuilder: entry operator is required"),
            transitions: self.transitions,
            dispatcher: self.dispatcher,
            max_handoffs: self.max_handoffs,
        }
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

    fn test_ctx(name: &str) -> DispatchContext {
        DispatchContext::new(DispatchId::new(name), OperatorId::new(name))
    }

    fn simple_input(msg: &str) -> OperatorInput {
        OperatorInput::new(Content::text(msg), TriggerType::User)
    }

    /// Operator that completes with a fixed reply.
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

    /// Operator that emits `Effect::Handoff` with a structured `HandoffContext`.
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
        ) -> Result<OperatorOutput, ProtocolError> {
            let mut out = OperatorOutput::new(
                Content::text(self.reply.clone()),
                Outcome::Transfer {
                    transfer: TransferOutcome::HandedOff,
                },
            );
            out.effects.push(Effect::new(EffectKind::Handoff {
                operator: self.target.clone(),
                context: HandoffContext {
                    task: Content::text(self.reply.clone()),
                    history: None,
                    metadata: None,
                },
            }));
            Ok(out)
        }
    }

    /// A → B (allowed), B completes. Final message must come from B.
    #[tokio::test]
    async fn swarm_follows_handoff_chain() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("op-a"),
            Arc::new(HandoffOp {
                target: OperatorId::new("op-b"),
                reply: "from-a".into(),
            }),
        );
        orch.register(
            OperatorId::new("op-b"),
            Arc::new(CompleteOp {
                reply: "from-b".into(),
            }),
        );
        let orch = Arc::new(orch);

        let swarm =
            SwarmOperator::builder(Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>)
                .entry(OperatorId::new("op-a"))
                .transition(OperatorId::new("op-a"), OperatorId::new("op-b"))
                .max_handoffs(5)
                .build();

        let output = swarm
            .execute(simple_input("task"), &test_ctx("swarm"))
            .await
            .expect("swarm should complete");

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(
            output.message.as_text().unwrap_or(""),
            "from-b",
            "final message must come from op-b"
        );
        // One Effect::Handoff from op-a.
        assert_eq!(output.effects.len(), 1);
    }

    /// A tries to hand off to C, but only A→B is declared. Must error.
    #[tokio::test]
    async fn swarm_rejects_invalid_transition() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("op-a"),
            Arc::new(HandoffOp {
                target: OperatorId::new("op-c"), // not allowed
                reply: "from-a".into(),
            }),
        );
        // op-c is registered so the dispatcher can find it; the swarm must
        // refuse before dispatching.
        orch.register(
            OperatorId::new("op-c"),
            Arc::new(CompleteOp {
                reply: "from-c".into(),
            }),
        );
        let orch = Arc::new(orch);

        let swarm =
            SwarmOperator::builder(Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>)
                .entry(OperatorId::new("op-a"))
                .transition(OperatorId::new("op-a"), OperatorId::new("op-b")) // B, not C
                .max_handoffs(5)
                .build();

        let result = swarm
            .execute(simple_input("task"), &test_ctx("swarm-reject"))
            .await;

        assert!(
            result.is_err(),
            "swarm must error on an undeclared transition"
        );
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("op-c"),
            "error must name the rejected target: {msg}"
        );
    }

    /// Verifies that the swarm forwards `context.task` — not `output.message` —
    /// as the input to the next operator in the chain.
    #[tokio::test]
    async fn swarm_uses_handoff_context_task() {
        /// Emits a Handoff whose `context.task` differs from `output.message`.
        struct DistinctTaskOp;

        #[async_trait]
        impl Operator for DistinctTaskOp {
            async fn execute(
                &self,
                _input: OperatorInput,
                _ctx: &DispatchContext,
            ) -> Result<OperatorOutput, ProtocolError> {
                let mut out = OperatorOutput::new(
                    Content::text("output-message-ignored"),
                    Outcome::Transfer {
                        transfer: TransferOutcome::HandedOff,
                    },
                );
                out.effects.push(Effect::new(EffectKind::Handoff {
                    operator: OperatorId::new("receiver"),
                    context: HandoffContext {
                        task: Content::text("context-task-for-receiver"),
                        history: None,
                        metadata: None,
                    },
                }));
                Ok(out)
            }
        }

        /// Records the input message it received.
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
        orch.register(OperatorId::new("sender"), Arc::new(DistinctTaskOp));
        orch.register(
            OperatorId::new("receiver"),
            Arc::new(RecordingOp {
                received: received.clone(),
            }),
        );
        let orch = Arc::new(orch);

        let swarm =
            SwarmOperator::builder(Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>)
                .entry(OperatorId::new("sender"))
                .transition(OperatorId::new("sender"), OperatorId::new("receiver"))
                .max_handoffs(5)
                .build();

        let output = swarm
            .execute(simple_input("start"), &test_ctx("swarm-task"))
            .await
            .expect("swarm must complete");

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(
            *received.lock().unwrap(),
            "context-task-for-receiver",
            "receiver must get context.task, not output.message"
        );
    }
}
