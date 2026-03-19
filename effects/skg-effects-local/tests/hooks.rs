use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Artifact;
use layer0::effect::{Effect, EffectKind, HandoffContext, MemoryScope, Scope, SignalPayload};
use layer0::error::{OrchError, StateError};
use layer0::id::DispatchId;
use layer0::id::{OperatorId, WorkflowId};
use layer0::middleware::{StoreMiddleware, StoreStack, StoreWriteNext};
use layer0::state::{Lifetime, MemoryLink, StateStore, StoreOptions};
use layer0::test_utils::InMemoryStore;
use serde_json::json;
use skg_effects_core::Signalable;
use skg_effects_core::{EffectHandler, EffectOutcome, UnknownEffectPolicy};
use skg_effects_local::LocalEffectHandler;
use std::sync::Arc;
use tokio::sync::Mutex;

fn test_ctx() -> DispatchContext {
    DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"))
}

// ── Minimal no-op signaler ──────────────────────────────────────────────────

struct NoOpOrch;

#[async_trait]
impl Signalable for NoOpOrch {
    async fn signal(&self, _target: &WorkflowId, _signal: SignalPayload) -> Result<(), OrchError> {
        Ok(())
    }
}

// ── Test middleware ──────────────────────────────────────────────────────────

/// Guard that unconditionally blocks the write (does not call next).
struct HaltGuard;

#[async_trait]
impl StoreMiddleware for HaltGuard {
    async fn write(
        &self,
        _scope: &Scope,
        _key: &str,
        _value: serde_json::Value,
        _options: Option<&StoreOptions>,
        _next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        // Intentionally do not call next — skips the write.
        Ok(())
    }
}

/// Observer that records the key and value seen, then always calls next.
struct RecordingObserver {
    seen_key: Arc<Mutex<Option<String>>>,
    seen_value: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RecordingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen_key: Arc::new(Mutex::new(None)),
            seen_value: Arc::new(Mutex::new(None)),
        })
    }
}

#[async_trait]
impl StoreMiddleware for RecordingObserver {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        *self.seen_key.lock().await = Some(key.to_string());
        *self.seen_value.lock().await = Some(value.clone());
        next.write(scope, key, value, options).await
    }
}

/// Transformer that replaces the written value before calling next.
struct ModifyTransformer {
    new_value: serde_json::Value,
}

#[async_trait]
impl StoreMiddleware for ModifyTransformer {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        _value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        next.write(scope, key, self.new_value.clone(), options)
            .await
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Halt guardrail at `PreMemoryWrite` must prevent the write.
#[tokio::test]
async fn halt_hook_prevents_memory_write() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let stack = StoreStack::builder().guard(Arc::new(HaltGuard)).build();
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>))
        .with_store_middleware(stack);

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "secret".into(),
                value: json!("sensitive"),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: None,
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("halt is not an error — handle() succeeds");

    assert!(
        matches!(outcome, EffectOutcome::Skipped),
        "halt guard must produce Skipped outcome"
    );

    let got = state.read(&Scope::Global, "secret").await.expect("read ok");
    assert_eq!(got, None, "halt guard must prevent the write");
}

/// Without middleware the `WriteMemory` effect writes normally (regression guard).
#[tokio::test]
async fn no_hooks_writes_normally() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>));

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "k".into(),
                value: json!(42),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: None,
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "write must produce Applied outcome"
    );

    let got = state.read(&Scope::Global, "k").await.expect("read ok");
    assert_eq!(got, Some(json!(42)));
}

/// Observer middleware fires, sees the key and value, and write still succeeds.
#[tokio::test]
async fn observer_hook_sees_key_and_value() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let observer = RecordingObserver::new();
    let stack = StoreStack::builder().observe(observer.clone()).build();
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>))
        .with_store_middleware(stack);

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "observed_key".into(),
                value: json!({"x": 1}),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: None,
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "observer must not block — Applied"
    );

    assert_eq!(
        *observer.seen_key.lock().await,
        Some("observed_key".to_string()),
        "observer must see the key"
    );
    assert_eq!(
        *observer.seen_value.lock().await,
        Some(json!({"x": 1})),
        "observer must see the value"
    );

    // Write succeeded despite observer firing.
    let got = state
        .read(&Scope::Global, "observed_key")
        .await
        .expect("read ok");
    assert_eq!(got, Some(json!({"x": 1})));
}

/// Transformer middleware replaces the written value.
#[tokio::test]
async fn modify_hook_replaces_value() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let stack = StoreStack::builder()
        .transform(Arc::new(ModifyTransformer {
            new_value: json!("replaced"),
        }))
        .build();
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>))
        .with_store_middleware(stack);

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "m".into(),
                value: json!("original"),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: None,
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "transformer must commit — Applied"
    );

    let got = state.read(&Scope::Global, "m").await.expect("read ok");
    assert_eq!(
        got,
        Some(json!("replaced")),
        "transformer must replace the written value"
    );
}

// ── Lifetime-aware guardrail ────────────────────────────────────────────────

/// Blocks writes whose `options.lifetime` is `Transient`.
struct LifetimeGuardrail;

