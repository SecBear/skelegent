#![deny(missing_docs)]
//! Environment-aware Orchestrator.
//!
//! Routes operator dispatch through [`Environment::run`] based on registered
//! [`EnvironmentBinding`]s. Each operator is bound to an Environment + baseline
//! [`EnvironmentSpec`] at registration time.
//!
//! This is the bridge between "operators are location-agnostic" (layer0)
//! and "execution happens in containers" (skg-env-docker).

use async_trait::async_trait;
use layer0::dispatch::Dispatcher;
use layer0::environment::{Environment, EnvironmentSpec};
use layer0::error::{EnvError, OrchError};
use layer0::id::{OperatorId, WorkflowId};
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use layer0::effect::SignalPayload;
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
/// via `Arc<dyn Orchestrator>`. Registration is not thread-safe — complete
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
    pub fn bind_with(
        &mut self,
        id: OperatorId,
        env: Arc<dyn Environment>,
        spec: EnvironmentSpec,
    ) {
        self.bindings
            .insert(id.to_string(), EnvironmentBinding { env, spec });
    }
}

impl Default for EnvOrch {
    fn default() -> Self {
        Self::new()
    }
}

/// Map [`EnvError`] to [`OrchError`], preserving the inner [`OperatorError`]
/// when the environment propagated one.
fn env_err_to_orch(e: EnvError) -> OrchError {
    match e {
        EnvError::OperatorError(op_err) => OrchError::OperatorError(op_err),
        other => OrchError::DispatchFailed(other.to_string()),
    }
}

#[async_trait]
impl Dispatcher for EnvOrch {
    #[tracing::instrument(skip_all, fields(operator_id = %operator))]
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        if let Some(binding) = self.bindings.get(operator.as_str()) {
            binding
                .env
                .run(input, &binding.spec)
                .await
                .map_err(env_err_to_orch)
        } else if let Some(ref default) = self.default_env {
            default
                .run(input, &self.default_spec)
                .await
                .map_err(env_err_to_orch)
        } else {
            Err(OrchError::OperatorNotFound(operator.to_string()))
        }
    }
}

#[async_trait]
impl Orchestrator for EnvOrch {

    #[tracing::instrument(skip_all, fields(count = tasks.len()))]
    async fn dispatch_many(
        &self,
        tasks: Vec<(OperatorId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut handles = Vec::with_capacity(tasks.len());

        for (operator_id, input) in tasks {
            if let Some(binding) = self.bindings.get(operator_id.as_str()) {
                let env = Arc::clone(&binding.env);
                let spec = binding.spec.clone();
                handles.push(tokio::spawn(async move {
                    env.run(input, &spec).await.map_err(env_err_to_orch)
                }));
            } else if let Some(ref default) = self.default_env {
                let env = Arc::clone(default);
                let spec = self.default_spec.clone();
                handles.push(tokio::spawn(async move {
                    env.run(input, &spec).await.map_err(env_err_to_orch)
                }));
            } else {
                let name = operator_id.to_string();
                handles.push(tokio::spawn(
                    async move { Err(OrchError::OperatorNotFound(name)) },
                ));
            }
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(OrchError::DispatchFailed(e.to_string()))),
            }
        }

        results
    }

    async fn signal(
        &self,
        _target: &WorkflowId,
        _signal: SignalPayload,
    ) -> Result<(), OrchError> {
        Err(OrchError::DispatchFailed(
            "signals not supported in EnvOrch".into(),
        ))
    }

    async fn query(
        &self,
        _target: &WorkflowId,
        _query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError> {
        Err(OrchError::DispatchFailed(
            "query not supported in EnvOrch".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::id::OperatorId;
    use layer0::content::Content;
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
            .expect("dispatch should succeed");

        assert_eq!(result.message, Content::Text("hello".to_string()));
        assert!(matches!(result.exit_reason, ExitReason::Complete));
    }

    #[tokio::test]
    async fn dispatch_unbound_with_default_succeeds() {
        let orch = EnvOrch::with_default(echo_env(), EnvironmentSpec::default());

        let result = orch
            .dispatch(&OperatorId::new("anything"), input("fallback"))
            .await
            .expect("default env should handle unbound operator");

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

    #[tokio::test]
    async fn dispatch_many_runs_in_parallel() {
        let mut orch = EnvOrch::new();
        orch.bind_with(
            OperatorId::new("echo"),
            echo_env(),
            EnvironmentSpec::default(),
        );

        let tasks = vec![
            (OperatorId::new("echo"), input("a")),
            (OperatorId::new("echo"), input("b")),
            (OperatorId::new("echo"), input("c")),
        ];

        let results = orch.dispatch_many(tasks).await;
        assert_eq!(results.len(), 3);

        let messages: Vec<&Content> = results
            .iter()
            .map(|r| &r.as_ref().expect("each dispatch should succeed").message)
            .collect();

        assert_eq!(messages, vec![
            &Content::Text("a".into()),
            &Content::Text("b".into()),
            &Content::Text("c".into()),
        ]);
    }

    #[tokio::test]
    async fn dispatch_many_mixed_bound_and_unbound() {
        let mut orch = EnvOrch::new();
        orch.bind_with(
            OperatorId::new("echo"),
            echo_env(),
            EnvironmentSpec::default(),
        );

        let tasks = vec![
            (OperatorId::new("echo"), input("ok")),
            (OperatorId::new("missing"), input("fail")),
        ];

        let results = orch.dispatch_many(tasks).await;
        assert!(results[0].is_ok());
        assert!(matches!(
            results[1],
            Err(OrchError::OperatorNotFound(_))
        ));
    }

    #[tokio::test]
    async fn signal_returns_unsupported() {
        let orch = EnvOrch::new();
        let err = orch
            .signal(
                &WorkflowId::new("wf1"),
                SignalPayload::new("test", serde_json::Value::Null),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, OrchError::DispatchFailed(_)));
    }

    #[tokio::test]
    async fn query_returns_unsupported() {
        let orch = EnvOrch::new();
        let err = orch
            .query(&WorkflowId::new("wf1"), QueryPayload::new("test", serde_json::Value::Null))
            .await
            .unwrap_err();

        assert!(matches!(err, OrchError::DispatchFailed(_)));
    }
}
