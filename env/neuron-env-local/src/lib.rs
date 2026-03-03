#![deny(missing_docs)]
//! Local implementation of layer0's Environment trait.
//!
//! `LocalEnv` executes an operator directly in-process, with optional
//! credential resolution and injection:
//! - Resolve credentials through a [`neuron_secret::SecretResolver`]
//! - Inject credential material according to `EnvironmentSpec.credentials`
//! - Emit audit/lifecycle events through [`EnvironmentEventSink`]
//!
//! This crate is intentionally "local mode" only: no container isolation,
//! no remote execution boundaries, no network policy enforcement.

use async_trait::async_trait;
use layer0::duration::DurationMs;
use layer0::environment::{CredentialInjection, CredentialRef, Environment, EnvironmentSpec};
use layer0::error::EnvError;
use layer0::lifecycle::{EventSource, ObservableEvent};
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::secret::{SecretAccessEvent, SecretAccessOutcome};
use neuron_secret::{SecretError, SecretLease, SecretResolver};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Sink for environment credential/audit events.
///
/// This allows local mode to emit both:
/// - `SecretAccessEvent` audit records for credential resolution attempts
/// - `ObservableEvent` lifecycle events for resolution/injection milestones
pub trait EnvironmentEventSink: Send + Sync {
    /// Emit an observable lifecycle event.
    fn emit_observable(&self, event: ObservableEvent);

    /// Emit an audit event for secret access activity.
    fn emit_secret_access(&self, event: SecretAccessEvent);
}

/// Local passthrough environment.
///
/// Owns an `Arc<dyn Operator>` and delegates directly to it. Optional
/// components can be configured for credential resolution/injection.
pub struct LocalEnv {
    op: Arc<dyn Operator>,
    secret_resolver: Option<Arc<dyn SecretResolver>>,
    event_sink: Option<Arc<dyn EnvironmentEventSink>>,
}

impl LocalEnv {
    /// Create a new local environment wrapping the given operator.
    ///
    /// By default this is a pure passthrough executor with no credential
    /// resolution and no event emission.
    pub fn new(op: Arc<dyn Operator>) -> Self {
        Self {
            op,
            secret_resolver: None,
            event_sink: None,
        }
    }

    /// Attach a secret resolver used for `EnvironmentSpec.credentials`.
    pub fn with_secret_resolver(mut self, resolver: Arc<dyn SecretResolver>) -> Self {
        self.secret_resolver = Some(resolver);
        self
    }

