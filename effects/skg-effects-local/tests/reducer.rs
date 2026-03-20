//! Tests for ReducerRegistry wiring inside LocalEffectHandler.

use layer0::DispatchContext;
use layer0::effect::{Effect, EffectKind, MemoryScope, Scope};
use layer0::id::{DispatchId, OperatorId};
use layer0::reducer::{AppendList, ReducerRegistry};
use layer0::state::StateStore;
use layer0::test_utils::InMemoryStore;
use serde_json::json;
use skg_effects_core::{EffectHandler, EffectOutcome};
use skg_effects_local::LocalEffectHandler;
use std::sync::Arc;

fn test_ctx() -> DispatchContext {
    DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"))
}

fn write_effect(key: &str, value: serde_json::Value) -> Effect {
    Effect::new(EffectKind::WriteMemory {
        scope: Scope::Global,
        key: key.to_string(),
        value,
        memory_scope: MemoryScope::Session,
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    })
}

/// AppendList reducer is applied on successive writes: [1] then [2] → [1,2].
#[tokio::test]
async fn reducer_applied_on_write() {
    let store = Arc::new(InMemoryStore::new());
    let registry = Arc::new(ReducerRegistry::new().register("items", AppendList));
    let handler = LocalEffectHandler::new(store.clone(), None).with_reducer_registry(registry);

    let ctx = test_ctx();

    let outcome = handler
        .handle(&write_effect("items", json!([1])), &ctx)
        .await
        .expect("first write failed");
    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "first write must produce Applied outcome"
    );

    let outcome = handler
        .handle(&write_effect("items", json!([2])), &ctx)
        .await
        .expect("second write failed");
    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "second write must produce Applied outcome"
    );

    let stored = store
        .read(&Scope::Global, "items")
        .await
        .expect("read failed")
        .expect("key missing");
    assert_eq!(stored, json!([1, 2]), "AppendList should have produced [1,2]");
}

/// Without a registry, successive writes are last-writer-wins (Overwrite default).
#[tokio::test]
async fn no_registry_is_overwrite() {
    let store = Arc::new(InMemoryStore::new());
    // No registry attached — should use Overwrite for every key.
    let handler = LocalEffectHandler::new(store.clone(), None);

    let ctx = test_ctx();

    handler
        .handle(&write_effect("score", json!(1)), &ctx)
        .await
        .expect("first write failed");

    handler
        .handle(&write_effect("score", json!(2)), &ctx)
        .await
        .expect("second write failed");

    let stored = store
        .read(&Scope::Global, "score")
        .await
        .expect("read failed")
        .expect("key missing");
    assert_eq!(stored, json!(2), "last-writer-wins Overwrite should yield 2");
}

/// A registry with no key-specific entries behaves identically to Overwrite.
#[tokio::test]
async fn reducer_registry_not_set_is_overwrite() {
    let store = Arc::new(InMemoryStore::new());
    // Registry with no registered keys — default is Overwrite.
    let registry = Arc::new(ReducerRegistry::new());
    let handler = LocalEffectHandler::new(store.clone(), None).with_reducer_registry(registry);

    let ctx = test_ctx();

    handler
        .handle(&write_effect("x", json!("first")), &ctx)
        .await
        .expect("first write failed");

    handler
        .handle(&write_effect("x", json!("second")), &ctx)
        .await
        .expect("second write failed");

    let stored = store
        .read(&Scope::Global, "x")
        .await
        .expect("read failed")
        .expect("key missing");
    assert_eq!(stored, json!("second"), "empty registry should overwrite");
}
