use layer0::content::Content;
use layer0::dispatch::{DispatchEvent, DispatchHandle, Dispatcher};
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::DispatchContext;
use layer0::id::{DispatchId, OperatorId, WorkflowId};
use layer0::operator::{ExitReason, OperatorInput, OperatorOutput, TriggerType};
use layer0::state::StateStore;
use layer0::test_utils::InMemoryStore;
use serde_json::json;
use skg_effects_core::Signalable;
use skg_orch_kit::effects::{EffectExecutor as EffectsTrait, LocalEffectExecutor};
use std::sync::Arc;
use tokio::sync::Mutex;

struct MockOrch {
    dispatches: Mutex<Vec<(OperatorId, OperatorInput)>>,
    signals: Mutex<Vec<(WorkflowId, SignalPayload)>>,
}

impl MockOrch {
    fn new() -> Self {
        Self {
            dispatches: Mutex::new(vec![]),
            signals: Mutex::new(vec![]),
        }
    }

    async fn recorded_dispatches(&self) -> Vec<(OperatorId, OperatorInput)> {
        self.dispatches.lock().await.clone()
    }

    async fn recorded_signals(&self) -> Vec<(WorkflowId, SignalPayload)> {
        self.signals.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl Dispatcher for MockOrch {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<DispatchHandle, layer0::error::OrchError> {
        self.dispatches
            .lock()
            .await
            .push((operator.clone(), input.clone()));
        let output = OperatorOutput::new(Content::text("ok"), ExitReason::Complete);
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("mock-effects"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        Ok(handle)
    }
}

#[async_trait::async_trait]
impl Signalable for MockOrch {
    async fn signal(
        &self,
        target: &WorkflowId,
        signal: SignalPayload,
    ) -> Result<(), layer0::error::OrchError> {
        self.signals.lock().await.push((target.clone(), signal));
        Ok(())
    }
}

#[tokio::test]
async fn executes_write_read_delete_sequence_and_delete_missing_ok() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(MockOrch::new());
    let exec = LocalEffectExecutor::new(
        state.clone(),
        orch.clone() as Arc<dyn Dispatcher>,
        Some(orch as Arc<dyn Signalable>),
    );

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));

    // Write then read outside executor.
    exec.execute(&[Effect::WriteMemory {
        scope: Scope::Global,
        key: "k1".into(),
        value: json!({"v": 1}),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    }], &ctx)
    .await
    .expect("write ok");
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k1").await.expect("read ok");
    assert_eq!(got, Some(json!({"v": 1})));

    // Delete a missing key is Ok (idempotent)
    exec.execute(&[Effect::DeleteMemory {
        scope: Scope::Global,
        key: "missing".into(),
    }], &ctx)
    .await
    .expect("delete missing ok");

    // Delete existing key then verify None
    exec.execute(&[Effect::DeleteMemory {
        scope: Scope::Global,
        key: "k1".into(),
    }], &ctx)
    .await
    .expect("delete ok");
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k1").await.expect("read ok");
    assert_eq!(got, None);
}

#[tokio::test]
async fn delegate_handoff_and_signal_call_orchestrator_in_order() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(MockOrch::new());
    let exec = LocalEffectExecutor::new(
        state,
        orch.clone() as Arc<dyn Dispatcher>,
        Some(orch.clone() as Arc<dyn Signalable>),
    );

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
    let effects = vec![
        Effect::Delegate {
            operator: OperatorId::new("child"),
            input: Box::new(OperatorInput::new(
                Content::text("child task"),
                TriggerType::Task,
            )),
        },
        Effect::Handoff {
            operator: OperatorId::new("handoff_target"),
            state: json!({"ticket": 123}),
        },
        Effect::Signal {
            target: WorkflowId::new("wf1"),
            payload: SignalPayload::new("sig.type", json!({"ok": true})),
        },
    ];

    exec.execute(&effects, &ctx).await.expect("effects ok");

    // Verify dispatch order preserved: delegate then handoff
    let dispatches = orch.recorded_dispatches().await;
    assert_eq!(dispatches.len(), 2);
    assert_eq!(dispatches[0].0, OperatorId::new("child"));
    assert_eq!(dispatches[0].1.message.as_text().unwrap(), "child task");

    assert_eq!(dispatches[1].0, OperatorId::new("handoff_target"));
    // Handoff metadata flag present
    let meta = &dispatches[1].1.metadata;
    assert_eq!(meta.get("handoff").and_then(|v| v.as_bool()), Some(true));
    // Handoff message carries serialized JSON
    assert!(
        dispatches[1]
            .1
            .message
            .as_text()
            .unwrap()
            .contains("\"ticket\":")
    );

    // Signal recorded
    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf1"));
    assert_eq!(signals[0].1.signal_type, "sig.type");
}

#[tokio::test]
async fn preserves_effect_order_across_memory_and_orch_calls() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(MockOrch::new());
    let exec = LocalEffectExecutor::new(
        state.clone(),
        orch.clone() as Arc<dyn Dispatcher>,
        Some(orch.clone() as Arc<dyn Signalable>),
    );

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
    // Delete then Write ensures final value exists only if order preserved.
    let effects = vec![
        Effect::DeleteMemory {
            scope: Scope::Global,
            key: "k_order".into(),
        },
        Effect::Delegate {
            operator: OperatorId::new("a"),
            input: Box::new(OperatorInput::new(Content::text("x"), TriggerType::Task)),
        },
        Effect::WriteMemory {
            scope: Scope::Global,
            key: "k_order".into(),
            value: json!(42),
            tier: None,
            lifetime: None,
            content_kind: None,
            salience: None,
            ttl: None,
        },
        Effect::Signal {
            target: WorkflowId::new("wf_order"),
            payload: SignalPayload::new("t", json!({})),
        },
    ];

    exec.execute(&effects, &ctx).await.expect("effects ok");

    // Memory reflects order: write at end means value present
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k_order").await.unwrap();
    assert_eq!(got, Some(json!(42)));

    // Dispatch/signal call order preserved relative to other orch calls
    let dispatches = orch.recorded_dispatches().await;
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].0, OperatorId::new("a"));

    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf_order"));
}
