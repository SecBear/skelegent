//! Lifecycle events — cross-protocol coordination vocabulary.
//!
//! These are NOT a trait — they're a shared vocabulary. Each protocol
//! emits and/or consumes these events through whatever mechanism
//! is appropriate (channels, callbacks, event bus, direct calls).
//!
//! The Lifecycle Interface is deliberately not a trait because
//! lifecycle coordination is the orchestrator's job. The orchestrator
//! listens for events, applies policies, and takes action. There's
//! no separate "lifecycle service" — it's a responsibility of
//! the orchestration layer.
//!
//! ## Future: Sub-Turn Event Streaming
//!
//! The current lifecycle events are coarse-grained (budget, compaction,
//! observability). A future extension may add a sub-turn event stream
//! for real-time observation of individual inference calls, tool
//! executions, and context mutations within a turn — enabling live
//! dashboards and fine-grained telemetry beyond the Hook interface.

use crate::{content::Content, duration::DurationMs, effect::Scope, id::*};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Budget-related events.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BudgetEvent {
    /// Emitted by turn after each model call.
    CostIncurred {
        /// The operator that incurred the cost.
        operator: OperatorId,
        /// Cost of this individual operation.
        cost: Decimal,
        /// Cumulative cost so far.
        cumulative: Decimal,
    },
    /// Emitted by orchestrator when nearing limit.
    BudgetWarning {
        /// The workflow approaching its budget limit.
        workflow: WorkflowId,
        /// Amount spent so far.
        spent: Decimal,
        /// The budget limit.
        limit: Decimal,
    },
    /// Decision by orchestrator.
    BudgetAction {
        /// The workflow the decision applies to.
        workflow: WorkflowId,
        /// The budget decision.
        action: BudgetDecision,
    },
    /// Emitted by operator when sub-dispatch count approaches the configured limit.
    StepLimitApproaching {
        /// The operator approaching its step limit.
        operator: OperatorId,
        /// Current sub-dispatch count.
        current: u32,
        /// Configured maximum sub-dispatches.
        max: u32,
    },
    /// Emitted by operator when the step limit is reached.
    StepLimitReached {
        /// The operator that hit its step limit.
        operator: OperatorId,
        /// Total sub-dispatches executed.
        #[serde(alias = "total_tool_calls")]
        total_sub_dispatches: u32,
    },
    /// Emitted by operator when identical consecutive sub-dispatches exceed the loop limit.
    LoopDetected {
        /// The operator stuck in a loop.
        operator: OperatorId,
        /// Name of the operator being repeated.
        #[serde(alias = "tool_name")]
        operator_name: String,
        /// Number of consecutive identical calls detected.
        consecutive_count: u32,
        /// Configured maximum consecutive calls.
        max: u32,
    },
    /// Emitted by operator when elapsed time approaches the configured timeout.
    TimeoutApproaching {
        /// The operator approaching its timeout.
        operator: OperatorId,
        /// Elapsed duration so far.
        elapsed: DurationMs,
        /// Configured maximum duration.
        max_duration: DurationMs,
    },
    /// Emitted by operator when the timeout limit is reached.
    TimeoutReached {
        /// The operator that hit its timeout.
        operator: OperatorId,
        /// Elapsed duration at timeout.
        elapsed: DurationMs,
    },
}

/// What the orchestrator decides to do about budget pressure.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDecision {
    /// Continue as normal.
    Continue,
    /// Switch to a cheaper model.
    DowngradeModel {
        /// The model being switched from.
        from: String,
        /// The model being switched to.
        to: String,
    },
    /// Stop the entire workflow.
    HaltWorkflow,
    /// Request more budget from the caller.
    RequestIncrease {
        /// The additional amount requested.
        amount: Decimal,
    },
}

/// Context pressure events — for compaction coordination.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompactionEvent {
    /// Emitted by turn when context window is filling.
    ContextPressure {
        /// The operator experiencing context pressure.
        operator: OperatorId,
        /// Percentage of context window used.
        fill_percent: f64,
        /// Tokens currently used.
        tokens_used: u64,
        /// Tokens still available.
        tokens_available: u64,
    },
    /// Emitted before compaction to trigger memory flush.
    PreCompactionFlush {
        /// The operator about to compact.
        operator: OperatorId,
        /// The scope to flush.
        scope: Scope,
    },
    /// Emitted after compaction completes.
    CompactionComplete {
        /// The operator that completed compaction.
        operator: OperatorId,
        /// The compaction strategy used.
        strategy: String,
        /// Number of tokens freed.
        tokens_freed: u64,
    },
    /// Compaction was initiated and executed by the model provider
    /// (e.g., Anthropic's server-side context compaction). The turn
    /// receives a compacted context back from the inference call itself.
    ProviderManaged {
        /// The operator whose context was compacted.
        operator: OperatorId,
        /// The provider that performed the compaction.
        provider: String,
        /// Token count before compaction.
        tokens_before: u64,
        /// Token count after compaction.
        tokens_after: u64,
        /// Optional summary content produced by the provider.
        summary: Option<Content>,
    },
    /// Emitted when compaction fails with an error.
    CompactionFailed {
        /// The operator whose compaction failed.
        operator: OperatorId,
        /// Human-readable error description.
        error: String,
        /// The strategy that failed.
        strategy: String,
    },
    /// Emitted when compaction is intentionally skipped (conditions not met or hook blocked).
    CompactionSkipped {
        /// The operator whose compaction was skipped.
        operator: OperatorId,
        /// Why compaction was skipped.
        reason: String,
    },
    /// Emitted when a pre-compaction memory flush fails.
    FlushFailed {
        /// The operator whose flush failed.
        operator: OperatorId,
        /// The scope that failed to flush.
        scope: Scope,
        /// The key that failed to flush.
        key: String,
        /// The error description.
        error: String,
    },
    /// Emitted after compaction with quality metrics.
    CompactionQuality {
        /// The operator that compacted.
        operator: OperatorId,
        /// Token count before compaction.
        tokens_before: u64,
        /// Token count after compaction.
        tokens_after: u64,
        /// Number of messages preserved.
        items_preserved: u32,
        /// Number of messages lost.
        items_lost: u32,
    },
}

/// Policy controlling how a message survives compaction.
///
/// Stored in [`crate::MessageMeta`], which is attached to every message.
/// All variants are advisory when used with strategies that don't inspect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPolicy {
    /// Never compact this message. Architectural decisions, constraints, user instructions.
    Pinned,
    /// Subject to normal compaction. Default for all messages.
    #[default]
    Normal,
    /// Compress this message preferentially (verbose output, build logs).
    CompressFirst,
    /// Discard when the originating tool session or MCP session ends.
    DiscardWhenDone,
}
