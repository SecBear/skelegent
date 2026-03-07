//! Per-boundary middleware traits using the continuation pattern.
//!
//! Three middleware traits — one per layer0 protocol boundary:
//! - [`DispatchMiddleware`] wraps [`Orchestrator::dispatch`]
//! - [`StoreMiddleware`] wraps [`StateStore`] read/write
//! - [`ExecMiddleware`] wraps [`Environment::run`]
//!
//! Provider middleware is NOT here — it lives in the turn layer (Layer 1)
//! because Provider is RPITIT, not object-safe.

use crate::effect::Scope;
use crate::environment::EnvironmentSpec;
use crate::error::{EnvError, OrchError, StateError};
use crate::id::AgentId;
use crate::operator::{OperatorInput, OperatorOutput};
use async_trait::async_trait;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH MIDDLEWARE (wraps Orchestrator::dispatch)
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
        agent: &AgentId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError>;
}

/// Middleware wrapping `Orchestrator::dispatch`.
///
/// Code before `next.dispatch()` = pre-processing (input mutation, logging).
/// Code after `next.dispatch()` = post-processing (output mutation, metrics).
/// Not calling `next.dispatch()` = short-circuit (guardrail halt, cached response).
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    /// Intercept a dispatch call.
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError>;
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
    ) -> Result<(), StateError>;
}

/// The next layer in a store-read middleware chain.
#[async_trait]
pub trait StoreReadNext: Send + Sync {
    /// Forward the read to the next layer.
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError>;
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
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_middleware_is_object_safe() {
        struct TagMiddleware;

        #[async_trait]
        impl DispatchMiddleware for TagMiddleware {
            async fn dispatch(
                &self,
                agent: &AgentId,
                mut input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<OperatorOutput, OrchError> {
                input.metadata = serde_json::json!({"tagged": true});
                next.dispatch(agent, input).await
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
                next: &dyn StoreWriteNext,
            ) -> Result<(), StateError> {
                next.write(scope, key, value).await
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
                input: OperatorInput,
                spec: &EnvironmentSpec,
                next: &dyn ExecNext,
            ) -> Result<OperatorOutput, EnvError> {
                next.run(input, spec).await
            }
        }

        let _mw: Box<dyn ExecMiddleware> = Box::new(CredentialInjector);
    }
}
