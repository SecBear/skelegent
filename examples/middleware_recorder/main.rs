//! middleware_recorder — record and inspect dispatch operations.
//!
//! Demonstrates the recorder middleware:
//! - Wires a DispatchRecorder into a DispatchStack via InMemorySink
//! - Runs several dispatches through the stack (one succeeds, one fails)
//! - Prints all recorded entries from the sink with phase, boundary, and payload
//!
//! Teaches: DispatchRecorder, InMemorySink, RecordEntry inspection.
//!
//! No API keys required. Runs entirely in-process.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::dispatch::{DispatchEvent, DispatchHandle};
use layer0::dispatch_context::DispatchContext;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::middleware::{DispatchNext, DispatchStack};
use layer0::operator::{OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TriggerType};
use skg_hook_recorder::{DispatchRecorder, InMemorySink};
use std::sync::Arc;

// ── Terminal ──────────────────────────────────────────────────────────────

struct EchoTerminal;

#[async_trait]
impl DispatchNext for EchoTerminal {
    async fn dispatch(
        &self,
        _ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, ProtocolError> {
        let output = OperatorOutput::new(
            input.message,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            },
        );
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("echo"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        Ok(handle)
    }
}

// ── Failing terminal (for demonstrating error recording) ──────────────────

struct FailTerminal;

#[async_trait]
impl DispatchNext for FailTerminal {
    async fn dispatch(
        &self,
        _ctx: &DispatchContext,
        _input: OperatorInput,
    ) -> Result<DispatchHandle, ProtocolError> {
        Err(ProtocolError::internal("simulated failure"))
    }
}

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Shared sink — clone the Arc to read recordings after dispatch.
    let sink = Arc::new(InMemorySink::new());

    // Wire DispatchRecorder as an observer.
    let recorder = DispatchRecorder::new(sink.clone());
    let stack = DispatchStack::builder().observe(Arc::new(recorder)).build();

    // --- Dispatch 1: success ---
    println!("=== Dispatch 1: success ===");
    let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::from("my-op"));
    let input = OperatorInput::new(Content::text("ping"), TriggerType::User);
    let _ = stack
        .dispatch_with(&ctx, input, &EchoTerminal)
        .await?
        .collect()
        .await?;

    // --- Dispatch 2: another success with different operator ---
    println!("=== Dispatch 2: success ===");
    let ctx = DispatchContext::new(DispatchId::new("d-002"), OperatorId::from("summarizer"));
    let input = OperatorInput::new(Content::text("summarize this"), TriggerType::Task);
    let _ = stack
        .dispatch_with(&ctx, input, &EchoTerminal)
        .await?
        .collect()
        .await?;

    // --- Dispatch 3: failure ---
    println!("=== Dispatch 3: failure ===");
    let ctx = DispatchContext::new(DispatchId::new("d-003"), OperatorId::from("my-op"));
    let input = OperatorInput::new(Content::text("will fail"), TriggerType::User);
    let _ = stack.dispatch_with(&ctx, input, &FailTerminal).await;

    // --- Print all recorded entries ---
    println!("\n=== Recorded entries ({} total) ===", sink.len().await);
    for (i, entry) in sink.entries().await.iter().enumerate() {
        println!(
            "[{i}] boundary={:?}  phase={:?}  operator={}  dispatch_id={}",
            entry.boundary, entry.phase, entry.context.operator_id, entry.context.dispatch_id,
        );
        if let Some(ms) = entry.duration_ms {
            println!("     duration={}ms", ms);
        }
        if let Some(err) = &entry.error {
            println!("     error={}", err);
        }
        println!("     payload={}", entry.payload_json);
    }

    Ok(())
}
