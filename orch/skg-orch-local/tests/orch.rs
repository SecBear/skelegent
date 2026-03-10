use layer0::content::Content;
use layer0::id::{OperatorId, WorkflowId};
use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::test_utils::EchoOperator;
use skg_orch_local::LocalOrch;
use std::sync::Arc;

fn simple_input(msg: &str) -> OperatorInput {
    OperatorInput::new(Content::text(msg), TriggerType::User)
}

// --- Single dispatch ---

#[tokio::test]
async fn dispatch_to_registered_agent() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let output = orch
        .dispatch(&OperatorId::new("echo"), simple_input("hello"))
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("hello"));
}

#[tokio::test]
async fn dispatch_agent_not_found() {
    let orch = LocalOrch::new();

    let result = orch
        .dispatch(&OperatorId::new("missing"), simple_input("fail"))
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("operator not found")
    );
}

// --- Error propagation ---

struct FailingOperator;

#[async_trait::async_trait]
impl layer0::operator::Operator for FailingOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
    ) -> Result<OperatorOutput, layer0::error::OperatorError> {
        Err(layer0::error::OperatorError::NonRetryable(
            "always fails".into(),
        ))
    }
}

#[tokio::test]
async fn dispatch_propagates_operator_error() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("fail"), Arc::new(FailingOperator));

    let result = orch
        .dispatch(&OperatorId::new("fail"), simple_input("boom"))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("always fails"));
}

// --- Dispatch many ---

#[tokio::test]
async fn dispatch_many_concurrent() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("a"), Arc::new(EchoOperator));
    orch.register(OperatorId::new("b"), Arc::new(EchoOperator));

    let tasks = vec![
        (OperatorId::new("a"), simple_input("msg-a")),
        (OperatorId::new("b"), simple_input("msg-b")),
    ];

    let results = orch.dispatch_many(tasks).await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().message, Content::text("msg-a"));
    assert_eq!(results[1].as_ref().unwrap().message, Content::text("msg-b"));
}

#[tokio::test]
async fn dispatch_many_partial_failure() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("ok"), Arc::new(EchoOperator));
    // "bad" is not registered

    let tasks = vec![
        (OperatorId::new("ok"), simple_input("fine")),
        (OperatorId::new("bad"), simple_input("fail")),
    ];

    let results = orch.dispatch_many(tasks).await;
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
}

// --- Signal and query ---

#[tokio::test]
async fn signal_increments_journal_and_query_reports_count() {
    let orch = LocalOrch::new();
    let wf = WorkflowId::new("wf-1");
    // Initially zero, query should lazily create and report 0
    let initial = orch
        .query(&wf, QueryPayload::new("any", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(initial["signals"], serde_json::json!(0));

    // Send two signals and verify count via query
    orch.signal(
        &wf,
        layer0::effect::SignalPayload::new("a", serde_json::json!(null)),
    )
    .await
    .unwrap();
    orch.signal(
        &wf,
        layer0::effect::SignalPayload::new("b", serde_json::json!({"x":1})),
    )
    .await
    .unwrap();

    let result = orch
        .query(&wf, QueryPayload::new("ignored", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(result, serde_json::json!({"signals": 2}));
}

#[tokio::test]
async fn signal_count_getter_matches_query() {
    let orch = LocalOrch::new();
    let wf = WorkflowId::new("wf-2");
    orch.signal(
        &wf,
        layer0::effect::SignalPayload::new("s", serde_json::json!({})),
    )
    .await
    .unwrap();
    let count = orch.signal_count(&wf).await;
    let val = orch
        .query(&wf, QueryPayload::new("anything", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(serde_json::json!({"signals": count}), val);
}

#[tokio::test]
async fn parallel_signals_recorded_correctly() {
    let orch = Arc::new(LocalOrch::new());
    let wf = WorkflowId::new("wf-par");
    let n = 64usize;
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let orch = Arc::clone(&orch);
        let wf = wf.clone();
        handles.push(tokio::spawn(async move {
            let payload = layer0::effect::SignalPayload::new("p", serde_json::json!({"i": i}));
            orch.signal(&wf, payload).await.unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let count = orch.signal_count(&wf).await;
    assert_eq!(count, n);
}

// --- Object safety ---

#[tokio::test]
async fn usable_as_dyn_orchestrator() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let orch: Box<dyn Orchestrator> = Box::new(orch);
    let output = orch
        .dispatch(&OperatorId::new("echo"), simple_input("dyn"))
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("dyn"));
}

#[tokio::test]
async fn usable_as_arc_dyn_orchestrator() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let orch: Arc<dyn Orchestrator> = Arc::new(orch);
    let output = orch
        .dispatch(&OperatorId::new("echo"), simple_input("arc"))
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("arc"));
}

// --- Middleware ---

#[tokio::test]
async fn middleware_observer_fires_on_dispatch() {
    use async_trait::async_trait;
    use layer0::error::OrchError;
    use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountMiddleware {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DispatchMiddleware for CountMiddleware {
        async fn dispatch(
            &self,
            operator: &OperatorId,
            input: OperatorInput,
            next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            next.dispatch(operator, input).await
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let mw = Arc::new(CountMiddleware {
        calls: Arc::clone(&calls),
    });

    let stack = DispatchStack::builder().observe(mw).build();

    let mut orch = LocalOrch::new().with_middleware(stack);
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    orch.dispatch(&OperatorId::new("echo"), simple_input("ping"))
        .await
        .unwrap();

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "middleware observer must fire exactly once"
    );
}

#[tokio::test]
async fn middleware_guard_can_halt_dispatch() {
    use async_trait::async_trait;
    use layer0::error::OrchError;
    use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};

    struct DenyAll;

    #[async_trait]
    impl DispatchMiddleware for DenyAll {
        async fn dispatch(
            &self,
            _operator: &OperatorId,
            _input: OperatorInput,
            _next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            Err(OrchError::DispatchFailed("denied by guard".into()))
        }
    }

    let stack = DispatchStack::builder().guard(Arc::new(DenyAll)).build();

    let mut orch = LocalOrch::new().with_middleware(stack);
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let result = orch
        .dispatch(&OperatorId::new("echo"), simple_input("blocked"))
        .await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("denied by guard"),
        "guard middleware must halt dispatch"
    );
}
