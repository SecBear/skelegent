use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::{DispatchEvent, DispatchHandle, Dispatcher};
use layer0::effect::{Effect, EffectKind, HandoffContext, MemoryScope, Scope, SignalPayload};
use layer0::error::{OperatorError, OrchError, StateError};
use layer0::id::{OperatorId, WorkflowId};
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};
use layer0::state::{SearchResult, StateStore};
use serde_json::json;
use skg_effects_core::Signalable;
use skg_effects_local::LocalEffectHandler;
use skg_orch_kit::{Kit, KitError, OrchestratedRunner};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

struct SimpleOrch {
    agents: HashMap<String, Arc<dyn Operator>>,
    signals: Mutex<Vec<(WorkflowId, SignalPayload)>>,
}

impl SimpleOrch {
    fn new() -> Self {
        Self {
            agents: HashMap::new(),
            signals: Mutex::new(vec![]),
        }
    }

    fn register(&mut self, id: &str, op: Arc<dyn Operator>) {
        self.agents.insert(id.to_string(), op);
    }

    async fn recorded_signals(&self) -> Vec<(WorkflowId, SignalPayload)> {
        self.signals.lock().await.clone()
    }
}

#[async_trait]
impl Dispatcher for SimpleOrch {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let op = self
            .agents
            .get(ctx.operator_id.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(ctx.operator_id.to_string()))?
            .clone();
        let (handle, sender) = DispatchHandle::channel(ctx.dispatch_id.clone());
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match op.execute(input, &ctx).await {
                Ok(output) => {
                    let _ = sender.send(DispatchEvent::Completed { output }).await;
                }
                Err(err) => {
                    let _ = sender
                        .send(DispatchEvent::Failed {
                            error: OrchError::OperatorError(err),
                        })
                        .await;
                }
            }
        });
        Ok(handle)
    }
}

#[async_trait]
impl Signalable for SimpleOrch {
    async fn signal(&self, target: &WorkflowId, signal: SignalPayload) -> Result<(), OrchError> {
        self.signals.lock().await.push((target.clone(), signal));
        Ok(())
    }
}

struct TestStore {
    data: RwLock<HashMap<String, serde_json::Value>>,
    ops: Mutex<Vec<String>>,
}

impl TestStore {
    fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            ops: Mutex::new(vec![]),
        }
    }

    async fn read_raw(&self, key: &str) -> Option<serde_json::Value> {
        self.data.read().await.get(key).cloned()
    }

    async fn ops(&self) -> Vec<String> {
        self.ops.lock().await.clone()
    }
}

#[async_trait]
impl StateStore for TestStore {
    async fn read(
        &self,
        _scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        Ok(self.data.read().await.get(key).cloned())
    }

    async fn write(
        &self,
        _scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        self.data.write().await.insert(key.to_string(), value);
        self.ops.lock().await.push(format!("write:{key}"));
        Ok(())
    }

    async fn delete(&self, _scope: &Scope, key: &str) -> Result<(), StateError> {
        self.data.write().await.remove(key);
        self.ops.lock().await.push(format!("delete:{key}"));
        Ok(())
    }

    async fn list(&self, _scope: &Scope, _prefix: &str) -> Result<Vec<String>, StateError> {
        Ok(vec![])
    }

    async fn search(
        &self,
        _scope: &Scope,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        Ok(vec![])
    }
}

struct WriterOperator;

#[async_trait]
impl Operator for WriterOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("wrote"), ExitReason::Complete);
        output.effects.push(Effect::new(EffectKind::WriteMemory {
            scope: Scope::Global,
            key: "k1".into(),
            value: json!({"v": 1}),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        }));
        output.effects.push(Effect::new(EffectKind::Signal {
            target: WorkflowId::new("wf1"),
            payload: SignalPayload::new("sig.type", json!({"ok": true})),
        }));
        Ok(output)
    }
}

struct DelegateOperator;

#[async_trait]
impl Operator for DelegateOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("delegating"), ExitReason::Complete);
        output.effects.push(Effect::new(EffectKind::Delegate {
            operator: OperatorId::new("child"),
            input: Box::new(OperatorInput::new(
                Content::text("child task"),
                TriggerType::Task,
            )),
        }));
        Ok(output)
    }
}

struct ChildOperator;

