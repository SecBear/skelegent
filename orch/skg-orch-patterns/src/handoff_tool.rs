//! [`HandoffTool`] — a `ToolDyn` that emits a sentinel JSON payload for LLM-driven routing.
//!
//! When an LLM calls this tool it receives a structured JSON object containing
//! `__handoff: true`, the `target` operator ID, and the `reason` the model
//! supplied. The orchestration layer inspects the tool result and performs the
//! actual handoff (e.g. by emitting `IntentKind::Handoff`).

use std::future::Future;
use std::pin::Pin;

use layer0::id::OperatorId;
use skg_tool::{ApprovalPolicy, ToolDyn, ToolError};

/// A tool that signals the orchestrator to hand off to another operator.
///
/// When the LLM calls this tool the `call` implementation returns:
/// ```json
/// { "__handoff": true, "target": "<operator_id>", "reason": "<supplied reason>" }
/// ```
///
/// The orchestration layer is responsible for detecting this sentinel and
/// performing the actual handoff. The tool itself never dispatches.
pub struct HandoffTool {
    /// Target operator that should receive the handoff.
    target: OperatorId,
    /// Human-readable description shown to the LLM.
    description: String,
    /// Pre-computed tool name: `transfer_to_{target}`.
    name: String,
    /// Pre-computed JSON input schema.
    schema: serde_json::Value,
}

impl HandoffTool {
    /// Create a new handoff tool for the given `target` operator.
    pub fn new(target: OperatorId, description: impl Into<String>) -> Self {
        let name = format!("transfer_to_{}", target.as_str());
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why this handoff is needed"
                }
            },
            "required": ["reason"]
        });
        Self {
            target,
            description: description.into(),
            name,
            schema,
        }
    }
}

impl ToolDyn for HandoffTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn call(
        &self,
        input: serde_json::Value,
        _ctx: &layer0::DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let target = self.target.as_str().to_string();
        // Extract the reason field; fall back gracefully if missing.
        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Box::pin(async move {
            Ok(serde_json::json!({
                "__handoff": true,
                "target": target,
                "reason": reason,
            }))
        })
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::None
    }
}
