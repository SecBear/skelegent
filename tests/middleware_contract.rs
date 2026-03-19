//! Contract tests proving all 6 middleware stacks follow the Middleware Blueprint.
//!
//! For each stack, we verify three invariants:
//! 1. Empty stack passes through — terminal called directly.
//! 2. Observer sees all calls — count increments even with no other middleware.
//! 3. Guard can short-circuit — error returned, terminal NOT called.
//!
//! These tests catch structural drift if any stack diverges from the pattern.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 1. DispatchStack (layer0)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod dispatch_contract {
    use super::*;
    use async_trait::async_trait;
    use layer0::ExitReason;
    use layer0::content::Content;
    use layer0::dispatch::{DispatchEvent, DispatchHandle};
    use layer0::dispatch_context::DispatchContext;
    use layer0::error::OrchError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
    use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};

    fn make_input() -> OperatorInput {
        OperatorInput::new(Content::text("test"), TriggerType::User)
    }

    fn make_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("contract"), OperatorId::from("op"))
    }

    fn immediate_handle(output: OperatorOutput) -> DispatchHandle {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("contract"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        handle
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl DispatchNext for CountingTerminal {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            input: OperatorInput,
        ) -> Result<DispatchHandle, OrchError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(immediate_handle(OperatorOutput::new(
                input.message,
                ExitReason::Complete,
            )))
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = DispatchStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .dispatch_with(&make_ctx(), make_input(), &terminal)
            .await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchMiddleware for Observer {
            async fn dispatch(
                &self,
                ctx: &DispatchContext,
                input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.dispatch(ctx, input).await
            }
        }

        let stack = DispatchStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .dispatch_with(&make_ctx(), make_input(), &terminal)
            .await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl DispatchMiddleware for DenyGuard {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
                _next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, OrchError> {
                Err(OrchError::DispatchFailed("blocked by guard".into()))
            }
        }

        let stack = DispatchStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .dispatch_with(&make_ctx(), make_input(), &terminal)
            .await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 2. StoreStack (layer0) — tests write path
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod store_contract {
    use super::*;
    use async_trait::async_trait;
    use layer0::effect::Scope;
    use layer0::error::StateError;
    use layer0::id::{OperatorId, WorkflowId};
    use layer0::middleware::{StoreMiddleware, StoreStack, StoreWriteNext};
    use layer0::state::StoreOptions;

    fn make_scope() -> Scope {
        Scope::Operator {
            workflow: WorkflowId::from("w"),
            operator: OperatorId::from("op"),
        }
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl StoreWriteNext for CountingTerminal {
        async fn write(
            &self,
            _scope: &Scope,
            _key: &str,
            _value: serde_json::Value,
            _options: Option<&StoreOptions>,
        ) -> Result<(), StateError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = StoreStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .write_with(&make_scope(), "k", serde_json::json!(1), None, &terminal)
            .await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl StoreMiddleware for Observer {
            async fn write(
                &self,
                scope: &Scope,
                key: &str,
                value: serde_json::Value,
                options: Option<&StoreOptions>,
                next: &dyn StoreWriteNext,
            ) -> Result<(), StateError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.write(scope, key, value, options).await
            }
        }

        let stack = StoreStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .write_with(&make_scope(), "k", serde_json::json!(1), None, &terminal)
            .await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl StoreMiddleware for DenyGuard {
            async fn write(
                &self,
                _scope: &Scope,
                _key: &str,
                _value: serde_json::Value,
                _options: Option<&StoreOptions>,
                _next: &dyn StoreWriteNext,
            ) -> Result<(), StateError> {
                Err(StateError::WriteFailed("blocked by guard".into()))
            }
        }

        let stack = StoreStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .write_with(&make_scope(), "k", serde_json::json!(1), None, &terminal)
            .await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 3. ExecStack (layer0)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod exec_contract {
    use super::*;
    use async_trait::async_trait;
    use layer0::ExitReason;
    use layer0::content::Content;
    use layer0::dispatch_context::DispatchContext;
    use layer0::environment::EnvironmentSpec;
    use layer0::error::EnvError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::middleware::{ExecMiddleware, ExecNext, ExecStack};
    use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};

    fn make_input() -> OperatorInput {
        OperatorInput::new(Content::text("test"), TriggerType::User)
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl ExecNext for CountingTerminal {
        async fn run(
            &self,
            _ctx: &DispatchContext,
            input: OperatorInput,
            _spec: &EnvironmentSpec,
        ) -> Result<OperatorOutput, EnvError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = ExecStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .run_with(
                &DispatchContext::new(DispatchId::new("contract"), OperatorId::from("op")),
                make_input(),
                &EnvironmentSpec::default(),
                &terminal,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl ExecMiddleware for Observer {
            async fn run(
                &self,
                ctx: &DispatchContext,
                input: OperatorInput,
                spec: &EnvironmentSpec,
                next: &dyn ExecNext,
            ) -> Result<OperatorOutput, EnvError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.run(ctx, input, spec).await
            }
        }

        let stack = ExecStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .run_with(
                &DispatchContext::new(DispatchId::new("contract"), OperatorId::from("op")),
                make_input(),
                &EnvironmentSpec::default(),
                &terminal,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl ExecMiddleware for DenyGuard {
            async fn run(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
                _spec: &EnvironmentSpec,
                _next: &dyn ExecNext,
            ) -> Result<OperatorOutput, EnvError> {
                Err(EnvError::ProvisionFailed("blocked by guard".into()))
            }
        }

        let stack = ExecStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack
            .run_with(
                &DispatchContext::new(DispatchId::new("contract"), OperatorId::from("op")),
                make_input(),
                &EnvironmentSpec::default(),
                &terminal,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 4. InferStack (skg-turn)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod infer_contract {
    use super::*;
    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use skg_turn::infer::{InferRequest, InferResponse};
    use skg_turn::infer_middleware::{InferMiddleware, InferNext, InferStack};
    use skg_turn::provider::ProviderError;
    use skg_turn::types::{StopReason, TokenUsage};

    fn make_request() -> InferRequest {
        InferRequest::new(vec![Message::new(Role::User, Content::text("hello"))])
    }

    fn make_response() -> InferResponse {
        InferResponse {
            content: Content::text("response"),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        }
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl InferNext for CountingTerminal {
        async fn infer(&self, _request: InferRequest) -> Result<InferResponse, ProviderError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(make_response())
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = InferStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.infer_with(make_request(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl InferMiddleware for Observer {
            async fn infer(
                &self,
                request: InferRequest,
                next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.infer(request).await
            }
        }

        let stack = InferStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.infer_with(make_request(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl InferMiddleware for DenyGuard {
            async fn infer(
                &self,
                _request: InferRequest,
                _next: &dyn InferNext,
            ) -> Result<InferResponse, ProviderError> {
                Err(ProviderError::ContentBlocked {
                    message: "blocked by guard".into(),
                })
            }
        }

        let stack = InferStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.infer_with(make_request(), &terminal).await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 5. EmbedStack (skg-turn)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod embed_contract {
    use super::*;
    use async_trait::async_trait;
    use skg_turn::embedding::{EmbedRequest, EmbedResponse, Embedding};
    use skg_turn::infer_middleware::{EmbedMiddleware, EmbedNext, EmbedStack};
    use skg_turn::provider::ProviderError;
    use skg_turn::types::TokenUsage;

    fn make_request() -> EmbedRequest {
        EmbedRequest::new(vec!["hello".into()])
    }

    fn make_response() -> EmbedResponse {
        EmbedResponse {
            embeddings: vec![Embedding {
                vector: vec![0.1, 0.2, 0.3],
            }],
            model: "test".into(),
            usage: TokenUsage::default(),
        }
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl EmbedNext for CountingTerminal {
        async fn embed(&self, _request: EmbedRequest) -> Result<EmbedResponse, ProviderError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(make_response())
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = EmbedStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.embed_with(make_request(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl EmbedMiddleware for Observer {
            async fn embed(
                &self,
                request: EmbedRequest,
                next: &dyn EmbedNext,
            ) -> Result<EmbedResponse, ProviderError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.embed(request).await
            }
        }

        let stack = EmbedStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.embed_with(make_request(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl EmbedMiddleware for DenyGuard {
            async fn embed(
                &self,
                _request: EmbedRequest,
                _next: &dyn EmbedNext,
            ) -> Result<EmbedResponse, ProviderError> {
                Err(ProviderError::ContentBlocked {
                    message: "blocked by guard".into(),
                })
            }
        }

        let stack = EmbedStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.embed_with(make_request(), &terminal).await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 6. SecretStack (skg-secret)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod secret_contract {
    use super::*;
    use async_trait::async_trait;
    use layer0::secret::SecretSource;
    use skg_secret::middleware::{SecretMiddleware, SecretNext, SecretStack};
    use skg_secret::{SecretError, SecretLease, SecretValue};

    fn make_source() -> SecretSource {
        SecretSource::OsKeystore {
            service: "test-app".into(),
        }
    }

    struct CountingTerminal(Arc<AtomicU32>);

    #[async_trait]
    impl SecretNext for CountingTerminal {
        async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(SecretLease::permanent(SecretValue::new(
                b"test-secret".to_vec(),
            )))
        }
    }

    #[tokio::test]
    async fn empty_stack_passes_through() {
        let terminal_count = Arc::new(AtomicU32::new(0));
        let stack = SecretStack::builder().build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.resolve_with(&make_source(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observer_sees_all_calls() {
        let observe_count = Arc::new(AtomicU32::new(0));
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct Observer(Arc<AtomicU32>);

        #[async_trait]
        impl SecretMiddleware for Observer {
            async fn resolve(
                &self,
                source: &SecretSource,
                next: &dyn SecretNext,
            ) -> Result<SecretLease, SecretError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.resolve(source).await
            }
        }

        let stack = SecretStack::builder()
            .observe(Arc::new(Observer(observe_count.clone())))
            .build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.resolve_with(&make_source(), &terminal).await;
        assert!(result.is_ok());
        assert_eq!(observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(terminal_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guard_short_circuits() {
        let terminal_count = Arc::new(AtomicU32::new(0));

        struct DenyGuard;

        #[async_trait]
        impl SecretMiddleware for DenyGuard {
            async fn resolve(
                &self,
                _source: &SecretSource,
                _next: &dyn SecretNext,
            ) -> Result<SecretLease, SecretError> {
                Err(SecretError::AccessDenied("blocked by guard".into()))
            }
        }

        let stack = SecretStack::builder().guard(Arc::new(DenyGuard)).build();
        let terminal = CountingTerminal(terminal_count.clone());

        let result = stack.resolve_with(&make_source(), &terminal).await;
        assert!(result.is_err());
        assert_eq!(
            terminal_count.load(Ordering::SeqCst),
            0,
            "terminal must not be called when guard short-circuits"
        );
    }
}
