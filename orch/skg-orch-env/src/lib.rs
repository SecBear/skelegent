#![deny(missing_docs)]
//! Environment-aware Dispatcher.
//!
//! Routes operator dispatch through [`Environment::run`] based on registered
//! [`EnvironmentBinding`]s. Each operator is bound to an Environment + baseline
//! [`EnvironmentSpec`] at registration time.
//!
//! This is the bridge between "operators are location-agnostic" (layer0)
//! and "execution happens in containers" (skg-env-docker).

use async_trait::async_trait;
use layer0::dispatch::{DispatchEvent, DispatchHandle, Dispatcher};
use layer0::environment::{Environment, EnvironmentSpec};
use layer0::error::OrchError;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::OperatorInput;
use std::collections::HashMap;
use std::sync::Arc;

/// Binds an operator to an execution environment with a baseline spec.
///
/// The spec is cloned per dispatch — it's cheap (a few small vecs).
pub struct EnvironmentBinding {
    /// The environment that will execute the operator.
    pub env: Arc<dyn Environment>,
    /// Baseline spec applied on every dispatch (isolation, credentials, limits).
    pub spec: EnvironmentSpec,
}

/// Environment-aware orchestrator.
///
/// Constructed once, populated with [`bind`](Self::bind) calls, then shared
/// via `Arc<dyn Dispatcher>`. Registration is not thread-safe — complete
/// all bindings before sharing.
pub struct EnvOrch {
    bindings: HashMap<String, EnvironmentBinding>,
    /// Fallback environment for operators without an explicit binding.
    default_env: Option<Arc<dyn Environment>>,
    default_spec: EnvironmentSpec,
}

impl EnvOrch {
    /// Create a new orchestrator with no bindings and no default environment.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            default_env: None,
            default_spec: EnvironmentSpec::default(),
        }
    }

    /// Create a new orchestrator with a fallback environment for unbound operators.
    pub fn with_default(env: Arc<dyn Environment>, spec: EnvironmentSpec) -> Self {
        Self {
            bindings: HashMap::new(),
            default_env: Some(env),
            default_spec: spec,
        }
    }

    /// Register an operator→environment binding.
    pub fn bind(&mut self, id: OperatorId, binding: EnvironmentBinding) {
        self.bindings.insert(id.to_string(), binding);
    }

    /// Convenience: bind an operator to an environment + spec without constructing
    /// an [`EnvironmentBinding`] manually.
    pub fn bind_with(&mut self, id: OperatorId, env: Arc<dyn Environment>, spec: EnvironmentSpec) {
        self.bindings
            .insert(id.to_string(), EnvironmentBinding { env, spec });
    }
}

impl Default for EnvOrch {
    fn default() -> Self {
        Self::new()
    }
}


#[async_trait]
impl Dispatcher for EnvOrch {
    #[tracing::instrument(skip_all, fields(operator_id = %operator))]
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError> {
        let (env, spec) = if let Some(binding) = self.bindings.get(operator.as_str()) {
            (binding.env.clone(), binding.spec.clone())
        } else if let Some(ref default) = self.default_env {
            (default.clone(), self.default_spec.clone())
        } else {
            return Err(OrchError::OperatorNotFound(operator.to_string()));
        };

        let (handle, sender) = DispatchHandle::channel(DispatchId::new(operator.as_str()));
        tokio::spawn(async move {
            match env.run(input, &spec).await {
                Ok(output) => {
                    let _ = sender.send(DispatchEvent::Completed { output }).await;
                }
                Err(err) => {
                    let _ = sender
                        .send(DispatchEvent::Failed {
                            error: err.into(),
                        })
                        .await;
                }
            }
        });
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::id::OperatorId;
    use layer0::operator::{ExitReason, OperatorInput, TriggerType};
    use layer0::test_utils::{EchoOperator, LocalEnvironment};

    fn echo_env() -> Arc<dyn Environment> {
        Arc::new(LocalEnvironment::new(Arc::new(EchoOperator)))
    }

    fn input(msg: &str) -> OperatorInput {
        OperatorInput::new(Content::Text(msg.to_string()), TriggerType::Task)
    }

    #[tokio::test]
    async fn dispatch_bound_operator_succeeds() {
        let mut orch = EnvOrch::new();
        orch.bind_with(
            OperatorId::new("echo"),
            echo_env(),
            EnvironmentSpec::default(),
        );

        let result = orch
            .dispatch(&OperatorId::new("echo"), input("hello"))
            .await
            .expect("dispatch should succeed")
            .collect()
            .await
            .expect("collect should succeed");

        assert_eq!(result.message, Content::Text("hello".to_string()));
        assert!(matches!(result.exit_reason, ExitReason::Complete));
    }

    #[tokio::test]
    async fn dispatch_unbound_with_default_succeeds() {
        let orch = EnvOrch::with_default(echo_env(), EnvironmentSpec::default());

        let result = orch
            .dispatch(&OperatorId::new("anything"), input("fallback"))
            .await
            .expect("default env should handle unbound operator")
            .collect()
            .await
            .expect("collect should succeed");

        assert_eq!(result.message, Content::Text("fallback".to_string()));
    }

    #[tokio::test]
    async fn dispatch_unbound_without_default_returns_not_found() {
        let orch = EnvOrch::new();

        let err = orch
            .dispatch(&OperatorId::new("missing"), input("nope"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, OrchError::OperatorNotFound(ref name) if name == "missing"),
            "expected OperatorNotFound, got: {err:?}"
        );
    }
}
