//! Operator-local interception traits for ReactOperator.
//!
//! `ReactInterceptor` replaces the old `HookRegistry`-based interception with
//! typed, per-boundary methods. Every method has a default no-op implementation,
//! so consumers only override the hook points they care about.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::duration::DurationMs;
use rust_decimal::Decimal;
use serde_json::Value;

/// Snapshot of the operator loop state, passed to interceptor methods.
#[derive(Debug, Clone)]
pub struct LoopState {
    /// Total input tokens consumed so far.
    pub tokens_in: u64,
    /// Total output tokens consumed so far.
    pub tokens_out: u64,
    /// Cumulative cost.
    pub cost: Decimal,
    /// Number of completed inference turns.
    pub turns_completed: u32,
    /// Wall-clock elapsed since operator start.
    pub elapsed: DurationMs,
}

/// Action returned by guard-capable interceptor methods.
#[derive(Debug, Clone)]
pub enum ReactAction {
    /// Continue normal execution.
    Continue,
    /// Halt the operator loop, returning with the given reason.
    Halt {
        /// Human-readable reason for halting.
        reason: String,
    },
}

/// Action returned by pre-sub-dispatch interceptor.
#[derive(Debug, Clone)]
pub enum SubDispatchAction {
    /// Continue with original input.
    Continue,
    /// Halt the entire operator loop.
    Halt {
        /// Reason for halting.
        reason: String,
    },
    /// Skip this tool call without error.
    Skip {
        /// Reason shown in tool result.
        reason: String,
    },
    /// Modify the tool input before dispatch.
    ModifyInput {
        /// Replacement input value.
        new_input: Value,
    },
}

/// Result returned by post-sub-dispatch interceptor.
#[derive(Debug, Clone)]
pub enum SubDispatchResult {
    /// Continue with original result.
    Continue,
    /// Halt the entire operator loop.
    Halt {
        /// Reason for halting.
        reason: String,
    },
    /// Replace the tool result content.
    ModifyOutput {
        /// Replacement output string.
        new_output: String,
    },
}

/// Extension point for ReactOperator loop interception.
///
/// Every method has a default no-op implementation that returns `Continue`.
/// Implement only the methods you need.
///
/// # Hook point mapping (from old HookRegistry)
///
/// | Old HookPoint         | New method              |
/// |-----------------------|-------------------------|
/// | PreSteeringInject     | `pre_steering_inject`   |
/// | PreInference          | `pre_inference`         |
/// | PostInference         | `post_inference`        |
/// | PostSteeringSkip      | `post_steering_skip`    |
/// | PreSubDispatch        | `pre_sub_dispatch`      |
/// | PostSubDispatch       | `post_sub_dispatch`     |
/// | SubDispatchUpdate     | (removed — use tracing) |
/// | ExitCheck             | `exit_check`            |
/// | PreCompaction         | `pre_compaction`        |
/// | PostCompaction        | `post_compaction`       |
#[async_trait]
pub trait ReactInterceptor: Send + Sync {
    /// Called before steering messages are injected into the context.
    ///
    /// Return `Halt` to block injection (messages are discarded).
    async fn pre_steering_inject(&self, _state: &LoopState, _messages: &[String]) -> ReactAction {
        ReactAction::Continue
    }

    /// Called before each inference (model) call.
    ///
    /// Return `Halt` to exit the loop early.
    async fn pre_inference(&self, _state: &LoopState) -> ReactAction {
        ReactAction::Continue
    }

    /// Called after each inference (model) call.
    ///
    /// Return `Halt` to exit the loop with the given reason.
    async fn post_inference(&self, _state: &LoopState, _response: &Content) -> ReactAction {
        ReactAction::Continue
    }

    /// Called when tool calls are skipped due to steering injection.
    ///
    /// Observer-only — return value is ignored.
    async fn post_steering_skip(&self, _state: &LoopState, _skipped: &[String]) {}

    /// Called before a tool/sub-operator dispatch.
    ///
    /// Can halt, skip, or modify the tool input.
    async fn pre_sub_dispatch(
        &self,
        _state: &LoopState,
        _tool_name: &str,
        _input: &Value,
    ) -> SubDispatchAction {
        SubDispatchAction::Continue
    }

    /// Called after a tool/sub-operator dispatch.
    ///
    /// Can halt or modify the result.
    async fn post_sub_dispatch(
        &self,
        _state: &LoopState,
        _tool_name: &str,
        _result: &str,
    ) -> SubDispatchResult {
        SubDispatchResult::Continue
    }

    /// Called after each tool batch to check whether to exit early.
    ///
    /// Fires before built-in limit checks (step limit, timeout, etc.).
    async fn exit_check(&self, _state: &LoopState) -> ReactAction {
        ReactAction::Continue
    }

    /// Called before context compaction.
    ///
    /// Return `Halt` to skip compaction for this cycle.
    async fn pre_compaction(&self, _state: &LoopState, _message_count: usize) -> ReactAction {
        ReactAction::Continue
    }

    /// Called after successful context compaction.
    ///
    /// Observer-only — return value is ignored.
    async fn post_compaction(&self, _state: &LoopState, _before: usize, _after: usize) {}
}