#[async_trait]
impl StoreMiddleware for LifetimeGuardrail {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        if options.and_then(|o| o.lifetime) == Some(Lifetime::Transient) {
            // Block: transient writes are not allowed.
            return Ok(());
        }
        next.write(scope, key, value, options).await
    }
}

/// Transient lifetime is blocked; durable lifetime is allowed.
#[tokio::test]
async fn lifetime_guardrail_blocks_transient_write() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let stack = StoreStack::builder()
        .guard(Arc::new(LifetimeGuardrail))
        .build();
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>))
        .with_store_middleware(stack);

    // Transient write: middleware must block it.
    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "transient_key".into(),
                value: serde_json::json!("should_not_land"),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: Some(Lifetime::Transient),
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("halt is not an error");

    assert!(
        matches!(outcome, EffectOutcome::Skipped),
        "transient write must be Skipped"
    );

    let got = state
        .read(&Scope::Global, "transient_key")
        .await
        .expect("read ok");
    assert_eq!(got, None, "transient write must be blocked");

    // Durable write: middleware must allow it.
    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::WriteMemory {
                scope: Scope::Global,
                key: "durable_key".into(),
                value: serde_json::json!("should_land"),
                memory_scope: MemoryScope::Session,
                tier: None,
                lifetime: Some(layer0::state::Lifetime::Durable),
                content_kind: None,
                salience: None,
                ttl: None,
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "durable write must be Applied"
    );

    let got = state
        .read(&Scope::Global, "durable_key")
        .await
        .expect("read ok");
    assert_eq!(
        got,
        Some(serde_json::json!("should_land")),
        "durable write must succeed"
    );
}

/// Handoff must forward `context.task` as the input message and `context.metadata`
/// as `input.metadata` on the resulting `OperatorInput`.
#[tokio::test]
async fn handoff_preserves_structured_state_in_metadata() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>));

    let handoff_meta = json!({
        "conversation_id": "abc-123",
        "context": { "depth": 3, "tags": ["urgent", "follow-up"] },
        "score": 0.95
    });

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::Handoff {
                operator: OperatorId::new("next-op"),
                context: HandoffContext {
                    task: Content::text("pick up from here"),
                    history: None,
                    metadata: Some(handoff_meta.clone()),
                },
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    match outcome {
        EffectOutcome::Handoff { operator, input } => {
            assert_eq!(operator, OperatorId::new("next-op"));
            assert_eq!(
                input.message.as_text().unwrap_or(""),
                "pick up from here",
                "task must be the input message"
            );
            assert_eq!(
                input.metadata, handoff_meta,
                "metadata must carry the original structured JSON"
            );
        }
        other => panic!("expected Handoff outcome, got {:?}", other),
    }
}

/// Effect::LinkMemory creates a graph link that is traversable via the store.
#[tokio::test]
async fn link_memory_effect_creates_graph_link() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>));

    let outcome = handler
        .handle(
            &Effect::new(0, EffectKind::LinkMemory {
                scope: Scope::Global,
                link: MemoryLink::new("notes/meeting", "decisions/arch", "references"),
            }),
            &test_ctx(),
        )
        .await
        .expect("handle ok");

    assert!(
        matches!(outcome, EffectOutcome::Applied),
        "LinkMemory must produce Applied outcome"
    );

    // Verify the link was created by traversing from the source key.
    let reachable = state
        .traverse(&Scope::Global, "notes/meeting", Some("references"), 1)
        .await
        .expect("traverse ok");
    assert_eq!(
        reachable,
        vec!["decisions/arch"],
        "link must be traversable from source to target"
    );
}

/// Progress and Artifact effects are caller-interpreted (routed via dispatch-channel
/// wiring through EffectEmitter → DispatchHandle, not EffectHandler). The handler must
/// skip them cleanly under `IgnoreAndWarn`.
#[tokio::test]
async fn progress_and_artifact_effects_skip_cleanly() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);
    let handler = LocalEffectHandler::new(state.clone(), Some(orch as Arc<dyn Signalable>))
        .with_unknown_policy(UnknownEffectPolicy::IgnoreAndWarn);

    // Progress effect must be skipped.
    let progress_outcome = handler
        .handle(
            &Effect::new(0, EffectKind::Progress {
                content: Content::text("step 1"),
            }),
            &test_ctx(),
        )
        .await
        .expect("Progress must not error under IgnoreAndWarn");

    assert!(
        matches!(progress_outcome, EffectOutcome::Skipped),
        "Progress effect must produce Skipped, got {:?}",
        progress_outcome,
    );

    // Artifact effect must be skipped.
    let artifact_outcome = handler
        .handle(
            &Effect::new(0, EffectKind::Artifact {
                artifact: Artifact::new("art-1", vec![Content::text("hello")]),
            }),
            &test_ctx(),
        )
        .await
        .expect("Artifact must not error under IgnoreAndWarn");

    assert!(
        matches!(artifact_outcome, EffectOutcome::Skipped),
        "Artifact effect must produce Skipped, got {:?}",
        artifact_outcome,
    );
}
