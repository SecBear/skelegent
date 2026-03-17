use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::id::{DispatchId, OperatorId, WorkflowId};
use layer0::operator::{OperatorInput, TriggerType};
use layer0::state::StateStore;
use layer0::test_utils::InMemoryStore;
use serde_json::json;
use skg_effects_core::{EffectHandler, EffectOutcome, Signalable};
use skg_effects_local::LocalEffectHandler;
use std::sync::Arc;
use tokio::sync::Mutex;

struct MockOrch {
    signals: Mutex<Vec<(WorkflowId, SignalPayload)>>,
}

impl MockOrch {
    fn new() -> Self {
        Self {
            signals: Mutex::new(vec![]),
        }
    }

    async fn recorded_signals(&self) -> Vec<(WorkflowId, SignalPayload)> {
        self.signals.lock().await.clone()
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
    let handler = LocalEffectHandler::new(state.clone(), None);
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));

    // Write then read outside handler.
    let effect = Effect::WriteMemory {
        scope: Scope::Global,
        key: "k1".into(),
        value: json!({"v": 1}),
        tier: None,
        lifetime: None,
        content_kind: None,
        salience: None,
        ttl: None,
    };
    let outcome = handler.handle(&effect, &ctx).await.expect("write ok");
    assert!(matches!(outcome, EffectOutcome::Applied));
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k1").await.expect("read ok");
    assert_eq!(got, Some(json!({"v": 1})));

    // Delete a missing key is Ok (idempotent)
    let effect = Effect::DeleteMemory {
        scope: Scope::Global,
        key: "missing".into(),
    };
    let outcome = handler
        .handle(&effect, &ctx)
        .await
        .expect("delete missing ok");
    assert!(matches!(outcome, EffectOutcome::Applied));

    // Delete existing key then verify None
    let effect = Effect::DeleteMemory {
        scope: Scope::Global,
        key: "k1".into(),
    };
    let outcome = handler.handle(&effect, &ctx).await.expect("delete ok");
    assert!(matches!(outcome, EffectOutcome::Applied));
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k1").await.expect("read ok");
    assert_eq!(got, None);
}

#[tokio::test]
async fn delegate_handoff_and_signal_return_correct_outcomes() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(MockOrch::new());
    let handler = LocalEffectHandler::new(state, Some(orch.clone() as Arc<dyn Signalable>));

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));

    // Delegate returns EffectOutcome::Delegate
    let effect = Effect::Delegate {
        operator: OperatorId::new("child"),
        input: Box::new(OperatorInput::new(
            Content::text("child task"),
            TriggerType::Task,
        )),
    };
    let outcome = handler.handle(&effect, &ctx).await.expect("delegate ok");
    assert!(
        matches!(&outcome, EffectOutcome::Delegate { operator, .. } if operator == &OperatorId::new("child")),
        "expected Delegate outcome, got {outcome:?}"
    );

    // Handoff returns EffectOutcome::Handoff
    let effect = Effect::Handoff {
        operator: OperatorId::new("handoff_target"),
        state: json!({"ticket": 123}),
    };
    let outcome = handler.handle(&effect, &ctx).await.expect("handoff ok");
    assert!(
        matches!(&outcome, EffectOutcome::Handoff { operator, .. } if operator == &OperatorId::new("handoff_target")),
        "expected Handoff outcome, got {outcome:?}"
    );

    // Signal is sent via Signalable
    let effect = Effect::Signal {
        target: WorkflowId::new("wf1"),
        payload: SignalPayload::new("sig.type", json!({"ok": true})),
    };
    let outcome = handler.handle(&effect, &ctx).await.expect("signal ok");
    assert!(matches!(outcome, EffectOutcome::Applied));

    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf1"));
    assert_eq!(signals[0].1.signal_type, "sig.type");
}

#[tokio::test]
async fn preserves_effect_order_across_memory_and_orch_calls() {
    let state = Arc::new(InMemoryStore::new());
    let orch = Arc::new(MockOrch::new());
    let handler = LocalEffectHandler::new(state.clone(), Some(orch.clone() as Arc<dyn Signalable>));

    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));
    // Delete then Write ensures final value exists only if order preserved.
    let effects: Vec<Effect> = vec![
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

    let mut outcomes = vec![];
    for effect in &effects {
        let outcome = handler.handle(effect, &ctx).await.expect("effect ok");
        outcomes.push(outcome);
    }

    // Memory reflects order: write at end means value present
    let got: Option<serde_json::Value> = state.read(&Scope::Global, "k_order").await.unwrap();
    assert_eq!(got, Some(json!(42)));

    // Delegate returned as outcome
    assert!(
        matches!(&outcomes[1], EffectOutcome::Delegate { operator, .. } if operator == &OperatorId::new("a"))
    );

    // Signal was sent
    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf_order"));
}

#[tokio::test]
async fn signal_without_signaler_returns_error() {
    let state = Arc::new(InMemoryStore::new());
    let handler = LocalEffectHandler::new(state, None);
    let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::new("test"));

    let effect = Effect::Signal {
        target: WorkflowId::new("wf1"),
        payload: SignalPayload::new("t", json!({})),
    };
    let result = handler.handle(&effect, &ctx).await;
    assert!(result.is_err(), "signal without signaler should error");
}
