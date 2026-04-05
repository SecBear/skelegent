//! Per-boundary middleware traits using the continuation pattern.
//!
//! Three middleware traits — one per layer0 protocol boundary:
//! - [`DispatchMiddleware`] wraps [`crate::Dispatcher`]`::dispatch`
//! - [`StoreMiddleware`] wraps [`crate::StateStore`] read/write
//! - [`ExecMiddleware`] wraps [`crate::Environment`]`::run`
//!
//! Provider middleware is NOT here — it lives in the turn layer (Layer 1)
//! because Provider is RPITIT, not object-safe.

use crate::dispatch::DispatchHandle;
use crate::dispatch_context::DispatchContext;
use crate::environment::EnvironmentSpec;
use crate::error::{EnvError, ProtocolError, StateError};
use crate::intent::Scope;
use crate::operator::{OperatorInput, OperatorOutput};
use crate::state::StoreOptions;
use async_trait::async_trait;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH MIDDLEWARE (wraps Dispatcher::dispatch)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in a dispatch middleware chain.
///
/// Call `dispatch()` to pass control to the inner layer.
/// Don't call it to short-circuit (guardrail halt).
#[async_trait]
pub trait DispatchNext: Send + Sync {
    /// Forward the dispatch to the next layer.
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, ProtocolError>;
}

/// Middleware wrapping `Dispatcher::dispatch`.
///
/// Code before `next.dispatch()` = pre-processing (input mutation, logging).
/// Code after `next.dispatch()` = post-processing (output mutation, metrics).
/// Not calling `next.dispatch()` = short-circuit (guardrail halt, cached response).
///
/// The `ctx` parameter carries dispatch correlation, identity, tracing, and
/// typed extensions. The operator being invoked is `ctx.operator_id`.
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    /// Intercept a dispatch call.
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, ProtocolError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STORE MIDDLEWARE (wraps StateStore read/write)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in a store-write middleware chain.
#[async_trait]
pub trait StoreWriteNext: Send + Sync {
    /// Forward the write to the next layer.
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
    ) -> Result<(), StateError>;
}

/// The next layer in a store-read middleware chain.
#[async_trait]
pub trait StoreReadNext: Send + Sync {
    /// Forward the read to the next layer.
    async fn read(&self, scope: &Scope, key: &str)
    -> Result<Option<serde_json::Value>, StateError>;
}

