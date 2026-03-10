use async_trait::async_trait;
use layer0::effect::{Effect, Scope};
use layer0::error::StateError;
use layer0::id::OperatorId;
use layer0::middleware::{StoreMiddleware, StoreStack, StoreWriteNext};
use layer0::state::{StateStore, StoreOptions};
use layer0::test_utils::InMemoryStore;
use serde_json::json;
use skg_orch_kit::{EffectInterpreter, ExecutionEvent, ExecutionTrace, LocalEffectInterpreter};
use std::sync::Arc;

// ── Test middleware ──────────────────────────────────────────────────────────

/// Guard that silently blocks every write without error.
struct HaltMiddleware;

#[async_trait]
impl StoreMiddleware for HaltMiddleware {
    async fn write(
        &self,
        _scope: &Scope,
        _key: &str,
        _value: serde_json::Value,
        _options: Option<&StoreOptions>,
        _next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        // Do not call next — silently skip the write (not an error).
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Halt guardrail at `PreMemoryWrite` skips the write and produces no
/// `MemoryWritten` trace event.
#[tokio::test]
async fn interpreter_halt_hook_skips_write() {
    let state = Arc::new(InMemoryStore::new());

    let stack = StoreStack::builder()
        .guard(Arc::new(HaltMiddleware))
        .build();

    let interp = LocalEffectInterpreter::new(state.clone()).with_store_middleware(stack);

    let effect = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: json!("v"),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };

    let mut followups = vec![];
    let mut trace = ExecutionTrace::new();

    interp
        .execute_effect(&effect, &mut followups, &mut trace)
        .await
        .expect("execute_effect ok — halt is not an error");

    assert!(
        !trace
            .events
            .iter()
            .any(|e| matches!(e, ExecutionEvent::MemoryWritten { .. })),
        "halt hook must suppress the MemoryWritten trace event"
    );

    let got = state.read(&Scope::Global, "k").await.expect("read ok");
    assert_eq!(got, None, "halt hook must prevent the state write");
}

/// Without hooks, `WriteMemory` writes and emits `MemoryWritten`.
#[tokio::test]
async fn interpreter_no_hooks_writes_normally() {
    let state = Arc::new(InMemoryStore::new());
    let interp = LocalEffectInterpreter::new(state.clone());

    let effect = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k2".into(),
        value: json!(99),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };

    let mut followups: Vec<(OperatorId, layer0::operator::OperatorInput)> = vec![];
    let mut trace = ExecutionTrace::new();

    interp
        .execute_effect(&effect, &mut followups, &mut trace)
        .await
        .expect("execute_effect ok");

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e, ExecutionEvent::MemoryWritten { key } if key == "k2")),
        "expected MemoryWritten event in trace"
    );

    let got = state.read(&Scope::Global, "k2").await.expect("read ok");
    assert_eq!(got, Some(json!(99)));
}
