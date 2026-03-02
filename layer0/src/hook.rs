//! The Hook interface — observation and intervention in the turn's inner loop.

use crate::{content::Content, error::HookError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Where in the turn's inner loop a hook fires.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    /// Before each model inference call.
    PreInference,
    /// After model responds, before tool execution.
    PostInference,
    /// Before each tool is executed.
    PreToolUse,
    /// After each tool completes, before result enters context.
    PostToolUse,
    /// At each exit-condition check.
    ExitCheck,
    /// During tool execution: a streaming update chunk is available.
    ToolExecutionUpdate,
}

/// What context is available to a hook at its firing point.
/// Read-only — hooks observe and decide, they don't mutate directly.
/// (Mutation happens via HookAction::Modify.)
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Current hook point.
    pub point: HookPoint,
    /// Current tool being called (only at Pre/PostToolUse).
    pub tool_name: Option<String>,
    /// Tool input (only at PreToolUse).
    pub tool_input: Option<serde_json::Value>,
    /// Tool result (only at PostToolUse).
    /// Tool result (only at PostToolUse).
    pub tool_result: Option<String>,
    /// Model response (only at PostInference).
    pub model_output: Option<Content>,
    /// Running count of tokens used.
    pub tokens_used: u64,
    /// Running cost in USD.
    pub cost: rust_decimal::Decimal,
    /// Number of turns completed so far.
    pub turns_completed: u32,
    /// Time elapsed since the turn started.
    pub elapsed: crate::duration::DurationMs,
    /// Streaming chunk text (only at ToolExecutionUpdate).
    pub tool_chunk: Option<String>,
}

impl HookContext {
    /// Create a new HookContext with only the hook point set.
    pub fn new(point: HookPoint) -> Self {
        Self {
            point,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            model_output: None,
            tokens_used: 0,
            cost: rust_decimal::Decimal::ZERO,
            turns_completed: 0,
            elapsed: crate::duration::DurationMs::ZERO,
            tool_chunk: None,
        }
    }
}

/// What a hook decides to do.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HookAction {
    /// Continue normally.
    Continue,
    /// Halt the turn (observer tripwire). The turn exits with
    /// ExitReason::ObserverHalt.
    Halt {
        /// Reason for halting.
        reason: String,
    },
    /// Skip this tool call (only valid at PreToolUse).
    /// The tool is not executed and a synthetic "skipped by policy"
    /// result is backfilled.
    SkipTool {
        /// Reason for skipping.
        reason: String,
    },
    /// Modify the tool input before execution (only at PreToolUse).
    /// Used for: parameter sanitization, injection of defaults.
    ModifyToolInput {
        /// The replacement tool input.
        new_input: serde_json::Value,
    },
    /// Replace the tool output with a modified version (e.g., redacted secrets).
    /// Only valid at PostToolUse. v0 scope: PostToolUse only.
    /// Future: PostInference for redacting final assistant text before return/logging.
    ModifyToolOutput {
        /// The replacement output.
        new_output: serde_json::Value,
    },
}

/// A hook that can observe and intervene in the turn's inner loop.
///
/// Hooks are registered externally (by the orchestrator, environment,
/// or lifecycle coordinator) and the turn runtime calls them at the
/// defined points. The turn doesn't know who's watching.
///
/// Implementations:
/// - BudgetHook: track cost, halt if over budget
/// - GuardrailHook: validate tool calls against policy
/// - TelemetryHook: emit OpenTelemetry spans
/// - HeartbeatHook: signal liveness to orchestrator (Temporal)
/// - MemorySyncHook: trigger memory writes after state-changing tools
///
/// Hook handlers SHOULD complete quickly. An LLM-based guardrail that
/// calls a model on every tool use adds latency to every tool call.
/// The performance cost is the hook author's responsibility.
#[async_trait]
pub trait Hook: Send + Sync {
    /// Which points this hook fires at.
    fn points(&self) -> &[HookPoint];

    /// Called at each registered hook point.
    /// Returning an error does NOT halt the turn — it logs the error
    /// and continues. Use HookAction::Halt to halt.
    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError>;
}
