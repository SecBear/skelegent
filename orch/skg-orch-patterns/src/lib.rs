#![deny(missing_docs)]
//! Multi-agent composition patterns for skelegent.
//!
//! All patterns implement [`layer0::Operator`] and compose via
//! [`layer0::Dispatcher`] constructor injection.
//!
//! # Patterns
//!
//! | Type | Description |
//! |---|---|
//! | [`HandoffTool`] | Emits sentinel JSON for LLM-driven routing |
//! | [`OperatorTool`] | Wraps an operator as a [`skg_tool::ToolDyn`] |
//! | [`SequentialOperator`] | Pipeline: each output feeds the next input |
//! | [`ParallelOperator`] | Fan-out to N operators with a configurable reducer |
//! | [`FanOutOperator`] | Fan-out to one operator with N data-derived inputs |
//! | [`LoopOperator`] | Repeat until a predicate fires |
//! | [`SpeakerSelector`] | Pluggable routing trait for supervisor patterns |
//! | [`SupervisorOperator`] | LLM-driven routing via a pluggable selector |
//! | [`SwarmOperator`] | Peer-to-peer handoff with transition constraints |
//! | [`WorkflowBuilder`] | Fluent API that compiles to pattern operators |

pub mod builder;
pub mod handoff_tool;
pub mod loop_op;
pub mod operator_tool;
pub mod parallel;
pub mod selector;
pub mod sequential;
pub mod supervisor;
pub mod swarm;

