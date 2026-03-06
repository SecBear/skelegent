//! The Hook interface — observation and intervention in the turn's inner loop.

use crate::state::StoreOptions;
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
    /// Before each sub-dispatch is executed.
    #[serde(alias = "pre_tool_use")]
    PreSubDispatch,
    /// After each sub-dispatch completes, before result enters context.
    #[serde(alias = "post_tool_use")]
    PostSubDispatch,
    /// At each exit-condition check.
    ExitCheck,
    /// During sub-dispatch execution: a streaming update chunk is available.
    #[serde(alias = "tool_execution_update")]
    SubDispatchUpdate,
    /// After steering source returns messages, before they enter context.
    /// Guardrails can Halt to reject the steering injection.
    PreSteeringInject,
    /// After tools are skipped due to steering. Observation only.
    PostSteeringSkip,
    /// Before a WriteMemory effect executes. Guardrails can Halt to prevent the write.
    PreMemoryWrite,
}

/// What context is available to a hook at its firing point.
/// Read-only — hooks observe and decide, they don't mutate directly.
/// (Mutation happens via HookAction::Modify.)
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Current hook point.
    pub point: HookPoint,
    /// Current operator being called (only at Pre/PostSubDispatch).
    #[serde(alias = "tool_name")]
    pub operator_name: Option<String>,
    /// Operator input (only at PreSubDispatch).
    #[serde(alias = "tool_input")]
    pub operator_input: Option<serde_json::Value>,
    /// Operator result (only at PostSubDispatch).
    #[serde(alias = "tool_result")]
    pub operator_result: Option<String>,
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
    /// Streaming chunk text (only at SubDispatchUpdate).
    #[serde(alias = "tool_chunk")]
    pub operator_chunk: Option<String>,
    /// Steering messages about to be injected (only at PreSteeringInject).
    #[serde(default)]
    pub steering_messages: Option<Vec<String>>,
    /// Operator names skipped due to steering (only at PostSteeringSkip).
    #[serde(default)]
    #[serde(alias = "skipped_tools")]
    pub skipped_operators: Option<Vec<String>>,
    /// Memory key being written (only at PreMemoryWrite).
    #[serde(default)]
    pub memory_key: Option<String>,
    /// Memory value being written (only at PreMemoryWrite).
    #[serde(default)]
    pub memory_value: Option<serde_json::Value>,
    /// Advisory storage options for the write being evaluated (only at PreMemoryWrite).
    ///
    /// Contains tier, lifetime, content_kind, salience, and ttl hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_options: Option<StoreOptions>,
}

impl HookContext {
    /// Create a new HookContext with only the hook point set.
    pub fn new(point: HookPoint) -> Self {
        Self {
            point,
            operator_name: None,
            operator_input: None,
            operator_result: None,
            model_output: None,
            tokens_used: 0,
            cost: rust_decimal::Decimal::ZERO,
            turns_completed: 0,
            elapsed: crate::duration::DurationMs::ZERO,
            operator_chunk: None,
            steering_messages: None,
            skipped_operators: None,
            memory_key: None,
            memory_value: None,
            memory_options: None,
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
    /// Skip this sub-dispatch (only valid at PreSubDispatch).
    /// The operator is not executed and a synthetic "skipped by policy"
    /// result is backfilled.
    #[serde(alias = "skip_tool")]
    SkipDispatch {
        /// Reason for skipping.
        reason: String,
    },
    /// Modify the operator input before execution (only at PreSubDispatch).
    /// Used for: parameter sanitization, injection of defaults.
    #[serde(alias = "modify_tool_input")]
    ModifyDispatchInput {
        /// The replacement operator input.
        new_input: serde_json::Value,
    },
    /// Replace the operator output with a modified version (e.g., redacted secrets).
    /// Only valid at PostSubDispatch. v0 scope: PostSubDispatch only.
    /// Future: PostInference for redacting final assistant text before return/logging.
    #[serde(alias = "modify_tool_output")]
    ModifyDispatchOutput {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hookpoint_serde_roundtrip() {
        let variants = [
            HookPoint::PreSteeringInject,
            HookPoint::PostSteeringSkip,
            HookPoint::PreMemoryWrite,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize");
            let roundtripped: HookPoint = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(variant, roundtripped);
        }
    }

    #[test]
    fn hookcontext_new_steering_fields_are_none() {
        let ctx = HookContext::new(HookPoint::PreSteeringInject);
        assert!(ctx.steering_messages.is_none());
        assert!(ctx.skipped_operators.is_none());
    }

    #[test]
    fn hookcontext_new_memory_fields_are_none() {
        let ctx = HookContext::new(HookPoint::PreMemoryWrite);
        assert!(ctx.memory_key.is_none());
        assert!(ctx.memory_value.is_none());
        assert!(ctx.memory_options.is_none());
    }

    #[test]
    fn hookcontext_steering_fields_populated() {
        let mut ctx = HookContext::new(HookPoint::PreSteeringInject);
        ctx.steering_messages = Some(vec!["msg".to_string()]);
        let msgs = ctx.steering_messages.as_ref().expect("should be Some");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "msg");
    }
}