#[async_trait]
impl Operator for ChildOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        assert_eq!(input.message.as_text().unwrap_or_default(), "child task");
        Ok(OperatorOutput::new(
            Content::text("child done"),
            ExitReason::Complete,
        ))
    }
}

struct HandoffOperator;

#[async_trait]
impl Operator for HandoffOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("handoff"), ExitReason::Complete);
        output.effects.push(Effect::new(EffectKind::Handoff {
            operator: OperatorId::new("handoff_target"),
            context: HandoffContext {
                task: Content::text(""),
                history: None,
                metadata: Some(json!({"ticket": 123})),
            },
        }));
        Ok(output)
    }
}

struct HandoffTargetOperator;

#[async_trait]
impl Operator for HandoffTargetOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        // LocalEffectHandler routes context.metadata → input.metadata.
        // The ticket field must be present in the structured metadata.
        assert_eq!(input.metadata.get("ticket").and_then(|v| v.as_i64()), Some(123));
        Ok(OperatorOutput::new(
            Content::text("accepted"),
            ExitReason::Complete,
        ))
    }
}

struct FullPipelineRootOperator;

#[async_trait]
impl Operator for FullPipelineRootOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("root"), ExitReason::Complete);
        output.effects.push(Effect::new(EffectKind::WriteMemory {
            scope: Scope::Global,
            key: "k-pipeline".into(),
            value: json!({"v": 42}),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        }));
        output.effects.push(Effect::new(EffectKind::Delegate {
            operator: OperatorId::new("child"),
            input: Box::new(OperatorInput::new(
                Content::text("child task"),
                TriggerType::Task,
            )),
        }));
        output.effects.push(Effect::new(EffectKind::Handoff {
            operator: OperatorId::new("handoff_target"),
            context: HandoffContext {
                task: Content::text(""),
                history: None,
                metadata: Some(json!({"ticket": 123})),
            },
        }));
        output.effects.push(Effect::new(EffectKind::Signal {
            target: WorkflowId::new("wf-pipeline"),
            payload: SignalPayload::new("pipeline.signal", json!({"ok": true})),
        }));
        output.effects.push(Effect::new(EffectKind::DeleteMemory {
            scope: Scope::Global,
            key: "k-pipeline".into(),
        }));
        Ok(output)
    }
}

#[tokio::test]
async fn runner_executes_memory_and_signal_effects() {
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(WriterOperator));
    let orch = Arc::new(orch);

    let state = Arc::new(TestStore::new());
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    assert_eq!(trace.outputs.len(), 1);
    assert_eq!(state.read_raw("k1").await, Some(json!({"v": 1})));

    // Signal is sent by the handler via Signalable::signal and recorded by our orch.
    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf1"));
    assert_eq!(signals[0].1.signal_type, "sig.type");
    assert_eq!(signals[0].1.data, json!({"ok": true}));
}

#[tokio::test]
async fn runner_enqueues_and_executes_delegate() {
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(DelegateOperator));
    orch.register("child", Arc::new(ChildOperator));

    let state = Arc::new(TestStore::new());
    let orch = Arc::new(orch);
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    assert_eq!(trace.outputs.len(), 2, "root + child outputs expected");
    assert_eq!(trace.outputs[0].message.as_text().unwrap(), "delegating");
    assert_eq!(trace.outputs[1].message.as_text().unwrap(), "child done");
}

#[tokio::test]
async fn runner_enqueues_and_executes_handoff() {
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(HandoffOperator));
    orch.register("handoff_target", Arc::new(HandoffTargetOperator));

    let orch = Arc::new(orch);
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::new(TestStore::new()) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    assert_eq!(trace.outputs.len(), 2);
    assert_eq!(trace.outputs[0].message.as_text().unwrap(), "handoff");
    assert_eq!(trace.outputs[1].message.as_text().unwrap(), "accepted");
}

