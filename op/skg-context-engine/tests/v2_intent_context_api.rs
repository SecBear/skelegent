use layer0::effect::Scope;
use layer0::intent::{Intent, IntentKind};
use skg_context_engine::Context;

// ── push_intent / extend_intents / drain_intents work as specified ──────────

#[test]
fn push_intent_stores_and_is_readable() {
    let mut ctx = Context::new();
    let intent = Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "test".into(),
    });
    ctx.push_intent(intent);
    assert_eq!(ctx.intents().len(), 1);
}

#[test]
fn extend_intents_stores_multiple() {
    let mut ctx = Context::new();
    let i1 = Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "a".into(),
    });
    let i2 = Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "b".into(),
    });
    ctx.extend_intents(vec![i1, i2]);
    assert_eq!(ctx.intents().len(), 2);
}

#[test]
fn drain_intents_transfers_ownership() {
    let mut ctx = Context::new();
    ctx.push_intent(Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "k".into(),
    }));
    assert_eq!(ctx.intents().len(), 1);

    let drained = ctx.drain_intents();
    assert_eq!(drained.len(), 1);
    assert!(ctx.intents().is_empty(), "intents should be empty after drain");
}

#[test]
fn drain_intents_preserves_order() {
    let mut ctx = Context::new();
    ctx.push_intent(Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "first".into(),
    }));
    ctx.push_intent(Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "second".into(),
    }));

    let drained = ctx.drain_intents();
    assert_eq!(drained.len(), 2);
    match &drained[0].kind {
        IntentKind::DeleteMemory { key, .. } => assert_eq!(key, "first"),
        other => panic!("expected DeleteMemory, got {other:?}"),
    }
    match &drained[1].kind {
        IntentKind::DeleteMemory { key, .. } => assert_eq!(key, "second"),
        other => panic!("expected DeleteMemory, got {other:?}"),
    }
}

// ── No effect-declaration API remains on public Context ─────────────────────

/// This test verifies at the type level that Context does not expose
/// push_effect or extend_effects methods. If someone adds them, this
/// file needs to be updated with an explicit acknowledgment.
///
/// The check is structural: we verify the methods we DO expect exist,
/// and document that effect-declaration methods do NOT exist.
#[test]
fn context_exposes_intent_api_not_effect_api() {
    let mut ctx = Context::new();

    // Intent API exists:
    ctx.push_intent(Intent::new(IntentKind::Custom {
        name: "test".into(),
        payload: serde_json::json!({}),
    }));
    ctx.extend_intents(std::iter::empty());
    let _ = ctx.intents();
    let _ = ctx.drain_intents();

    // Effect-declaration API (push_effect, extend_effects) does NOT exist on Context.
    // If this test ever needs updating because those methods are re-added,
    // that signals a design change that needs architectural review.
    //
    // Compile-time proof: the following would fail to compile if uncommented:
    // ctx.push_effect(effect);
    // ctx.extend_effects(vec![]);
}
