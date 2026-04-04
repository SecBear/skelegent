//! workflow — sequential + parallel composition example.
//!
//! Demonstrates [`WorkflowBuilder`] composing four operators into a pipeline:
//!
//! ```text
//!  Input
//!    │
//!    ▼
//! [research]           ← Step 1: sequential
//!    │
//!    ▼
//! [analyze] [summarize] ← Step 2: parallel fan-out (same input, both run)
//!    │        │
//!    └──┬─────┘         (default reducer joins outputs with newline)
//!       │
//!       ▼
//!  [format]            ← Step 3: sequential
//!       │
//!       ▼
//!   Output
//! ```
//!
//! No API keys required — all operators are pure in-process functions.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{Operator, OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TriggerType};
use skg_orch_local::LocalOrch;
use skg_orch_patterns::WorkflowBuilder;
use std::sync::Arc;

// ── Operator definitions ──────────────────────────────────────────────────────

/// Step 1: simulate a research phase.
///
/// Takes a topic as input, returns a canned research summary.
struct ResearchOp;

#[async_trait]
impl Operator for ResearchOp {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let topic = input.message.as_text().unwrap_or("unknown topic");
        let result = format!("Research findings on {topic}");
        Ok(OperatorOutput::new(
            Content::text(result),
            Outcome::Terminal { terminal: TerminalOutcome::Completed },
        ))
    }
}

/// Step 2a (parallel branch): simulate analysis.
///
/// Receives research output and produces a structured analysis.
struct AnalyzeOp;

#[async_trait]
impl Operator for AnalyzeOp {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let text = input.message.as_text().unwrap_or("");
        let result = format!("Analysis of: {text}");
        Ok(OperatorOutput::new(
            Content::text(result),
            Outcome::Terminal { terminal: TerminalOutcome::Completed },
        ))
    }
}

/// Step 2b (parallel branch): simulate summarization.
///
/// Receives the same research output as AnalyzeOp and produces a short summary.
struct SummarizeOp;

#[async_trait]
impl Operator for SummarizeOp {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let text = input.message.as_text().unwrap_or("");
        let result = format!("Summary: {text}");
        Ok(OperatorOutput::new(
            Content::text(result),
            Outcome::Terminal { terminal: TerminalOutcome::Completed },
        ))
    }
}

/// Step 3: format the combined output from the parallel branches.
///
/// The default reducer joins parallel outputs with a newline, so this operator
/// receives both analysis and summary as a single block of text.
struct FormatOp;

#[async_trait]
impl Operator for FormatOp {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        let text = input.message.as_text().unwrap_or("");
        let result = format!("Formatted:\n{text}");
        Ok(OperatorOutput::new(
            Content::text(result),
            Outcome::Terminal { terminal: TerminalOutcome::Completed },
        ))
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Register all operators with a LocalOrch.
    //
    //    LocalOrch must be mutable during registration, then frozen into an
    //    Arc<dyn Dispatcher> before it can be shared with WorkflowBuilder.
    let research_id = OperatorId::new("research");
    let analyze_id = OperatorId::new("analyze");
    let summarize_id = OperatorId::new("summarize");
    let format_id = OperatorId::new("format");

    let mut orch = LocalOrch::new();
    orch.register(research_id.clone(), Arc::new(ResearchOp));
    orch.register(analyze_id.clone(), Arc::new(AnalyzeOp));
    orch.register(summarize_id.clone(), Arc::new(SummarizeOp));
    orch.register(format_id.clone(), Arc::new(FormatOp));

    let dispatcher: Arc<dyn Dispatcher> = Arc::new(orch);

    // 2. Build the workflow using WorkflowBuilder.
    //
    //    The builder compiles the step list into a single Arc<dyn Operator>:
    //    - step()     → SingleDispatch (sequential)
    //    - parallel() → ParallelOperator with the default concatenating reducer
    //    - Two or more compiled steps are wrapped in a Pipeline that chains them.
    let workflow = WorkflowBuilder::new(Arc::clone(&dispatcher))
        .step(research_id) // Step 1: research
        .parallel(vec![analyze_id, summarize_id]) // Step 2: analyze + summarize concurrently
        .step(format_id) // Step 3: format combined result
        .build();

    // 3. Execute the workflow with a topic as the initial input.
    let topic = "agentic AI runtimes";
    let input = OperatorInput::new(Content::text(topic), TriggerType::User);
    // The root DispatchContext identifies this top-level invocation. Each
    // step the Pipeline creates a child context with the step's operator ID.
    let ctx = DispatchContext::new(
        DispatchId::new("workflow-run-1"),
        OperatorId::new("workflow"),
    );

    println!("=== Workflow: sequential → parallel → sequential ===");
    println!("Input topic: {topic}\n");

    let output = workflow.execute(input, &ctx).await?;

    // 4. Print the final output.
    //
    //    The pipeline accumulates all effects across steps; the content here is
    //    whatever FormatOp produced from the joined analyze + summarize text.
    println!("--- Final output ---");
    println!("{}", output.message.as_text().unwrap_or("(no text)"));
    println!("\nOutcome: {:?}", output.outcome);

    Ok(())
}