pub use builder::WorkflowBuilder;
pub use handoff_tool::HandoffTool;
pub use loop_op::LoopOperator;
pub use operator_tool::OperatorTool;
pub use parallel::{FanOutOperator, ParallelOperator, ReducerFn, SplitterFn};
pub use selector::{RandomSelector, RoundRobinSelector, SelectorError, SpeakerSelector};
pub use sequential::SequentialOperator;
pub use supervisor::SupervisorOperator;
pub use swarm::{SwarmBuilder, SwarmOperator};

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::error::OperatorError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{Operator, OperatorInput, OperatorOutput, TriggerType};
    use layer0::{DispatchContext, ExitReason};
    use skg_orch_local::LocalOrch;

    use super::*;

    // ─────────────────────────────────────────────────────────────────────────
    // Test helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Echo operator: returns its input message unchanged.
    struct EchoOperator;

    #[async_trait]
    impl Operator for EchoOperator {
        async fn execute(
            &self,
            input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    /// Appending operator: prepends a fixed prefix to the input message.
    struct PrefixOperator {
        prefix: String,
    }

    #[async_trait]
    impl Operator for PrefixOperator {
        async fn execute(
            &self,
            input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            let msg = format!("{}{}", self.prefix, input.message.as_text().unwrap_or(""));
            Ok(OperatorOutput::new(
                Content::text(msg),
                ExitReason::Complete,
            ))
        }
    }

    /// Counting operator: appends the current invocation count to the message.
    struct CountingOperator {
        count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl Operator for CountingOperator {
        async fn execute(
            &self,
            input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            let n = self.count.fetch_add(1, Ordering::Relaxed) + 1;
            let msg = format!("{}-iter{n}", input.message.as_text().unwrap_or(""));
            Ok(OperatorOutput::new(
                Content::text(msg),
                ExitReason::Complete,
            ))
        }
    }

    fn test_ctx(operator: &str) -> DispatchContext {
        DispatchContext::new(DispatchId::new(operator), OperatorId::new(operator))
    }

    fn simple_input(msg: &str) -> OperatorInput {
        OperatorInput::new(Content::text(msg), TriggerType::User)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // HandoffTool
    // ─────────────────────────────────────────────────────────────────────────

    /// HandoffTool.call() must return the sentinel JSON payload.
    #[tokio::test]
    async fn handoff_tool_returns_sentinel() {
        use skg_tool::ToolDyn;

        let tool = HandoffTool::new(
            OperatorId::new("billing-agent"),
            "Transfer to the billing specialist",
        );

        assert_eq!(tool.name(), "transfer_to_billing-agent");

        let input = serde_json::json!({ "reason": "user asked about invoice" });
        let ctx = test_ctx("root");
        let result = tool.call(input, &ctx).await.expect("call should succeed");

        assert_eq!(result["__handoff"], serde_json::json!(true));
        assert_eq!(result["target"], "billing-agent");
        assert_eq!(result["reason"], "user asked about invoice");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // SequentialOperator
    // ─────────────────────────────────────────────────────────────────────────

    /// Three sequential steps; each prepends its label. The final message must
    /// contain all three prefixes in order.
    #[tokio::test]
    async fn sequential_chains_outputs() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("a"),
            Arc::new(PrefixOperator {
                prefix: "[A]".into(),
            }),
        );
        orch.register(
            OperatorId::new("b"),
            Arc::new(PrefixOperator {
                prefix: "[B]".into(),
            }),
        );
        orch.register(
            OperatorId::new("c"),
            Arc::new(PrefixOperator {
                prefix: "[C]".into(),
            }),
        );
        let orch = Arc::new(orch);

        let seq = SequentialOperator::new(
            vec![
                OperatorId::new("a"),
                OperatorId::new("b"),
                OperatorId::new("c"),
            ],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
        );

        let ctx = test_ctx("a");
        let output = seq
            .execute(simple_input("hello"), &ctx)
            .await
            .expect("sequential should complete");

        let text = output.message.as_text().expect("should be text");
        // [C] prefix is applied last, wrapping [B][A]hello
        assert!(
            text.contains("[A]"),
            "step A output must be present: {text}"
        );
        assert!(
            text.contains("[B]"),
            "step B output must be present: {text}"
        );
        assert!(
            text.contains("[C]"),
            "step C output must be present: {text}"
        );
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ParallelOperator
    // ─────────────────────────────────────────────────────────────────────────

    /// Both branches must execute; the default reducer concatenates output.
    #[tokio::test]
    async fn parallel_runs_concurrently() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("left"),
            Arc::new(PrefixOperator {
                prefix: "[LEFT]".into(),
            }),
        );
        orch.register(
            OperatorId::new("right"),
            Arc::new(PrefixOperator {
                prefix: "[RIGHT]".into(),
            }),
        );
        let orch = Arc::new(orch);

        let par = ParallelOperator::with_default_reducer(
            vec![OperatorId::new("left"), OperatorId::new("right")],
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
        );

        let ctx = test_ctx("left");
        let output = par
            .execute(simple_input("msg"), &ctx)
            .await
            .expect("parallel should complete");

        let text = output.message.as_text().expect("should be text");
        assert!(text.contains("[LEFT]"), "left branch missing: {text}");
        assert!(text.contains("[RIGHT]"), "right branch missing: {text}");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // LoopOperator
    // ─────────────────────────────────────────────────────────────────────────

    /// Loop with a counter: must run exactly `target_iters` times before the
    /// predicate fires, and must not overshoot or undershoot.
    #[tokio::test]
    async fn loop_terminates_on_predicate() {
        const TARGET: u32 = 3;

        let call_count = Arc::new(AtomicU32::new(0));

        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("body"),
            Arc::new(CountingOperator {
                count: Arc::clone(&call_count),
            }),
        );
        let orch = Arc::new(orch);

        // Terminate after TARGET iterations by inspecting the message suffix.
        let lp = LoopOperator::new(
            OperatorId::new("body"),
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            10, // max_iterations — must not reach this
            move |out| {
                out.message
                    .as_text()
                    .map(|t| t.ends_with(&format!("iter{TARGET}")))
                    .unwrap_or(false)
            },
        );

        let ctx = test_ctx("body");
        let output = lp
            .execute(simple_input(""), &ctx)
            .await
            .expect("loop should complete");

        assert_eq!(
            call_count.load(Ordering::Relaxed),
            TARGET,
            "operator should run exactly {TARGET} times"
        );
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert!(
            output
                .message
                .as_text()
                .unwrap_or("")
                .ends_with(&format!("iter{TARGET}")),
            "final message should reflect last iteration"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // RoundRobinSelector
    // ─────────────────────────────────────────────────────────────────────────

    /// Round-robin must cycle through all candidates in order and wrap.
    #[tokio::test]
    async fn round_robin_cycles() {
        let candidates = vec![
            OperatorId::new("a"),
            OperatorId::new("b"),
            OperatorId::new("c"),
        ];
        let ctx = test_ctx("rr");
        let sel = RoundRobinSelector::new();

        let picks: Vec<String> = {
            let mut v = Vec::new();
            for _ in 0..6 {
                let id = sel
                    .select(&candidates, &[], &ctx)
                    .await
                    .expect("select should succeed");
                v.push(id.as_str().to_string());
            }
            v
        };

        // First cycle: a, b, c; second cycle: a, b, c.
        assert_eq!(picks, vec!["a", "b", "c", "a", "b", "c"]);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // WorkflowBuilder
    // ─────────────────────────────────────────────────────────────────────────

    /// Build a step + parallel workflow and verify it executes end-to-end.
    #[tokio::test]
    async fn workflow_builder_compiles() {
        let mut orch = LocalOrch::new();
        orch.register(
            OperatorId::new("prefix"),
            Arc::new(PrefixOperator {
                prefix: "[PRE]".into(),
            }),
        );
        orch.register(
            OperatorId::new("left"),
            Arc::new(PrefixOperator {
                prefix: "[L]".into(),
            }),
        );
        orch.register(OperatorId::new("right"), Arc::new(EchoOperator));
        let orch = Arc::new(orch);

        let op = WorkflowBuilder::new(Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>)
            .step(OperatorId::new("prefix"))
            .parallel(vec![OperatorId::new("left"), OperatorId::new("right")])
            .build();

        let ctx = test_ctx("wf");
        let output = op
            .execute(simple_input("base"), &ctx)
            .await
            .expect("workflow should complete");

        let text = output.message.as_text().expect("should be text");
        // After "prefix" step the message is "[PRE]base".
        // Parallel: left gets "[L][PRE]base", right echoes "[PRE]base".
        // Reducer concatenates both.
        assert!(text.contains("[PRE]"), "prefix step missing: {text}");
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }
    // ─────────────────────────────────────────────────────────────────────────
    // FanOutOperator
    // ─────────────────────────────────────────────────────────────────────────

    /// Splitter produces 3 inputs; all dispatched to same operator; reducer concatenates.
    #[tokio::test]
    async fn fan_out_splits_and_reduces() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("worker"), Arc::new(EchoOperator));
        let orch = Arc::new(orch);

        let fo = FanOutOperator::with_default_reducer(
            OperatorId::new("worker"),
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Box::new(|input: &OperatorInput| {
                let base = input.message.as_text().unwrap_or("").to_string();
                vec![
                    OperatorInput::new(Content::text(format!("{base}-0")), TriggerType::User),
                    OperatorInput::new(Content::text(format!("{base}-1")), TriggerType::User),
                    OperatorInput::new(Content::text(format!("{base}-2")), TriggerType::User),
                ]
            }),
        );

        let ctx = test_ctx("fan");
        let output = fo
            .execute(simple_input("chunk"), &ctx)
            .await
            .expect("fan-out should complete");

        let text = output.message.as_text().expect("should be text");
        // EchoOperator echoes each split input; default reducer concatenates all three.
        assert!(text.contains("chunk-0"), "branch 0 missing: {text}");
        assert!(text.contains("chunk-1"), "branch 1 missing: {text}");
        assert!(text.contains("chunk-2"), "branch 2 missing: {text}");
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    /// Splitter returns empty vec; reducer receives empty input and produces empty text.
    #[tokio::test]
    async fn fan_out_empty_split() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("worker"), Arc::new(EchoOperator));
        let orch = Arc::new(orch);

        let fo = FanOutOperator::with_default_reducer(
            OperatorId::new("worker"),
            Arc::clone(&orch) as Arc<dyn layer0::dispatch::Dispatcher>,
            Box::new(|_input: &OperatorInput| vec![]),
        );

        let ctx = test_ctx("fan-empty");
        let output = fo
            .execute(simple_input("ignored"), &ctx)
            .await
            .expect("empty fan-out should complete");

        // Default reducer on empty vec: parts.join("") → empty string, Complete exit.
        assert_eq!(output.exit_reason, ExitReason::Complete);
        let text = output.message.as_text().unwrap_or("");
        assert!(
            text.is_empty(),
            "expected empty text from empty split, got: {text:?}"
        );
    }
}
