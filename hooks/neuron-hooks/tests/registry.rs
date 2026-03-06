use layer0::error::HookError;
use layer0::hook::{Hook, HookAction, HookContext, HookPoint};
use layer0::test_utils::LoggingHook;
use neuron_hooks::HookRegistry;
use std::sync::Arc;

// --- Empty registry ---

#[tokio::test]
async fn empty_registry_returns_continue() {
    let registry = HookRegistry::new();
    let ctx = HookContext::new(HookPoint::PreInference);
    let action = registry.dispatch(&ctx).await;
    assert!(matches!(action, HookAction::Continue));
}

// --- Single hook ---

#[tokio::test]
async fn single_hook_dispatches() {
    let mut registry = HookRegistry::new();
    let hook = Arc::new(LoggingHook::new());
    registry.add_observer(hook.clone());

    let ctx = HookContext::new(HookPoint::PreInference);
    let action = registry.dispatch(&ctx).await;
    assert!(matches!(action, HookAction::Continue));

    // LoggingHook records events
    let events = hook.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].point, HookPoint::PreInference);
}

// --- Hook ordering ---

/// A hook that records its name for ordering verification.
struct NamedHook {
    name: String,
    log: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl Hook for NamedHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreInference, HookPoint::PostInference]
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        self.log.lock().unwrap().push(self.name.clone());
        Ok(HookAction::Continue)
    }
}

#[tokio::test]
async fn hooks_execute_in_registration_order() {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));

    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(NamedHook {
        name: "first".into(),
        log: Arc::clone(&log),
    }));
    registry.add_guardrail(Arc::new(NamedHook {
        name: "second".into(),
        log: Arc::clone(&log),
    }));
    registry.add_guardrail(Arc::new(NamedHook {
        name: "third".into(),
        log: Arc::clone(&log),
    }));

    let ctx = HookContext::new(HookPoint::PreInference);
    registry.dispatch(&ctx).await;

    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["first", "second", "third"]);
}

// --- Halt propagation ---

/// A hook that halts.
struct HaltingHook;

#[async_trait::async_trait]
impl Hook for HaltingHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreInference]
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Ok(HookAction::Halt {
            reason: "policy violation".into(),
        })
    }
}

#[tokio::test]
async fn halt_stops_pipeline() {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));

    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(NamedHook {
        name: "before-halt".into(),
        log: Arc::clone(&log),
    }));
    registry.add_guardrail(Arc::new(HaltingHook));
    registry.add_guardrail(Arc::new(NamedHook {
        name: "after-halt".into(),
        log: Arc::clone(&log),
    }));

    let ctx = HookContext::new(HookPoint::PreInference);
    let action = registry.dispatch(&ctx).await;

    // Should halt
    assert!(matches!(action, HookAction::Halt { .. }));

    // "after-halt" should NOT have been called
    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["before-halt"]);
}

// --- Point filtering ---

#[tokio::test]
async fn hooks_only_fire_at_registered_points() {
    let mut registry = HookRegistry::new();
    let hook = Arc::new(LoggingHook::new());
    registry.add_observer(hook.clone());

    // LoggingHook registers for all 5 points. Let's check it fires.
    let ctx = HookContext::new(HookPoint::ExitCheck);
    registry.dispatch(&ctx).await;
    assert_eq!(hook.events().len(), 1);

    // A hook that only registers for PreSubDispatch should not fire on PreInference
    struct PreDispatchOnly;
    #[async_trait::async_trait]
    impl Hook for PreDispatchOnly {
        fn points(&self) -> &[HookPoint] {
            &[HookPoint::PreSubDispatch]
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            panic!("should not fire at PreInference!");
        }
    }

    let mut registry2 = HookRegistry::new();
    registry2.add_guardrail(Arc::new(PreDispatchOnly));

    let ctx = HookContext::new(HookPoint::PreInference);
    let action = registry2.dispatch(&ctx).await;
    assert!(matches!(action, HookAction::Continue));
}

// --- Error handling ---

/// A hook that errors (but errors don't halt).
struct ErroringHook;

#[async_trait::async_trait]
impl Hook for ErroringHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreInference]
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Err(HookError::Failed("something broke".into()))
    }
}

#[tokio::test]
async fn hook_error_does_not_halt_pipeline() {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));

    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(ErroringHook));
    registry.add_guardrail(Arc::new(NamedHook {
        name: "after-error".into(),
        log: Arc::clone(&log),
    }));

    let ctx = HookContext::new(HookPoint::PreInference);
    let action = registry.dispatch(&ctx).await;

    // Errors are logged, pipeline continues
    assert!(matches!(action, HookAction::Continue));
    let log = log.lock().unwrap();
    assert_eq!(*log, vec!["after-error"]);
}

// --- SkipDispatch and ModifyDispatchInput propagation ---

struct SkipDispatchHook;

#[async_trait::async_trait]
impl Hook for SkipDispatchHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSubDispatch]
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Ok(HookAction::SkipDispatch {
            reason: "not allowed".into(),
        })
    }
}

#[tokio::test]
async fn skip_dispatch_stops_pipeline() {
    let mut registry = HookRegistry::new();
    registry.add_guardrail(Arc::new(SkipDispatchHook));

    let ctx = HookContext::new(HookPoint::PreSubDispatch);
    let action = registry.dispatch(&ctx).await;
    assert!(matches!(action, HookAction::SkipDispatch { .. }));
}

struct ModifyInputHook;

#[async_trait::async_trait]
impl Hook for ModifyInputHook {
    fn points(&self) -> &[HookPoint] {
        &[HookPoint::PreSubDispatch]
    }

    async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
        Ok(HookAction::ModifyDispatchInput {
            new_input: serde_json::json!({"sanitized": true}),
        })
    }
}

#[tokio::test]
async fn modify_dispatch_input_stops_pipeline() {
    let mut registry = HookRegistry::new();
    registry.add_transformer(Arc::new(ModifyInputHook));

    let ctx = HookContext::new(HookPoint::PreSubDispatch);
    let action = registry.dispatch(&ctx).await;
    assert!(matches!(action, HookAction::ModifyDispatchInput { .. }));
}
