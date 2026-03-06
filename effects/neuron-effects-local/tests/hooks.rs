use async_trait::async_trait;
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::error::{HookError, OrchError};
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::id::{AgentId, WorkflowId};
use layer0::operator::{ExitReason, OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::state::{Lifetime, StateStore};
use layer0::test_utils::InMemoryStore;
use neuron_effects_core::EffectExecutor;
use neuron_effects_local::LocalEffectExecutor;
use neuron_hooks::HookRegistry;
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

// ── Test hooks ──────────────────────────────────────────────────────────────

struct HaltHook {
    points: Vec<HookPoint>,
    reason: String,
}

#[async_trait]
impl Hook for HaltHook {
    fn points(&self) -> &[HookPoint] {
        &self.points
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Ok(HookAction::Halt {
            reason: self.reason.clone(),
        })
    }
}

struct RecordingObserver {
    points: Vec<HookPoint>,
    seen_key: Arc<Mutex<Option<String>>>,
    seen_value: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RecordingObserver {
    fn new(point: HookPoint) -> Arc<Self> {
        Arc::new(Self {
            points: vec![point],
            seen_key: Arc::new(Mutex::new(None)),
            seen_value: Arc::new(Mutex::new(None)),
        })
    }
}

#[async_trait]
impl Hook for RecordingObserver {
    fn points(&self) -> &[HookPoint] {
        &self.points
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        *self.seen_key.lock().await = ctx.memory_key.clone();
        *self.seen_value.lock().await = ctx.memory_value.clone();
        Ok(HookAction::Continue)
    }
}

struct ModifyTransformer {
    points: Vec<HookPoint>,
    new_value: serde_json::Value,
}

#[async_trait]
impl Hook for ModifyTransformer {
    fn points(&self) -> &[HookPoint] {
        &self.points
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Ok(HookAction::ModifyDispatchOutput {
            new_output: self.new_value.clone(),
        })
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Halt guardrail at `PreMemoryWrite` must prevent the write.
#[tokio::test]
async fn halt_hook_prevents_memory_write() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(HaltHook {
        points: vec![HookPoint::PreMemoryWrite],
        reason: "blocked by policy".into(),
    }));

    let exec = LocalEffectExecutor::new(state.clone(), orch).with_hooks(Arc::new(registry));

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
    assert_eq!(got, None, "halt hook must prevent the write");
}

/// Without hooks the `WriteMemory` effect writes normally (regression guard).
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

/// Observer hook fires, sees the key and value, and write still succeeds.
#[tokio::test]
async fn observer_hook_sees_key_and_value() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let observer = RecordingObserver::new(HookPoint::PreMemoryWrite);
    let mut registry = HookRegistry::new();
    registry.add_observer(observer.clone());

    let exec = LocalEffectExecutor::new(state.clone(), orch).with_hooks(Arc::new(registry));

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

/// Transformer returning `ModifyDispatchOutput` replaces the written value.
#[tokio::test]
async fn modify_hook_replaces_value() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let mut registry = HookRegistry::new();
    registry.add_transformer(Arc::new(ModifyTransformer {
        points: vec![HookPoint::PreMemoryWrite],
        new_value: json!("replaced"),
    }));

    let exec = LocalEffectExecutor::new(state.clone(), orch).with_hooks(Arc::new(registry));

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

/// Blocks writes whose `memory_options.lifetime` is `Transient`.
struct LifetimeGuardrail;

#[async_trait]
impl Hook for LifetimeGuardrail {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreMemoryWrite]
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        if ctx.memory_options.as_ref().and_then(|o| o.lifetime) == Some(Lifetime::Transient) {
            return Ok(HookAction::Halt {
                reason: "transient blocked".into(),
            });
        }
        Ok(HookAction::Continue)
    }
}

/// Transient lifetime is blocked; durable lifetime is allowed.
#[tokio::test]
async fn lifetime_guardrail_blocks_transient_write() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(NoOpOrch);

    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(LifetimeGuardrail));

    let exec = LocalEffectExecutor::new(state.clone(), orch).with_hooks(Arc::new(registry));

    // Transient write: hook must block it.
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

    // Durable write: hook must allow it.
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