#[tokio::test]
async fn kit_local_runner_requires_state_backend() {
    let orch: Arc<SimpleOrch> = Arc::new(SimpleOrch::new());
    let kit = Kit::new(orch.clone() as Arc<dyn Dispatcher>)
        .with_signaler(orch.clone() as Arc<dyn Signalable>);
    let err = match kit.local_runner() {
        Ok(_) => panic!("expected error, got Ok"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("requires a state backend"));
}

#[tokio::test]
async fn unknown_agent_returns_agent_not_found() {
    let orch: Arc<SimpleOrch> = Arc::new(SimpleOrch::new());
    let kit = Kit::new(orch.clone() as Arc<dyn Dispatcher>)
        .with_signaler(orch.clone() as Arc<dyn Signalable>)
        .with_state(Arc::new(TestStore::new()));
    let runner = kit.local_runner().unwrap();
    let err = runner
        .run(
            OperatorId::new("missing"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .unwrap_err();
    match err {
        KitError::Dispatch(OrchError::OperatorNotFound(name)) => assert_eq!(name, "missing"),
        other => panic!("expected OperatorNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn runner_has_safety_bound_for_infinite_followups() {
    struct SelfDelegate;
    #[async_trait]
    impl Operator for SelfDelegate {
        async fn execute(
            &self,
            _input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            let mut output = OperatorOutput::new(Content::text("loop"), ExitReason::Complete);
            output.effects.push(Effect::new(EffectKind::Delegate {
                operator: OperatorId::new("root"),
                input: Box::new(OperatorInput::new(Content::text("loop"), TriggerType::Task)),
            }));
            Ok(output)
        }
    }

    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(SelfDelegate));

    let orch = Arc::new(orch);
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::new(TestStore::new()) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner =
        OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler).with_max_followups(8);

    let err = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("max_followups"));
}

#[tokio::test]
async fn runner_effect_pipeline_end_to_end() {
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(FullPipelineRootOperator));
    orch.register("child", Arc::new(ChildOperator));
    orch.register("handoff_target", Arc::new(HandoffTargetOperator));
    let orch = Arc::new(orch);
    let state = Arc::new(TestStore::new());
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    // root + delegate + handoff follow-up dispatches
    assert_eq!(trace.outputs.len(), 3);
    let messages: Vec<String> = trace
        .outputs
        .iter()
        .map(|o| o.message.as_text().unwrap_or_default().to_string())
        .collect();
    assert!(messages.iter().any(|m| m == "child done"));
    assert!(messages.iter().any(|m| m == "accepted"));

    // WriteMemory/DeleteMemory executed against state backend.
    assert_eq!(state.read_raw("k-pipeline").await, None);
    assert_eq!(
        state.ops().await,
        vec![
            "write:k-pipeline".to_string(),
            "delete:k-pipeline".to_string()
        ]
    );

    // Signal is sent by handler via Signalable::signal and is observable.
    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf-pipeline"));
    assert_eq!(signals[0].1.signal_type, "pipeline.signal");
}

// ── Middleware tests ─────────────────────────────────────────────────────────

use layer0::{EffectAction, EffectMiddleware, EffectStack};

/// Middleware that renames every WriteMemory key by prepending a prefix.
/// Used to verify that an enriched (modified) effect reaches the handler.
struct PrefixKeyMiddleware {
    prefix: String,
}

#[async_trait]
impl EffectMiddleware for PrefixKeyMiddleware {
    async fn on_effect(&self, mut effect: layer0::Effect, _ctx: &DispatchContext) -> EffectAction {
        if let layer0::EffectKind::WriteMemory { ref mut key, .. } = effect.kind {
            *key = format!("{}{}", self.prefix, key);
        }
        EffectAction::Continue(Box::new(effect))
    }
}

/// Middleware that skips all WriteMemory effects.
struct SkipWriteMemoryMiddleware;

#[async_trait]
impl EffectMiddleware for SkipWriteMemoryMiddleware {
    async fn on_effect(&self, effect: layer0::Effect, _ctx: &DispatchContext) -> EffectAction {
        if matches!(effect.kind, layer0::EffectKind::WriteMemory { .. }) {
            EffectAction::Skip
        } else {
            EffectAction::Continue(Box::new(effect))
        }
    }
}

/// Operator that writes a single key "mw-key" to state.
struct MwWriterOperator;

#[async_trait]
impl Operator for MwWriterOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, layer0::error::OperatorError> {
        let mut output = OperatorOutput::new(Content::text("wrote"), ExitReason::Complete);
        output.effects.push(Effect::new(EffectKind::WriteMemory {
            scope: Scope::Global,
            key: "mw-key".into(),
            value: json!({"v": 99}),
            memory_scope: MemoryScope::Session,
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        }));
        Ok(output)
    }
}

