//! multi-agent — supervisor-style routing with peer-to-peer handoff.
//!
//! Demonstrates:
//!   - `CognitiveOperator<TestProvider>` — deterministic agents, no API keys required
//!   - `HandoffTool` — LLM sentinel detected by react_loop, emits Effect::Handoff
//!   - `SwarmOperator` — peer-to-peer handoff with explicit transition validation
//!   - `LocalOrch` — in-process dispatcher registering all three agents
//!   - Custom `ToolDyn` impls — `charge` (billing) and `lookup_ticket` (support)
//!
//! Flow for "I need help with my bill":
//!   1. triage-agent  → calls HandoffTool → Effect::Handoff to billing-agent
//!   2. billing-agent → calls ChargeTool  → confirms charge → Complete
//!
//! The support-agent is registered and reachable (triage→support transition is
//! declared) but not exercised by this input. Swap the triage provider response
//! to `transfer_to_support-agent` to route there instead.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::effect::EffectKind;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{Operator, OperatorInput, TriggerType};
use skg_context_engine::{CognitiveOperator, ReactLoopConfig};
use skg_orch_local::LocalOrch;
use skg_orch_patterns::{HandoffTool, SwarmOperator};
use skg_tool::{ApprovalPolicy, ToolDyn, ToolError, ToolRegistry};
use skg_turn::test_utils::{TestProvider, make_text_response, make_tool_call_response};

// ─── ChargeTool ──────────────────────────────────────────────────────────────

/// Billing tool that processes a charge against the customer's account.
///
/// In production this would call a payments API. Here it returns a fixed
/// success payload so the example runs without any external services.
struct ChargeTool;

impl ToolDyn for ChargeTool {
    fn name(&self) -> &str {
        "charge"
    }

    fn description(&self) -> &str {
        "Charge an amount to the customer's account and return a transaction ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "amount": {
                    "type": "string",
                    "description": "Amount in USD, e.g. \"50.00\""
                },
                "reason": {
                    "type": "string",
                    "description": "Human-readable reason for the charge"
                }
            },
            "required": ["amount"]
        })
    }

    fn call(
        &self,
        input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let amount = input
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0.00")
            .to_string();
        Box::pin(async move {
            Ok(serde_json::json!({
                "status": "success",
                "charged": amount,
                "transaction_id": "txn-001"
            }))
        })
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::None
    }
}

// ─── LookupTicketTool ────────────────────────────────────────────────────────

/// Support tool that retrieves a ticket from the issue tracker.
///
/// Returns a synthetic ticket so the example runs without any external services.
struct LookupTicketTool;

impl ToolDyn for LookupTicketTool {
    fn name(&self) -> &str {
        "lookup_ticket"
    }

    fn description(&self) -> &str {
        "Look up a support ticket by ID and return its current status and summary."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ticket_id": {
                    "type": "string",
                    "description": "The support ticket identifier"
                }
            },
            "required": ["ticket_id"]
        })
    }

    fn call(
        &self,
        input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let ticket_id = input
            .get("ticket_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Box::pin(async move {
            Ok(serde_json::json!({
                "ticket_id": ticket_id,
                "status": "open",
                "summary": "Customer reports login issues after password reset"
            }))
        })
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::None
    }
}

