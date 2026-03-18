//! Human-in-the-loop (HITL) approval types.
//!
//! When an operator reaches a point where tool calls require human review,
//! it suspends and emits an [`ApprovalRequest`]. The calling layer routes
//! this to an approver (human, policy engine, or automated gate). The approver
//! responds with an [`ApprovalResponse`] and the operator resumes.
//!
//! ## Wire format
//!
//! All types are serde-compatible (JSON by default). The `run_id` and
//! `wait_point` fields are plain `String` for layer0 portability — callers
//! that need strongly-typed IDs (e.g. `RunId`, `WaitPointId` from
//! `skg-run-core`) apply `From` conversions at the boundary.

use crate::content::Content;
use serde::{Deserialize, Serialize};

/// A request for human approval of pending tool calls.
///
/// Emitted by the dispatch layer as
/// [`DispatchEvent::AwaitingApproval`](crate::dispatch::DispatchEvent::AwaitingApproval)
/// when one or more tool calls require human review before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApprovalRequest {
    /// Durable run ID for correlation (plain `String` for layer0 portability).
    pub run_id: String,
    /// Wait point to resume at after a decision is made.
    pub wait_point: String,
    /// Tool calls awaiting a decision.
    pub pending: Vec<PendingToolCall>,
    /// Optional human-readable context surfaced to the approver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The operator's partial output at the suspension point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Content>,
}

impl ApprovalRequest {
    /// Construct a minimal [`ApprovalRequest`].
    ///
    /// `prompt` and `context` default to `None`; set them via field assignment
    /// or builder methods after construction.
    pub fn new(
        run_id: impl Into<String>,
        wait_point: impl Into<String>,
        pending: Vec<PendingToolCall>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            wait_point: wait_point.into(),
            pending,
            prompt: None,
            context: None,
        }
    }

    /// Attach a human-readable prompt for the approver.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Attach partial operator output as context for the approver.
    pub fn with_context(mut self, context: Content) -> Self {
        self.context = Some(context);
        self
    }
}

/// A single tool call awaiting a human decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    /// Provider-assigned ID for this tool call (used for correlation in responses).
    pub call_id: String,
    /// Name of the tool to be called.
    pub tool_name: String,
    /// Input arguments the model wants to pass to the tool (JSON).
    pub tool_input: serde_json::Value,
    /// Why this tool call requires approval.
    pub reason: ApprovalReason,
}

/// Why a tool call requires human approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ApprovalReason {
    /// The tool's configured `ApprovalPolicy` is `Always`.
    PolicyAlways,
    /// The tool's `ApprovalPolicy` predicate evaluated to `true`.
    PolicyPredicate {
        /// Human-readable description of what triggered the predicate.
        description: String,
    },
    /// A middleware guard blocked the tool call.
    MiddlewareBlocked {
        /// Name of the guard that blocked execution.
        guard: String,
    },
}

/// A human (or automated approver's) response to an [`ApprovalRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ApprovalResponse {
    /// Approve specific tool calls by their `call_id`.
    Approve {
        /// IDs of the tool calls to approve.
        call_ids: Vec<String>,
    },
    /// Approve every pending tool call in the request.
    ApproveAll,
    /// Reject specific tool calls.
    Reject {
        /// IDs of the tool calls to reject.
        call_ids: Vec<String>,
        /// Human-readable reason surfaced back to the operator.
        reason: String,
    },
    /// Reject every pending tool call in the request.
    RejectAll {
        /// Human-readable reason surfaced back to the operator.
        reason: String,
    },
    /// Approve a single tool call after modifying its input.
    Modify {
        /// ID of the tool call to modify.
        call_id: String,
        /// Replacement input to pass to the tool.
        new_input: serde_json::Value,
    },
    /// Per-call decisions when different calls need different actions.
    Batch {
        /// Ordered list of per-call decisions.
        decisions: Vec<ToolCallDecision>,
    },
}

impl ApprovalResponse {
    /// Approve all pending tool calls.
    pub fn approve_all() -> Self {
        Self::ApproveAll
    }