#[tokio::test]
async fn runner_processes_effects_through_middleware() {
    // Middleware rewrites "mw-key" → "enriched-mw-key".
    // The state store must contain the enriched key, not the original.
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(MwWriterOperator));
    let orch = Arc::new(orch);

    let state = Arc::new(TestStore::new());
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let stack = EffectStack::new().push(PrefixKeyMiddleware { prefix: "enriched-".into() });
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler)
        .with_effect_middleware(stack);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    // State must have been written under the enriched key.
    assert_eq!(state.read_raw("enriched-mw-key").await, Some(json!({"v": 99})));
    // The original key must not exist.
    assert_eq!(state.read_raw("mw-key").await, None);
    // Trace event reflects the enriched key name.
    assert!(trace.events.iter().any(|e| matches!(
        e,
        skg_orch_kit::ExecutionEvent::MemoryWritten { key } if key == "enriched-mw-key"
    )));
}

#[tokio::test]
async fn runner_skips_effects_via_middleware() {
    // Middleware suppresses WriteMemory; state must never be touched.
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(MwWriterOperator));
    let orch = Arc::new(orch);

    let state = Arc::new(TestStore::new());
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let stack = EffectStack::new().push(SkipWriteMemoryMiddleware);
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler)
        .with_effect_middleware(stack);

    let _trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    // The WriteMemory effect was skipped, so state must be empty.
    assert_eq!(state.read_raw("mw-key").await, None);
    assert_eq!(state.ops().await, Vec::<String>::new());
}

#[tokio::test]
async fn runner_without_middleware_unchanged() {
    // No middleware set: effects reach the handler unmodified.
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(MwWriterOperator));
    let orch = Arc::new(orch);

    let state = Arc::new(TestStore::new());
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::clone(&state) as Arc<dyn StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    // Deliberately no .with_effect_middleware().
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    // Without middleware, the original key must be written verbatim.
    assert_eq!(state.read_raw("mw-key").await, Some(json!({"v": 99})));
    assert_eq!(state.ops().await, vec!["write:mw-key".to_string()]);
    assert!(trace.events.iter().any(|e| matches!(
        e,
        skg_orch_kit::ExecutionEvent::MemoryWritten { key } if key == "mw-key"
    )));
}

// ── FIFO ordering test ──────────────────────────────────────────────────────

/// Root emits three Delegate effects in emission order (first, second, third).
/// With FIFO scheduling the outputs must arrive in that same order.
struct FifoRootOperator;

#[async_trait]
impl Operator for FifoRootOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("root"), ExitReason::Complete);
        for name in ["first", "second", "third"] {
            output.effects.push(Effect::new(EffectKind::Delegate {
                operator: OperatorId::new(name),
                input: Box::new(OperatorInput::new(Content::text(name), TriggerType::Task)),
            }));
        }
        Ok(output)
    }
}

struct EchoOperator {
    name: &'static str,
}

#[async_trait]
impl Operator for EchoOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError> {
        Ok(OperatorOutput::new(
            Content::text(self.name),
            ExitReason::Complete,
        ))
    }
}

#[tokio::test]
async fn runner_fifo_ordering() {
    // Root emits Delegate[first, second, third]; FIFO scheduling must run them
    // in emission order. outputs = [root, first, second, third].
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(FifoRootOperator));
    orch.register("first", Arc::new(EchoOperator { name: "first" }));
    orch.register("second", Arc::new(EchoOperator { name: "second" }));
    orch.register("third", Arc::new(EchoOperator { name: "third" }));

    let orch = Arc::new(orch);
    let handler = Arc::new(LocalEffectHandler::new(
        Arc::new(TestStore::new()) as Arc<dyn layer0::state::StateStore>,
        Some(orch.clone() as Arc<dyn Signalable>),
    ));
    let runner = OrchestratedRunner::new(orch.clone() as Arc<dyn Dispatcher>, handler);

    let trace = runner
        .run(
            OperatorId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    // Four outputs: root, first, second, third — in FIFO emission order.
    assert_eq!(trace.outputs.len(), 4);
    let messages: Vec<&str> = trace
        .outputs
        .iter()
        .map(|o| o.message.as_text().unwrap_or_default())
        .collect();
    assert_eq!(
        messages,
        ["root", "first", "second", "third"],
        "followups must execute in FIFO (emission) order"
    );
}
