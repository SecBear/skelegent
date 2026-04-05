//! middleware_approval — approval guard blocking dispatches.
//!
//! Demonstrates an inline approval-guard pattern:
//! - An operator denylist guard that blocks specific operator IDs
//! - A cost-budget guard that tracks per-session spend and halts when exceeded
//! - Allowed dispatches pass through; blocked ones return a descriptive error
//!
//! Teaches: guard middleware, short-circuiting the chain, stateful guards,
//! the PolicyDecision pattern (approve vs deny with reason).
//!
//! No API keys required. Runs entirely in-process.
//! NOTE: skg-hook-approval lives in the extras repo and is not available here.
//! This example inlines equivalent guard logic to demonstrate the pattern.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::dispatch::{DispatchEvent, DispatchHandle};
use layer0::dispatch_context::DispatchContext;
use layer0::error::ProtocolError;
use layer0::id::{DispatchId, OperatorId};
use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
use layer0::operator::{OperatorInput, OperatorOutput, Outcome, TerminalOutcome, TriggerType};
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

// ── PolicyDecision: approve or deny with reason ────────────────────────────

enum PolicyDecision {
    Approve,
    Deny { reason: String },
}

// ── Guard 1: denylist certain operator IDs ────────────────────────────────

struct DenylistGuard {
    denied: HashSet<String>,
}

impl DenylistGuard {
    fn new(denied: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            denied: denied.into_iter().map(String::from).collect(),
        }
    }

    fn check(&self, ctx: &DispatchContext) -> PolicyDecision {
        let op = ctx.operator_id.to_string();
        if self.denied.contains(&op) {
            PolicyDecision::Deny {
                reason: format!("operator '{op}' is on the denylist"),
            }
        } else {
            PolicyDecision::Approve
        }
    }
}

#[async_trait]
impl DispatchMiddleware for DenylistGuard {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, ProtocolError> {
        match self.check(ctx) {
            PolicyDecision::Approve => next.dispatch(ctx, input).await,
            PolicyDecision::Deny { reason } => {
                println!("[denylist-guard] DENIED: {reason}");
                Err(ProtocolError::internal(reason))
            }
        }
    }
}

// ── Guard 2: budget limit — max N dispatches before halting ───────────────

struct BudgetGuard {
    limit: u32,
    used: Arc<AtomicU32>,
}

impl BudgetGuard {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            used: Arc::new(AtomicU32::new(0)),
        }
    }

    fn check(&self) -> PolicyDecision {
        let used = self.used.fetch_add(1, Ordering::SeqCst);
        if used >= self.limit {
            PolicyDecision::Deny {
                reason: format!(
                    "budget exhausted: used {used} of {} allowed dispatches",
                    self.limit
                ),
            }
        } else {
            PolicyDecision::Approve
        }
    }
}

#[async_trait]
impl DispatchMiddleware for BudgetGuard {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, ProtocolError> {
        match self.check() {
            PolicyDecision::Approve => next.dispatch(ctx, input).await,
            PolicyDecision::Deny { reason } => {
                println!("[budget-guard]   DENIED: {reason}");
                Err(ProtocolError::internal(reason))
            }
        }
    }
}

// ── Terminal ──────────────────────────────────────────────────────────────

struct EchoTerminal;

#[async_trait]
impl DispatchNext for EchoTerminal {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, ProtocolError> {
        println!("[terminal]       ALLOWED: operator={}", ctx.operator_id);
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

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Stack: denylist guard -> budget guard (both innermost / guard slot).
    // Budget is 2 dispatches; "dangerous-op" is always denied.
    let stack = DispatchStack::builder()
        .guard(Arc::new(DenylistGuard::new(["dangerous-op", "exec-shell"])))
        .guard(Arc::new(BudgetGuard::new(2)))
        .build();

    let terminal = EchoTerminal;

    let dispatch = |id: &str, op: &str, msg: &str| {
        (
            DispatchContext::new(DispatchId::new(id), OperatorId::from(op)),
            OperatorInput::new(Content::text(msg), TriggerType::User),
        )
    };

    // --- 1: allowed ---
    println!("\n=== Dispatch 1: safe-op (should pass) ===");
    let (ctx, input) = dispatch("d-001", "safe-op", "hello");
    match stack.dispatch_with(&ctx, input, &terminal).await {
        Ok(h) => {
            let out = h.collect().await?;
            println!("[main] result: {:?}", out.message.as_text());
        }
        Err(e) => println!("[main] blocked: {e}"),
    }

    // --- 2: denied by denylist ---
    println!("\n=== Dispatch 2: dangerous-op (should be denied) ===");
    let (ctx, input) = dispatch("d-002", "dangerous-op", "do something bad");
    match stack.dispatch_with(&ctx, input, &terminal).await {
        Ok(_) => println!("[main] unexpectedly allowed"),
        Err(e) => println!("[main] blocked as expected: {e}"),
    }

    // --- 3: allowed (budget=1 of 2 used) ---
    println!("\n=== Dispatch 3: safe-op again (budget: 1/2) ===");
    let (ctx, input) = dispatch("d-003", "safe-op", "second call");
    match stack.dispatch_with(&ctx, input, &terminal).await {
        Ok(h) => {
            let out = h.collect().await?;
            println!("[main] result: {:?}", out.message.as_text());
        }
        Err(e) => println!("[main] blocked: {e}"),
    }

    // --- 4: budget exhausted ---
    println!("\n=== Dispatch 4: safe-op (budget exhausted) ===");
    let (ctx, input) = dispatch("d-004", "safe-op", "third call - over budget");
    match stack.dispatch_with(&ctx, input, &terminal).await {
        Ok(_) => println!("[main] unexpectedly allowed"),
        Err(e) => println!("[main] blocked as expected: {e}"),
    }

    Ok(())
}