    /// Attach an event sink for audit/lifecycle emission.
    pub fn with_event_sink(mut self, sink: Arc<dyn EnvironmentEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    async fn resolve_and_inject(
        &self,
        spec: &EnvironmentSpec,
        correlation: &CorrelationContext,
        started_at: Instant,
    ) -> Result<InjectionCleanup, EnvError> {
        let mut cleanup = InjectionCleanup::default();

        for credential in &spec.credentials {
            let resolver = match &self.secret_resolver {
                Some(resolver) => resolver,
                None => {
                    let reason = "resolver not configured";
                    self.emit_resolution_failure(credential, reason, correlation, started_at);
                    return Err(EnvError::CredentialFailed(format!(
                        "credential '{}' resolution failed for source '{}': {}",
                        credential.name,
                        credential.source.kind(),
                        reason
                    )));
                }
            };

            let lease = match resolver.resolve(&credential.source).await {
                Ok(lease) => lease,
                Err(err) => {
                    let reason = sanitize_secret_error(&err);
                    self.emit_resolution_failure(credential, reason, correlation, started_at);
                    return Err(EnvError::CredentialFailed(format!(
                        "credential '{}' resolution failed for source '{}': {}",
                        credential.name,
                        credential.source.kind(),
                        reason
                    )));
                }
            };

            self.emit_resolution_success(credential, &lease, correlation, started_at);

            if let Err(reason) = inject_credential(credential, &lease, &mut cleanup) {
                self.emit_observable(
                    "environment.credential_injection_failed",
                    json!({
                        "credential_name": credential.name,
                        "source_kind": credential.source.kind(),
                        "injection": injection_kind(&credential.injection),
                        "reason": reason,
                    }),
                    correlation,
                    started_at,
                );
                return Err(EnvError::CredentialFailed(format!(
                    "credential '{}' injection failed: {}",
                    credential.name, reason
                )));
            }

            self.emit_observable(
                "environment.credential_injected",
                json!({
                    "credential_name": credential.name,
                    "source_kind": credential.source.kind(),
                    "injection": injection_kind(&credential.injection),
                }),
                correlation,
                started_at,
            );
        }

        Ok(cleanup)
    }

    fn emit_resolution_success(
        &self,
        credential: &CredentialRef,
        lease: &SecretLease,
        correlation: &CorrelationContext,
        started_at: Instant,
    ) {
        self.emit_secret_access(
            credential,
            SecretAccessOutcome::Resolved,
            lease_reason(lease),
            lease.lease_id.clone(),
            lease_ttl_secs(lease),
            correlation,
        );
        self.emit_observable(
            "environment.credential_resolved",
            json!({
                "credential_name": credential.name,
                "source_kind": credential.source.kind(),
                "injection": injection_kind(&credential.injection),
            }),
            correlation,
            started_at,
        );
    }

    fn emit_resolution_failure(
        &self,
        credential: &CredentialRef,
        reason: &str,
        correlation: &CorrelationContext,
        started_at: Instant,
    ) {
        self.emit_secret_access(
            credential,
            SecretAccessOutcome::Failed,
            Some(reason.to_owned()),
            None,
            None,
            correlation,
        );
        self.emit_observable(
            "environment.credential_resolution_failed",
            json!({
                "credential_name": credential.name,
                "source_kind": credential.source.kind(),
                "injection": injection_kind(&credential.injection),
                "reason": reason,
            }),
            correlation,
            started_at,
        );
    }

    fn emit_secret_access(
        &self,
        credential: &CredentialRef,
        outcome: SecretAccessOutcome,
        reason: Option<String>,
        lease_id: Option<String>,
        lease_ttl_secs: Option<u64>,
        correlation: &CorrelationContext,
    ) {
        let Some(sink) = &self.event_sink else {
            return;
        };

        let mut event = SecretAccessEvent::new(
            credential.name.clone(),
            credential.source.clone(),
            outcome,
            unix_time_ms(),
        );
        event.reason = reason;
        event.lease_id = lease_id;
        event.lease_ttl_secs = lease_ttl_secs;
        event.workflow_id = correlation.workflow_id.clone();
        event.agent_id = correlation.agent_id.clone();
        event.trace_id = correlation.trace_id.clone();
        sink.emit_secret_access(event);
    }

    fn emit_observable(
        &self,
        event_type: &str,
        data: serde_json::Value,
        correlation: &CorrelationContext,
        started_at: Instant,
    ) {
        let Some(sink) = &self.event_sink else {
            return;
        };

        let mut event = ObservableEvent::new(
            EventSource::Environment,
            event_type,
            DurationMs::from_millis(started_at.elapsed().as_millis() as u64),
            data,
        );
        event.trace_id = correlation.trace_id.clone();
        event.workflow_id = correlation.workflow_id.clone().map(Into::into);
        event.agent_id = correlation.agent_id.clone().map(Into::into);
        sink.emit_observable(event);
    }
}

#[async_trait]
impl Environment for LocalEnv {
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError> {
        let started_at = Instant::now();
        let correlation = CorrelationContext::from_metadata(&input.metadata);
        let cleanup = self
            .resolve_and_inject(spec, &correlation, started_at)
            .await?;

        let result = self
            .op
            .execute(input)
            .await
            .map_err(EnvError::OperatorError);
        drop(cleanup);
        result
    }
}

#[derive(Default)]
struct InjectionCleanup {
    actions: Vec<CleanupAction>,
}

enum CleanupAction {
    RestoreEnvVar {
        var_name: String,
        previous: Option<String>,
    },
    RestoreFile {
        path: PathBuf,
        previous: Option<Vec<u8>>,
    },
}

impl InjectionCleanup {
    fn push(&mut self, action: CleanupAction) {
        self.actions.push(action);
    }
}

impl Drop for InjectionCleanup {
    fn drop(&mut self) {
        for action in self.actions.drain(..).rev() {
            match action {
                CleanupAction::RestoreEnvVar { var_name, previous } => match previous {
                    // SAFETY: cleanup runs on drop in the same thread that set the var.
                    // The test harness ensures single-threaded env access.
                    Some(value) => unsafe { std::env::set_var(var_name, value) },
                    None => unsafe { std::env::remove_var(var_name) },
                },
                CleanupAction::RestoreFile { path, previous } => match previous {
                    Some(bytes) => {
                        let _ = fs::write(path, bytes);
                    }
                    None => {
                        let _ = fs::remove_file(path);
                    }
                },
            }
        }
    }
}

fn inject_credential(
    credential: &CredentialRef,
    lease: &SecretLease,
    cleanup: &mut InjectionCleanup,
) -> Result<(), String> {
    match &credential.injection {
        CredentialInjection::EnvVar { var_name } => {
            let value = lease
                .value
                .with_bytes(|bytes| std::str::from_utf8(bytes).map(str::to_owned))
                .map_err(|_| {
                    "credential value is not valid UTF-8 for env var injection".to_owned()
                })?;
            let previous = std::env::var(var_name).ok();
            // SAFETY: credential injection targets a single operator's process-local
            // environment. The caller is responsible for ensuring no concurrent env access.
            unsafe { std::env::set_var(var_name, value) };
            cleanup.push(CleanupAction::RestoreEnvVar {
                var_name: var_name.clone(),
                previous,
            });
            Ok(())
        }
        CredentialInjection::File { path } => {
            let path_buf = PathBuf::from(path);
            let previous = fs::read(&path_buf).ok();

            if let Some(parent) = path_buf.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("unable to create parent directory for '{path}': {e}"))?;
            }

            lease
                .value
                .with_bytes(|bytes| fs::write(&path_buf, bytes))
                .map_err(|e| format!("unable to write credential file '{path}': {e}"))?;

            cleanup.push(CleanupAction::RestoreFile {
                path: path_buf,
                previous,
            });
            Ok(())
        }
        CredentialInjection::Sidecar => Ok(()),
        _ => Err("unsupported credential injection mode".to_owned()),
    }
}