    /// Reject all pending tool calls with a reason.
    pub fn reject_all(reason: impl Into<String>) -> Self {
        Self::RejectAll {
            reason: reason.into(),
        }
    }
}

/// A per-call decision within a [`ApprovalResponse::Batch`] response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDecision {
    /// ID of the tool call this decision applies to.
    pub call_id: String,
    /// The action to take.
    pub action: ToolCallAction,
}

/// Action to take for a single tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ToolCallAction {
    /// Execute the tool call as-is.
    Approve,
    /// Do not execute the tool call.
    Reject {
        /// Human-readable reason.
        reason: String,
    },
    /// Execute the tool call with a modified input.
    Modify {
        /// Replacement input.
        new_input: serde_json::Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pending() -> PendingToolCall {
        PendingToolCall {
            call_id: "call-1".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"cmd": "rm -rf /"}),
            reason: ApprovalReason::PolicyAlways,
        }
    }

    #[test]
    fn approval_request_round_trip() {
        let req = ApprovalRequest::new("run-abc", "wp-1", vec![make_pending()])
            .with_prompt("Please review this command.");

        let json = serde_json::to_string(&req).unwrap();
        let decoded: ApprovalRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.run_id, "run-abc");
        assert_eq!(decoded.wait_point, "wp-1");
        assert_eq!(decoded.pending.len(), 1);
        assert_eq!(decoded.pending[0].call_id, "call-1");
        assert_eq!(
            decoded.prompt.as_deref(),
            Some("Please review this command.")
        );
        assert!(decoded.context.is_none());
    }

    #[test]
    fn pending_tool_call_round_trip() {
        let ptc = make_pending();
        let json = serde_json::to_string(&ptc).unwrap();
        let decoded: PendingToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.call_id, "call-1");
        assert_eq!(decoded.tool_name, "bash");
    }

    #[test]
    fn approval_reason_variants_round_trip() {
        let variants: Vec<ApprovalReason> = vec![
            ApprovalReason::PolicyAlways,
            ApprovalReason::PolicyPredicate {
                description: "writes to disk".into(),
            },
            ApprovalReason::MiddlewareBlocked {
                guard: "no-delete-guard".into(),
            },
        ];

        for reason in &variants {
            let json = serde_json::to_string(reason).unwrap();
            let _decoded: ApprovalReason = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn approval_response_approve_all_round_trip() {
        let resp = ApprovalResponse::approve_all();
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, ApprovalResponse::ApproveAll));
    }

    #[test]
    fn approval_response_reject_all_round_trip() {
        let resp = ApprovalResponse::reject_all("too dangerous");
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(decoded, ApprovalResponse::RejectAll { reason } if reason == "too dangerous")
        );
    }

    #[test]
    fn approval_response_modify_round_trip() {
        let resp = ApprovalResponse::Modify {
            call_id: "call-1".into(),
            new_input: serde_json::json!({"cmd": "echo safe"}),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: ApprovalResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, ApprovalResponse::Modify { call_id, .. } if call_id == "call-1"));
    }

    #[test]
    fn approval_response_batch_round_trip() {
        let resp = ApprovalResponse::Batch {
            decisions: vec![
                ToolCallDecision {
                    call_id: "call-1".into(),
                    action: ToolCallAction::Approve,
                },
                ToolCallDecision {
                    call_id: "call-2".into(),
                    action: ToolCallAction::Reject {
                        reason: "not allowed".into(),
                    },
                },
                ToolCallDecision {
                    call_id: "call-3".into(),
                    action: ToolCallAction::Modify {
                        new_input: serde_json::json!({"safe": true}),
                    },
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: ApprovalResponse = serde_json::from_str(&json).unwrap();
        if let ApprovalResponse::Batch { decisions } = decoded {
            assert_eq!(decisions.len(), 3);
            assert_eq!(decisions[0].call_id, "call-1");
        } else {
            panic!("expected Batch");
        }
    }

    #[test]
    fn tool_call_decision_round_trip() {
        let d = ToolCallDecision {
            call_id: "c1".into(),
            action: ToolCallAction::Approve,
        };
        let json = serde_json::to_string(&d).unwrap();
        let decoded: ToolCallDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.call_id, "c1");
    }
}
