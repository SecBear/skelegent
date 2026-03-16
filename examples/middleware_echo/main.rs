//! middleware_echo — minimal middleware demo.
//!
//! Demonstrates the simplest possible middleware composition:
//! - A logging observer that prints every dispatch
//! - A keyword guard that blocks dispatches containing "forbidden"
//! - A passthrough echo terminal that returns the input as output
//!
//! Teaches: DispatchMiddleware, DispatchStack builder, observer/guard pattern.
//!
//! No API keys required. Runs entirely in-process.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::dispatch::{DispatchEvent, DispatchHandle};
use layer0::dispatch_context::DispatchContext;
use layer0::error::OrchError;
use layer0::id::{DispatchId, OperatorId};
use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
use layer0::operator::{ExitReason, OperatorInput, OperatorOutput, TriggerType};
use std::sync::Arc;

// ── Terminal (the "real" handler at the end of the chain) ──────────────────

struct EchoTerminal;

#[async_trait]
impl DispatchNext for EchoTerminal {
    async fn dispatch(
        &self,
        _ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let output = OperatorOutput::new(input.message, ExitReason::Complete);
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("echo"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        Ok(handle)
    }
}

// ── Observer: logs every dispatch ─────────────────────────────────────────

struct LogObserver;

#[async_trait]
impl DispatchMiddleware for LogObserver {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, OrchError> {
        println!(
            "[observer] dispatch start  operator={} dispatch_id={}",
            ctx.operator_id, ctx.dispatch_id
        );
        let result = next.dispatch(ctx, input).await;
        match &result {
            Ok(_) => println!("[observer] dispatch ok"),
            Err(e) => println!("[observer] dispatch err: {e}"),
        }
        result
    }
}

// ── Guard: blocks inputs that contain "forbidden" ─────────────────────────

struct KeywordGuard;

#[async_trait]
impl DispatchMiddleware for KeywordGuard {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, OrchError> {
        let text = input.message.as_text().unwrap_or("");
        if text.contains("forbidden") {
            println!("[guard]    blocked: input contains 'forbidden'");
            return Err(OrchError::DispatchFailed("keyword blocked".into()));
        }
        next.dispatch(ctx, input).await
    }
}

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build the stack: observer (outermost) -> guard (innermost).
    let stack = DispatchStack::builder()
        .observe(Arc::new(LogObserver))
        .guard(Arc::new(KeywordGuard))
        .build();

    let terminal = EchoTerminal;

    // --- Dispatch 1: allowed ---
    println!("\n=== Dispatch 1: allowed message ===");
    let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::from("echo-op"));
    let input = OperatorInput::new(Content::text("hello, world"), TriggerType::User);
    let output = stack
        .dispatch_with(&ctx, input, &terminal)
        .await?
        .collect()
        .await?;
    println!(
        "[main]     result: {:?}",
        output.message.as_text().unwrap_or("(no text)")
    );

    // --- Dispatch 2: blocked by guard ---
    println!("\n=== Dispatch 2: blocked message ===");
    let ctx = DispatchContext::new(DispatchId::new("d-002"), OperatorId::from("echo-op"));
    let input = OperatorInput::new(
        Content::text("this is a forbidden request"),
        TriggerType::User,
    );
    match stack.dispatch_with(&ctx, input, &terminal).await {
        Ok(_) => println!("[main]     unexpectedly succeeded"),
        Err(e) => println!("[main]     blocked as expected: {e}"),
    }

    Ok(())
}
