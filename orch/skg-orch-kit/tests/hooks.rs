use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::error::StateError;
use layer0::id::{DispatchId, OperatorId};
use layer0::middleware::{StoreMiddleware, StoreStack, StoreWriteNext};
use layer0::state::StateStore;
use layer0::state::StoreOptions;
use layer0::test_utils::InMemoryStore;
use layer0::{Intent, IntentKind, MemoryScope, Scope};
use serde_json::json;
use skg_effects_core::{EffectHandler, EffectOutcome};
use skg_effects_local::LocalEffectHandler;
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

/// Halt guardrail at `PreMemoryWrite` skips the write and produces
/// `EffectOutcome::Skipped` (no trace event).
#[tokio::test]
async fn handler_halt_hook_skips_write() {
    let state = Arc::new(InMemoryStore::new());

    let stack = StoreStack::builder()
        .guard(Arc::new(HaltMiddleware))
        .build();

    let handler = LocalEffectHandler::new(state.clone(), None).with_store_middleware(stack);

    let effect = Intent::new(IntentKind::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: json!("v"),
        memory_scope: MemoryScope::Session,
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    });

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
    let outcome = handler
        .handle(&effect, &ctx)
        .await
        .expect("handle ok — halt is not an error");

    assert!(
        matches!(outcome, EffectOutcome::Skipped),
        "halt hook must produce Skipped outcome"
    );

    let got = state.read(&Scope::Global, "k").await.expect("read ok");
    assert_eq!(got, None, "halt hook must prevent the state write");
}

/// Without hooks, `WriteMemory` writes and returns `EffectOutcome::Applied`.
#[tokio::test]
async fn handler_no_hooks_writes_normally() {
    let state = Arc::new(InMemoryStore::new());
    let handler = LocalEffectHandler::new(state.clone(), None);

    let effect = Intent::new(IntentKind::WriteMemory {
        scope: Scope::Global,
        key: "k2".into(),
        value: json!(99),
        memory_scope: MemoryScope::Session,
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    });

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
    let outcome = handler.handle(&effect, &ctx).await.expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "expected Applied outcome"
    );

    let got = state.read(&Scope::Global, "k2").await.expect("read ok");
    assert_eq!(got, Some(json!(99)));
}