/// Middleware wrapping `StateStore` read and write operations.
///
/// Use for: encryption-at-rest, audit trails, caching, access control.
#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    /// Intercept a state write.
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError>;

    /// Intercept a state read. Default: pass through.
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
        next: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError> {
        next.read(scope, key).await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXEC MIDDLEWARE (wraps Environment::run)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in an exec middleware chain.
#[async_trait]
pub trait ExecNext: Send + Sync {
    /// Forward the execution to the next layer.
    async fn run(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError>;
}

/// Middleware wrapping `Environment::run`.
///
/// Use for: credential injection, isolation upgrades, resource enforcement.
#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    /// Intercept an environment execution.
    async fn run(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for dispatch operations.
///
/// Built via [`DispatchStack::builder()`]. Stacking order:
/// Observers (outermost) → Transformers → Guards (innermost).
///
/// Observers always run (even if a guard halts) because they're
/// the outermost layer. Guards see transformed input because
/// transformers are between observers and guards.
pub struct DispatchStack {
    /// Middleware layers in call order (outermost first).
    layers: Vec<Arc<dyn DispatchMiddleware>>,
}

/// Builder for [`DispatchStack`].
pub struct DispatchStackBuilder {
    observers: Vec<Arc<dyn DispatchMiddleware>>,
    transformers: Vec<Arc<dyn DispatchMiddleware>>,
    guards: Vec<Arc<dyn DispatchMiddleware>>,
}

impl DispatchStack {
    /// Start building a dispatch middleware stack.
    pub fn builder() -> DispatchStackBuilder {
        DispatchStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Dispatch through the middleware chain, ending at `terminal`.
    pub async fn dispatch_with(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        terminal: &dyn DispatchNext,
    ) -> Result<DispatchHandle, ProtocolError> {
        if self.layers.is_empty() {
            return terminal.dispatch(ctx, input).await;
        }
        let chain = DispatchChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.dispatch(ctx, input).await
    }
}

impl DispatchStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware (mutates input/output, always calls next).
    pub fn transform(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit by not calling next).
    pub fn guard(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> DispatchStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        DispatchStack { layers }
    }
}

struct DispatchChain<'a> {
    layers: &'a [Arc<dyn DispatchMiddleware>],
    index: usize,
    terminal: &'a dyn DispatchNext,
}

#[async_trait]
impl DispatchNext for DispatchChain<'_> {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, ProtocolError> {
        if self.index >= self.layers.len() {
            return self.terminal.dispatch(ctx, input).await;
        }
        let next = DispatchChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].dispatch(ctx, input, &next).await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STORE STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for state store operations.
///
/// Built via [`StoreStack::builder()`]. Same observer/transform/guard
/// ordering as [`DispatchStack`].
pub struct StoreStack {
    layers: Vec<Arc<dyn StoreMiddleware>>,
}

/// Builder for [`StoreStack`].
pub struct StoreStackBuilder {
    observers: Vec<Arc<dyn StoreMiddleware>>,
    transformers: Vec<Arc<dyn StoreMiddleware>>,
    guards: Vec<Arc<dyn StoreMiddleware>>,
}

impl StoreStack {
    /// Start building a store middleware stack.
    pub fn builder() -> StoreStackBuilder {
        StoreStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Write through the middleware chain, ending at `terminal`.
    pub async fn write_with(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
        terminal: &dyn StoreWriteNext,
    ) -> Result<(), StateError> {
        if self.layers.is_empty() {
            return terminal.write(scope, key, value, options).await;
        }
        let chain = StoreWriteChain {
            layers: &self.layers,
            index: 0,
            terminal,
            options,
        };
        chain.write(scope, key, value, options).await
    }

    /// Read through the middleware chain, ending at `terminal`.
    pub async fn read_with(
        &self,
        scope: &Scope,
        key: &str,
        terminal: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError> {
        if self.layers.is_empty() {
            return terminal.read(scope, key).await;
        }
        let chain = StoreReadChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.read(scope, key).await
    }
}

impl StoreStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn StoreMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware.
    pub fn transform(mut self, mw: Arc<dyn StoreMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit).
    pub fn guard(mut self, mw: Arc<dyn StoreMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> StoreStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        StoreStack { layers }
    }
}

struct StoreWriteChain<'a> {
    layers: &'a [Arc<dyn StoreMiddleware>],
    index: usize,
    terminal: &'a dyn StoreWriteNext,
    options: Option<&'a StoreOptions>,
}

#[async_trait]
impl StoreWriteNext for StoreWriteChain<'_> {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
    ) -> Result<(), StateError> {
        if self.index >= self.layers.len() {
            return self.terminal.write(scope, key, value, options).await;
        }
        let next = StoreWriteChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
            options: self.options,
        };
        self.layers[self.index]
            .write(scope, key, value, options, &next)
            .await
    }
}

struct StoreReadChain<'a> {
    layers: &'a [Arc<dyn StoreMiddleware>],
    index: usize,
    terminal: &'a dyn StoreReadNext,
}

#[async_trait]
impl StoreReadNext for StoreReadChain<'_> {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        if self.index >= self.layers.len() {
            return self.terminal.read(scope, key).await;
        }
        let next = StoreReadChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].read(scope, key, &next).await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Blueprint).
// Traits are hand-written (unique method signatures per boundary).
// Stack + Builder + Chain are structurally identical across all 6 boundaries.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXEC STACK (composed middleware chain)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A composed middleware stack for environment execution.
///
/// Built via [`ExecStack::builder()`]. Same observer/transform/guard
/// ordering as [`DispatchStack`].
pub struct ExecStack {
    layers: Vec<Arc<dyn ExecMiddleware>>,
}