fn lease_reason(lease: &SecretLease) -> Option<String> {
    if lease.is_expired() {
        Some("lease expired".to_owned())
    } else {
        None
    }
}

fn lease_ttl_secs(lease: &SecretLease) -> Option<u64> {
    lease.expires_at.and_then(|expires_at| {
        expires_at
            .duration_since(SystemTime::now())
            .ok()
            .map(|ttl| ttl.as_secs())
    })
}

fn injection_kind(injection: &CredentialInjection) -> &'static str {
    match injection {
        CredentialInjection::EnvVar { .. } => "env_var",
        CredentialInjection::File { .. } => "file",
        CredentialInjection::Sidecar => "sidecar",
        _ => "unknown",
    }
}

fn sanitize_secret_error(err: &SecretError) -> &'static str {
    match err {
        SecretError::NotFound(_) => "secret not found",
        SecretError::AccessDenied(_) => "access denied",
        SecretError::BackendError(_) => "backend error",
        SecretError::LeaseExpired(_) => "lease expired",
        SecretError::NoResolver(_) => "no resolver",
        SecretError::Other(_) => "internal error",
        _ => "internal error",
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
struct CorrelationContext {
    workflow_id: Option<String>,
    agent_id: Option<String>,
    trace_id: Option<String>,
}

impl CorrelationContext {
    fn from_metadata(metadata: &serde_json::Value) -> Self {
        Self {
            workflow_id: metadata
                .get("workflow_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            agent_id: metadata
                .get("agent_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            trace_id: metadata
                .get("trace_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::error::OperatorError;
    use layer0::operator::{ExitReason, OperatorOutput, TriggerType};

    struct EchoOperator;

    #[async_trait]
    impl Operator for EchoOperator {
        async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    struct FailOperator;

    #[async_trait]
    impl Operator for FailOperator {
        async fn execute(&self, _input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
            Err(OperatorError::Model("deliberate failure".into()))
        }
    }

    #[tokio::test]
    async fn local_env_delegates_to_operator() {
        let op: Arc<dyn Operator> = Arc::new(EchoOperator);
        let env = LocalEnv::new(op);

        let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
        let spec = EnvironmentSpec::default();

        let output = env.run(input, &spec).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "hello");
    }

    #[tokio::test]
    async fn local_env_propagates_operator_error() {
        let op: Arc<dyn Operator> = Arc::new(FailOperator);
        let env = LocalEnv::new(op);

        let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
        let spec = EnvironmentSpec::default();

        let result = env.run(input, &spec).await;
        assert!(result.is_err());
    }

    #[test]
    fn local_env_implements_environment() {
        fn _assert_env<T: Environment>() {}
        _assert_env::<LocalEnv>();
    }
}
