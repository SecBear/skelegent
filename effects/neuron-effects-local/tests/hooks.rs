use async_trait::async_trait;
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::error::{OrchError, StateError};
use layer0::id::{AgentId, WorkflowId};
use layer0::middleware::{StoreMiddleware, StoreStack, StoreWriteNext};
use layer0::operator::{ExitReason, OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::state::{Lifetime, StateStore, StoreOptions};
use layer0::test_utils::InMemoryStore;
use neuron_effects_core::EffectExecutor;
use neuron_effects_local::LocalEffectExecutor;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Minimal no-op orchestrator ──────────────────────────────────────────────

struct NoOpOrch;

#[async_trait]
impl Orchestrator for NoOpOrch {
    async fn dispatch(
        &self,
        _agent: &AgentId,
        _input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        Ok(OperatorOutput::new(
            layer0::content::Content::text("ok"),
            ExitReason::Complete,
        ))
    }

    async fn dispatch_many(
        &self,
        tasks: Vec<(AgentId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        tasks
            .into_iter()
            .map(|_| {
                Ok(OperatorOutput::new(
                    layer0::content::Content::text("ok"),
                    ExitReason::Complete,
                ))
            })
            .collect()
    }

    async fn signal(&self, _target: &WorkflowId, _signal: SignalPayload) -> Result<(), OrchError> {
        Ok(())
    }

    async fn query(
        &self,
        _target: &WorkflowId,
        _query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError> {
        Ok(serde_json::Value::Null)
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
    let exec = LocalEffectExecutor::new(state.clone(), orch).with_store_middleware(stack);

    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "secret".into(),
        value: json!("sensitive"),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("halt is not an error — execute() succeeds");

    let got = state.read(&Scope::Global, "secret").await.expect("read ok");
    assert_eq!(got, None, "halt guard must prevent the write");
}

/// Without middleware the `WriteMemory` effect writes normally (regression guard).
#[tokio::test]
async fn no_hooks_writes_normally() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);
    let exec = LocalEffectExecutor::new(state.clone(), orch);

    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "k".into(),
        value: json!(42),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("execute ok");

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
    let exec = LocalEffectExecutor::new(state.clone(), orch).with_store_middleware(stack);

    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "observed_key".into(),
        value: json!({"x": 1}),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("execute ok");

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
    let exec = LocalEffectExecutor::new(state.clone(), orch).with_store_middleware(stack);

    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "m".into(),
        value: json!("original"),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("execute ok");

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
    let exec = LocalEffectExecutor::new(state.clone(), orch).with_store_middleware(stack);

    // Transient write: middleware must block it.
    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "transient_key".into(),
        value: serde_json::json!("should_not_land"),
        tier: None,
        lifetime: Some(Lifetime::Transient),
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("halt is not an error");

    let got = state
        .read(&Scope::Global, "transient_key")
        .await
        .expect("read ok");
    assert_eq!(got, None, "transient write must be blocked");

    // Durable write: middleware must allow it.
    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "durable_key".into(),
        value: serde_json::json!("should_land"),
        tier: None,
        lifetime: Some(layer0::state::Lifetime::Durable),
        content_kind: None,
        salience: None,
        ttl: None,
    }])
    .await
    .expect("execute ok");

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
