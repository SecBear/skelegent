use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::id::OperatorId;
use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};
use layer0::test_utils::EchoOperator;
use layer0::{DispatchContext, DispatchId};
use skg_orch_local::LocalOrch;
use std::sync::Arc;

fn simple_input(msg: &str) -> OperatorInput {
    OperatorInput::new(Content::text(msg), TriggerType::User)
}

fn test_ctx(name: &str) -> DispatchContext {
    DispatchContext::new(DispatchId::new(name), OperatorId::new(name))
}

// --- Single dispatch ---

#[tokio::test]
async fn dispatch_to_registered_agent() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let output = orch
        .dispatch(&test_ctx("echo"), simple_input("hello"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("hello"));
}

#[tokio::test]
async fn dispatch_agent_not_found() {
    let orch = LocalOrch::new();

    let result = orch
        .dispatch(&test_ctx("missing"), simple_input("fail"))
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
        _ctx: &layer0::DispatchContext,
        _emitter: &layer0::dispatch::EffectEmitter,
    ) -> Result<OperatorOutput, layer0::error::OperatorError> {
        Err(layer0::error::OperatorError::non_retryable(
            "always fails",
        ))
    }
}

#[tokio::test]
async fn dispatch_propagates_operator_error() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("fail"), Arc::new(FailingOperator));

    let result = orch
        .dispatch(&test_ctx("fail"), simple_input("boom"))
        .await
        .unwrap()
        .collect()
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("always fails"));
}

// --- Object safety ---

#[tokio::test]
async fn usable_as_dyn_dispatcher() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let orch: Box<dyn Dispatcher> = Box::new(orch);
    let output = orch
        .dispatch(&test_ctx("echo"), simple_input("dyn"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("dyn"));
}

#[tokio::test]
async fn usable_as_arc_dyn_dispatcher() {
    let mut orch = LocalOrch::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let orch: Arc<dyn Dispatcher> = Arc::new(orch);
    let output = orch
        .dispatch(&test_ctx("echo"), simple_input("arc"))
        .await
        .unwrap()
        .collect()
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
    use layer0::DispatchContext;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountMiddleware {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DispatchMiddleware for CountMiddleware {
        async fn dispatch(
            &self,
            ctx: &DispatchContext,
            input: OperatorInput,
            next: &dyn DispatchNext,
        ) -> Result<layer0::dispatch::DispatchHandle, OrchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            next.dispatch(ctx, input).await
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let mw = Arc::new(CountMiddleware {
        calls: Arc::clone(&calls),
    });

    let stack = DispatchStack::builder().observe(mw).build();

    let mut orch = LocalOrch::new().with_middleware(stack);
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let _output = orch
        .dispatch(&test_ctx("echo"), simple_input("ping"))
        .await
        .unwrap()
        .collect()
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
    use layer0::DispatchContext;

    struct DenyAll;

    #[async_trait]
    impl DispatchMiddleware for DenyAll {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            _input: OperatorInput,
            _next: &dyn DispatchNext,
        ) -> Result<layer0::dispatch::DispatchHandle, OrchError> {
            Err(OrchError::DispatchFailed("denied by guard".into()))
        }
    }

    let stack = DispatchStack::builder().guard(Arc::new(DenyAll)).build();

    let mut orch = LocalOrch::new().with_middleware(stack);
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));

    let result = orch
        .dispatch(&test_ctx("echo"), simple_input("blocked"))
        .await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("denied by guard"),
        "guard middleware must halt dispatch"
    );
}
