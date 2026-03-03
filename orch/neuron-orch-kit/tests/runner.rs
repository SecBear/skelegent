use async_trait::async_trait;
use layer0::content::Content;
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::error::{OperatorError, OrchError, StateError};
use layer0::id::{AgentId, WorkflowId};
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::state::{SearchResult, StateStore};
use neuron_orch_kit::{Kit, KitError, LocalEffectInterpreter, OrchestratedRunner};
use serde_json::json;
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
impl Orchestrator for SimpleOrch {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .agents
            .get(agent.as_str())
            .ok_or_else(|| OrchError::AgentNotFound(agent.to_string()))?;
        op.execute(input).await.map_err(OrchError::OperatorError)
    }

    async fn dispatch_many(
        &self,
        tasks: Vec<(AgentId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut results = Vec::with_capacity(tasks.len());
        for (agent, input) in tasks {
            results.push(self.dispatch(&agent, input).await);
        }
        results
    }

    async fn signal(&self, target: &WorkflowId, signal: SignalPayload) -> Result<(), OrchError> {
        self.signals.lock().await.push((target.clone(), signal));
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
    async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("wrote"), ExitReason::Complete);
        output.effects.push(Effect::WriteMemory {
            scope: Scope::Global,
            key: "k1".into(),
            value: json!({"v": 1}),
        });
        output.effects.push(Effect::Signal {
            target: WorkflowId::new("wf1"),
            payload: SignalPayload::new("sig.type", json!({"ok": true})),
        });
        Ok(output)
    }
}

struct DelegateOperator;

#[async_trait]
impl Operator for DelegateOperator {
    async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("delegating"), ExitReason::Complete);
        output.effects.push(Effect::Delegate {
            agent: AgentId::new("child"),
            input: Box::new(OperatorInput::new(
                Content::text("child task"),
                TriggerType::Task,
            )),
        });
        Ok(output)
    }
}

struct ChildOperator;

#[async_trait]
impl Operator for ChildOperator {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
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
    async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("handoff"), ExitReason::Complete);
        output.effects.push(Effect::Handoff {
            agent: AgentId::new("handoff_target"),
            state: json!({"ticket": 123}),
        });
        Ok(output)
    }
}

struct HandoffTargetOperator;

#[async_trait]
impl Operator for HandoffTargetOperator {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        // LocalEffectInterpreter serializes the JSON state to a string and puts it in message text.
        let text = input.message.as_text().unwrap_or_default();
        assert!(text.contains("\"ticket\":123") || text.contains("\"ticket\": 123"));
        Ok(OperatorOutput::new(
            Content::text("accepted"),
            ExitReason::Complete,
        ))
    }
}

struct FullPipelineRootOperator;

#[async_trait]
impl Operator for FullPipelineRootOperator {
    async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let mut output = OperatorOutput::new(Content::text("root"), ExitReason::Complete);
        output.effects.push(Effect::WriteMemory {
            scope: Scope::Global,
            key: "k-pipeline".into(),
            value: json!({"v": 42}),
        });
        output.effects.push(Effect::Delegate {
            agent: AgentId::new("child"),
            input: Box::new(OperatorInput::new(
                Content::text("child task"),
                TriggerType::Task,
            )),
        });
        output.effects.push(Effect::Handoff {
            agent: AgentId::new("handoff_target"),
            state: json!({"ticket": 123}),
        });
        output.effects.push(Effect::Signal {
            target: WorkflowId::new("wf-pipeline"),
            payload: SignalPayload::new("pipeline.signal", json!({"ok": true})),
        });
        output.effects.push(Effect::DeleteMemory {
            scope: Scope::Global,
            key: "k-pipeline".into(),
        });
        Ok(output)
    }
}

#[tokio::test]
async fn runner_executes_memory_and_signal_effects() {
    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(WriterOperator));
    let orch = Arc::new(orch);
    let orch_for_runner: Arc<dyn Orchestrator> = orch.clone();

    let state = Arc::new(TestStore::new());
    let runner = OrchestratedRunner::new(
        orch_for_runner,
        Arc::new(LocalEffectInterpreter::new(Arc::clone(&state))),
    );

    let trace = runner
        .run(
            AgentId::new("root"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .expect("runner should succeed");

    assert_eq!(trace.outputs.len(), 1);
    assert_eq!(state.read_raw("k1").await, Some(json!({"v": 1})));

    // Signal is sent by the runner via Orchestrator::signal and recorded by our orch.
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
    let runner = OrchestratedRunner::new(
        Arc::new(orch),
        Arc::new(LocalEffectInterpreter::new(Arc::clone(&state))),
    );

    let trace = runner
        .run(
            AgentId::new("root"),
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

    let runner = OrchestratedRunner::new(
        Arc::new(orch),
        Arc::new(LocalEffectInterpreter::new(Arc::new(TestStore::new()))),
    );

    let trace = runner
        .run(
            AgentId::new("root"),
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
    let kit = Kit::new(Arc::new(SimpleOrch::new()));
    let err = match kit.local_runner() {
        Ok(_) => panic!("expected error, got Ok"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("requires a state backend"));
}

#[tokio::test]
async fn unknown_agent_returns_agent_not_found() {
    let kit = Kit::new(Arc::new(SimpleOrch::new())).with_state(Arc::new(TestStore::new()));
    let runner = kit.local_runner().unwrap();
    let err = runner
        .run(
            AgentId::new("missing"),
            OperatorInput::new(Content::text("go"), TriggerType::User),
        )
        .await
        .unwrap_err();
    match err {
        KitError::Orchestrator(OrchError::AgentNotFound(name)) => assert_eq!(name, "missing"),
        other => panic!("expected AgentNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn runner_has_safety_bound_for_infinite_followups() {
    struct SelfDelegate;
    #[async_trait]
    impl Operator for SelfDelegate {
        async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
            let mut output = OperatorOutput::new(Content::text("loop"), ExitReason::Complete);
            output.effects.push(Effect::Delegate {
                agent: AgentId::new("root"),
                input: Box::new(OperatorInput::new(Content::text("loop"), TriggerType::Task)),
            });
            Ok(output)
        }
    }

    let mut orch = SimpleOrch::new();
    orch.register("root", Arc::new(SelfDelegate));

    let runner = OrchestratedRunner::new(
        Arc::new(orch),
        Arc::new(LocalEffectInterpreter::new(Arc::new(TestStore::new()))),
    )
    .with_max_followups(8);

    let err = runner
        .run(
            AgentId::new("root"),
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
    let orch_for_runner: Arc<dyn Orchestrator> = orch.clone();

    let state = Arc::new(TestStore::new());
    let runner = OrchestratedRunner::new(
        orch_for_runner,
        Arc::new(LocalEffectInterpreter::new(Arc::clone(&state))),
    );

    let trace = runner
        .run(
            AgentId::new("root"),
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

    // Signal is sent by runner via Orchestrator::signal and is observable.
    let signals = orch.recorded_signals().await;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].0, WorkflowId::new("wf-pipeline"));
    assert_eq!(signals[0].1.signal_type, "pipeline.signal");
}