// ─── main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== multi-agent: triage → billing handoff demo ===\n");

    // ── triage-agent ─────────────────────────────────────────────────────────
    //
    // The triage agent receives the user's request and routes it to a specialist.
    //
    // Its TestProvider is pre-loaded with a single tool call response:
    // calling `transfer_to_billing-agent` — a HandoffTool. When react_loop
    // sees the `{"__handoff": true, ...}` sentinel it emits Effect::Handoff and
    // exits with ExitReason::HandedOff; the SwarmOperator then dispatches the
    // next agent named in the effect.
    let triage_provider = TestProvider::with_responses(vec![make_tool_call_response(
        "transfer_to_billing-agent",
        "call-001",
        serde_json::json!({ "reason": "Customer needs help with a billing charge" }),
    )]);
    let mut triage_tools = ToolRegistry::new();
    triage_tools.register(Arc::new(HandoffTool::new(
        OperatorId::new("billing-agent"),
        "Transfer to billing when the customer has a billing or payment question.",
    )));
    triage_tools.register(Arc::new(HandoffTool::new(
        OperatorId::new("support-agent"),
        "Transfer to support when the customer has a technical or account issue.",
    )));
    let triage_op = CognitiveOperator::new(
        "triage-agent",
        triage_provider,
        triage_tools,
        ReactLoopConfig {
            system_prompt: "You are a triage agent. Route billing questions to \
                            billing-agent and technical questions to support-agent."
                .into(),
            ..ReactLoopConfig::default()
        },
    );

    // ── billing-agent ────────────────────────────────────────────────────────
    //
    // The billing specialist receives the handoff and processes the payment.
    //
    // Two queued responses simulate a complete ReAct turn:
    //   round 1 → tool call to `charge` (agent decides to charge $50)
    //   round 2 → text confirmation (agent reports the result to the user)
    let billing_provider = TestProvider::with_responses(vec![
        make_tool_call_response(
            "charge",
            "call-002",
            serde_json::json!({ "amount": "50.00", "reason": "Outstanding invoice balance" }),
        ),
        make_text_response(
            "I've processed a charge of $50.00 to your account (txn-001). \
             Is there anything else I can help you with?",
        ),
    ]);
    let mut billing_tools = ToolRegistry::new();
    billing_tools.register(Arc::new(ChargeTool));
    let billing_op = CognitiveOperator::new(
        "billing-agent",
        billing_provider,
        billing_tools,
        ReactLoopConfig {
            system_prompt: "You are a billing specialist. Use the charge tool to process \
                            payments, then confirm the transaction to the customer."
                .into(),
            ..ReactLoopConfig::default()
        },
    );

    // ── support-agent ────────────────────────────────────────────────────────
    //
    // The support specialist handles technical and account issues. Not reached
    // by this input — registered so the triage→support transition is valid and
    // can be exercised by changing the triage provider's queued response.
    let support_provider = TestProvider::with_responses(vec![make_text_response(
        "I've looked up your ticket. Our team will follow up within 24 hours.",
    )]);
    let mut support_tools = ToolRegistry::new();
    support_tools.register(Arc::new(LookupTicketTool));
    let support_op = CognitiveOperator::new(
        "support-agent",
        support_provider,
        support_tools,
        ReactLoopConfig {
            system_prompt: "You are a support specialist. Use lookup_ticket to find the \
                            customer's issue and provide a resolution timeline."
                .into(),
            ..ReactLoopConfig::default()
        },
    );

    // ── orchestration ────────────────────────────────────────────────────────
    //
    // LocalOrch is the in-process Dispatcher. All three agents are registered
    // before the swarm is built so the dispatcher can route to any of them.
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("triage-agent"), Arc::new(triage_op));
    orch.register(OperatorId::new("billing-agent"), Arc::new(billing_op));
    orch.register(OperatorId::new("support-agent"), Arc::new(support_op));
    let orch = Arc::new(orch);

    // SwarmOperator validates every handoff against a declared adjacency map.
    // Undeclared transitions (e.g. billing→support without a declaration here)
    // are rejected with a hard error — no silent fallthrough.
    let swarm = SwarmOperator::builder(Arc::clone(&orch) as Arc<dyn Dispatcher>)
        .entry(OperatorId::new("triage-agent"))
        .transition(
            OperatorId::new("triage-agent"),
            OperatorId::new("billing-agent"),
        )
        .transition(
            OperatorId::new("triage-agent"),
            OperatorId::new("support-agent"),
        )
        .max_handoffs(3)
        .build();

    // ── execution ────────────────────────────────────────────────────────────

    let ctx = DispatchContext::new(DispatchId::new("demo"), OperatorId::new("swarm"));
    let input = OperatorInput::new(Content::text("I need help with my bill"), TriggerType::User);

    println!("User: I need help with my bill\n");
    println!("--- executing swarm (triage → billing) ---\n");

    let output = swarm.execute(input, &ctx).await?;

    // ── results ──────────────────────────────────────────────────────────────

    println!(
        "Final response:  {}",
        output.message.as_text().unwrap_or("(no text)")
    );
    println!("Exit reason:     {:?}", output.exit_reason);
    println!();
    println!("Effects ({} total):", output.effects.len());
    for (i, effect) in output.effects.iter().enumerate() {
        match &effect.kind {
            EffectKind::Handoff { operator, context } => {
                println!(
                    "  [{}] Handoff → {} | reason: \"{}\"",
                    i,
                    operator.as_str(),
                    context.task.as_text().unwrap_or("(none)")
                );
            }
            other => println!("  [{}] {:?}", i, other),
        }
    }

    Ok(())
}