/// Builder for [`ExecStack`].
pub struct ExecStackBuilder {
    observers: Vec<Arc<dyn ExecMiddleware>>,
    transformers: Vec<Arc<dyn ExecMiddleware>>,
    guards: Vec<Arc<dyn ExecMiddleware>>,
}

impl ExecStack {
    /// Start building an exec middleware stack.
    pub fn builder() -> ExecStackBuilder {
        ExecStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Execute through the middleware chain, ending at `terminal`.
    pub async fn run_with(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        terminal: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError> {
        if self.layers.is_empty() {
            return terminal.run(ctx, input, spec).await;
        }
        let chain = ExecChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.run(ctx, input, spec).await
    }
}

impl ExecStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn ExecMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware.
    pub fn transform(mut self, mw: Arc<dyn ExecMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit).
    pub fn guard(mut self, mw: Arc<dyn ExecMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> ExecStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        ExecStack { layers }
    }
}

struct ExecChain<'a> {
    layers: &'a [Arc<dyn ExecMiddleware>],
    index: usize,
    terminal: &'a dyn ExecNext,
}

#[async_trait]
impl ExecNext for ExecChain<'_> {
    async fn run(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError> {
        if self.index >= self.layers.len() {
            return self.terminal.run(ctx, input, spec).await;
        }
        let next = ExecChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].run(ctx, input, spec, &next).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{DispatchEvent, DispatchHandle};
    use crate::id::{DispatchId, OperatorId};

    /// Helper: create a DispatchHandle that immediately completes with the given output.
    fn immediate_handle(output: OperatorOutput) -> DispatchHandle {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("test"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        handle
    }

    #[tokio::test]
    async fn dispatch_middleware_is_object_safe() {
        struct TagMiddleware;

        #[async_trait]
        impl DispatchMiddleware for TagMiddleware {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                mut input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, ProtocolError> {
                input.metadata = serde_json::json!({"tagged": true});
                next.dispatch(_ctx, input).await
            }
        }

        let _mw: Box<dyn DispatchMiddleware> = Box::new(TagMiddleware);
    }

    #[tokio::test]
    async fn store_middleware_is_object_safe() {
        struct AuditStore;

        #[async_trait]
        impl StoreMiddleware for AuditStore {
            async fn write(
                &self,
                scope: &Scope,
                key: &str,
                value: serde_json::Value,
                options: Option<&StoreOptions>,
                next: &dyn StoreWriteNext,
            ) -> Result<(), StateError> {
                next.write(scope, key, value, options).await
            }
        }

        let _mw: Box<dyn StoreMiddleware> = Box::new(AuditStore);
    }

    #[tokio::test]
    async fn exec_middleware_is_object_safe() {
        struct CredentialInjector;

        #[async_trait]
        impl ExecMiddleware for CredentialInjector {
            async fn run(
                &self,
                _ctx: &DispatchContext,
                input: OperatorInput,
                spec: &EnvironmentSpec,
                next: &dyn ExecNext,
            ) -> Result<OperatorOutput, EnvError> {
                next.run(_ctx, input, spec).await
            }
        }

        let _mw: Box<dyn ExecMiddleware> = Box::new(CredentialInjector);
    }

    #[tokio::test]
    async fn dispatch_stack_observer_always_runs() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let counter = Arc::new(AtomicU32::new(0));

        struct CountObserver(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchMiddleware for CountObserver {
            async fn dispatch(
                &self,
                ctx: &DispatchContext,
                input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, ProtocolError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.dispatch(ctx, input).await
            }
        }

        struct HaltGuard;

        #[async_trait]
        impl DispatchMiddleware for HaltGuard {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
                _next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, ProtocolError> {
                Err(ProtocolError::unavailable("budget exceeded"))
            }
        }

        let stack = DispatchStack::builder()
            .observe(Arc::new(CountObserver(counter.clone())))
            .guard(Arc::new(HaltGuard))
            .build();

        struct EchoTerminal;

        #[async_trait]
        impl DispatchNext for EchoTerminal {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                input: OperatorInput,
            ) -> Result<DispatchHandle, ProtocolError> {
                Ok(immediate_handle(OperatorOutput::new(
                    input.message,
                    crate::Outcome::Terminal {
                        terminal: crate::TerminalOutcome::Completed,
                    },
                )))
            }
        }

        let input = OperatorInput::new(
            crate::content::Content::text("test"),
            crate::operator::TriggerType::User,
        );
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::from("a"));
        let result = stack.dispatch_with(&ctx, input, &EchoTerminal).await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_stack_transform_then_terminal() {
        struct Uppercaser;

        #[async_trait]
        impl DispatchMiddleware for Uppercaser {
            async fn dispatch(
                &self,
                ctx: &DispatchContext,
                mut input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<DispatchHandle, ProtocolError> {
                input.metadata = serde_json::json!({"transformed": true});
                next.dispatch(ctx, input).await
            }
        }

        struct EchoTerminal;

        #[async_trait]
        impl DispatchNext for EchoTerminal {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                input: OperatorInput,
            ) -> Result<DispatchHandle, ProtocolError> {
                Ok(immediate_handle(OperatorOutput::new(
                    input.message,
                    crate::Outcome::Terminal {
                        terminal: crate::TerminalOutcome::Completed,
                    },
                )))
            }
        }

        let stack = DispatchStack::builder()
            .transform(Arc::new(Uppercaser))
            .build();

        let input = OperatorInput::new(
            crate::content::Content::text("hello"),
            crate::operator::TriggerType::User,
        );
        let ctx = DispatchContext::new(DispatchId::new("test"), OperatorId::from("a"));
        let result = stack.dispatch_with(&ctx, input, &EchoTerminal).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn store_stack_write_through() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let write_count = Arc::new(AtomicU32::new(0));

        struct CountWrites(Arc<AtomicU32>);

        #[async_trait]
        impl StoreMiddleware for CountWrites {
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

        struct NoOpStore;

        #[async_trait]
        impl StoreWriteNext for NoOpStore {
            async fn write(
                &self,
                _scope: &Scope,
                _key: &str,
                _value: serde_json::Value,
                _options: Option<&StoreOptions>,
            ) -> Result<(), StateError> {
                Ok(())
            }
        }

        let stack = StoreStack::builder()
            .observe(Arc::new(CountWrites(write_count.clone())))
            .build();

        let scope = Scope::Operator {
            workflow: crate::id::WorkflowId::from("w"),
            operator: OperatorId::from("a"),
        };
        stack
            .write_with(&scope, "k", serde_json::json!(1), None, &NoOpStore)
            .await
            .unwrap();
        assert_eq!(write_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exec_stack_passthrough() {
        struct LogExec;

        #[async_trait]
        impl ExecMiddleware for LogExec {
            async fn run(
                &self,
                _ctx: &DispatchContext,
                input: OperatorInput,
                spec: &EnvironmentSpec,
                next: &dyn ExecNext,
            ) -> Result<OperatorOutput, EnvError> {
                next.run(_ctx, input, spec).await
            }
        }

        struct EchoExec;

        #[async_trait]
        impl ExecNext for EchoExec {
            async fn run(
                &self,
                _ctx: &DispatchContext,
                input: OperatorInput,
                _spec: &EnvironmentSpec,
            ) -> Result<OperatorOutput, EnvError> {
                Ok(OperatorOutput::new(
                    input.message,
                    crate::Outcome::Terminal {
                        terminal: crate::TerminalOutcome::Completed,
                    },
                ))
            }
        }

        let stack = ExecStack::builder().observe(Arc::new(LogExec)).build();

        let input = OperatorInput::new(
            crate::content::Content::text("run"),
            crate::operator::TriggerType::User,
        );
        let spec = EnvironmentSpec::default();
        let result = stack
            .run_with(
                &DispatchContext::new(
                    crate::id::DispatchId::new("test"),
                    crate::id::OperatorId::new("test"),
                ),
                input,
                &spec,
                &EchoExec,
            )
            .await;
        assert!(result.is_ok());
    }
}
