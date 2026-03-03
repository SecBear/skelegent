use layer0::content::Content;
use layer0::environment::{CredentialInjection, CredentialRef, Environment, EnvironmentSpec};
use layer0::error::EnvError;
use layer0::lifecycle::ObservableEvent;
use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};
use layer0::secret::{SecretAccessEvent, SecretAccessOutcome, SecretSource};
use layer0::test_utils::EchoOperator;
use neuron_env_local::{EnvironmentEventSink, LocalEnv};
use neuron_secret::{SecretError, SecretLease, SecretResolver, SecretValue};
use std::sync::Arc;
use std::sync::Mutex;

fn simple_input(msg: &str) -> OperatorInput {
    OperatorInput::new(Content::text(msg), TriggerType::User)
}

// --- Basic execution ---

#[tokio::test]
async fn passthrough_execution() {
    let env = LocalEnv::new(Arc::new(EchoOperator));
    let input = simple_input("hello");
    let spec = EnvironmentSpec::default();

    let output = env.run(input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("hello"));
}

#[tokio::test]
async fn preserves_operator_metadata() {
    let env = LocalEnv::new(Arc::new(EchoOperator));
    let input = simple_input("test");
    let spec = EnvironmentSpec::default();

    let output = env.run(input, &spec).await.unwrap();
    // EchoOperator returns default metadata
    assert_eq!(output.metadata.tokens_in, 0);
}

// --- Error propagation ---

/// An operator that always fails.
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
async fn propagates_operator_error() {
    let env = LocalEnv::new(Arc::new(FailingOperator));
    let input = simple_input("will fail");
    let spec = EnvironmentSpec::default();

    let result = env.run(input, &spec).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        EnvError::OperatorError(e) => {
            assert_eq!(e.to_string(), "non-retryable: always fails");
        }
        other => panic!("expected OperatorError, got: {other}"),
    }
}

// --- Object safety ---

#[tokio::test]
async fn usable_as_box_dyn_environment() {
    let env: Box<dyn Environment> = Box::new(LocalEnv::new(Arc::new(EchoOperator)));
    let input = simple_input("dyn test");
    let spec = EnvironmentSpec::default();

    let output = env.run(input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("dyn test"));
}

#[tokio::test]
async fn usable_as_arc_dyn_environment() {
    let env: Arc<dyn Environment> = Arc::new(LocalEnv::new(Arc::new(EchoOperator)));
    let input = simple_input("arc test");
    let spec = EnvironmentSpec::default();

    let output = env.run(input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("arc test"));
}

// --- Spec is ignored (passthrough) ---

#[tokio::test]
async fn ignores_spec_fields() {
    let env = LocalEnv::new(Arc::new(EchoOperator));
    let input = simple_input("spec ignored");
    let spec = EnvironmentSpec::default();

    // LocalEnv ignores the spec — it's a passthrough
    let output = env.run(input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("spec ignored"));
}

struct ReadEnvVarOperator {
    var_name: String,
}

#[async_trait::async_trait]
impl layer0::operator::Operator for ReadEnvVarOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
    ) -> Result<OperatorOutput, layer0::error::OperatorError> {
        let value = std::env::var(&self.var_name)
            .map_err(|e| layer0::error::OperatorError::NonRetryable(e.to_string()))?;
        Ok(OperatorOutput::new(
            Content::text(value),
            layer0::operator::ExitReason::Complete,
        ))
    }
}

struct StubSecretResolver {
    result: Result<Vec<u8>, SecretError>,
}

#[async_trait::async_trait]
impl SecretResolver for StubSecretResolver {
    async fn resolve(&self, _source: &SecretSource) -> Result<SecretLease, SecretError> {
        match &self.result {
            Ok(bytes) => Ok(SecretLease::permanent(SecretValue::new(bytes.clone()))),
            Err(SecretError::NotFound(msg)) => Err(SecretError::NotFound(msg.clone())),
            Err(SecretError::AccessDenied(msg)) => Err(SecretError::AccessDenied(msg.clone())),
            Err(SecretError::BackendError(msg)) => Err(SecretError::BackendError(msg.clone())),
            Err(SecretError::LeaseExpired(msg)) => Err(SecretError::LeaseExpired(msg.clone())),
            Err(SecretError::NoResolver(msg)) => Err(SecretError::NoResolver(msg.clone())),
            Err(SecretError::Other(_)) => Err(SecretError::BackendError("other error".into())),
            Err(_) => Err(SecretError::BackendError("other error".into())),
        }
    }
}

#[derive(Default)]
struct EventCollector {
    observable: Mutex<Vec<ObservableEvent>>,
    secret_access: Mutex<Vec<SecretAccessEvent>>,
}

impl EventCollector {
    fn observable_events(&self) -> Vec<ObservableEvent> {
        self.observable.lock().unwrap().clone()
    }

    fn secret_access_events(&self) -> Vec<SecretAccessEvent> {
        self.secret_access.lock().unwrap().clone()
    }
}

impl EnvironmentEventSink for EventCollector {
    fn emit_observable(&self, event: ObservableEvent) {
        self.observable.lock().unwrap().push(event);
    }

    fn emit_secret_access(&self, event: SecretAccessEvent) {
        self.secret_access.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn resolves_injects_and_emits_events() {
    const VAR_NAME: &str = "NEURON_ENV_LOCAL_TEST_API_KEY";
    const SECRET_VALUE: &str = "super-secret-token";
    // SAFETY: test-only; unique var name avoids cross-test interference.
    unsafe { std::env::remove_var(VAR_NAME) };

    let resolver: Arc<dyn SecretResolver> = Arc::new(StubSecretResolver {
        result: Ok(SECRET_VALUE.as_bytes().to_vec()),
    });
    let events = Arc::new(EventCollector::default());
    let env = LocalEnv::new(Arc::new(ReadEnvVarOperator {
        var_name: VAR_NAME.to_string(),
    }))
    .with_secret_resolver(resolver)
    .with_event_sink(events.clone());

    let mut spec = EnvironmentSpec::default();
    spec.credentials.push(CredentialRef::new(
        "anthropic-api-key",
        SecretSource::Custom {
            provider: "test".into(),
            config: serde_json::json!({}),
        },
        CredentialInjection::EnvVar {
            var_name: VAR_NAME.to_string(),
        },
    ));

    let output = env.run(simple_input("inject"), &spec).await.unwrap();
    assert_eq!(output.message, Content::text(SECRET_VALUE));
    assert!(std::env::var(VAR_NAME).is_err());

    let audit = events.secret_access_events();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].credential_name, "anthropic-api-key");
    assert_eq!(audit[0].outcome, SecretAccessOutcome::Resolved);

    let observable = events.observable_events();
    assert!(
        observable
            .iter()
            .any(|e| e.event_type == "environment.credential_resolved")
    );
    assert!(
        observable
            .iter()
            .any(|e| e.event_type == "environment.credential_injected")
    );

    let observable_json = serde_json::to_string(&observable).unwrap();
    assert!(!observable_json.contains(SECRET_VALUE));
}

#[tokio::test]
async fn credential_failures_are_sanitized_and_audited() {
    const LEAKED_SECRET: &str = "should-not-leak-secret-value";

    let resolver: Arc<dyn SecretResolver> = Arc::new(StubSecretResolver {
        result: Err(SecretError::BackendError(format!(
            "backend said secret={LEAKED_SECRET}"
        ))),
    });
    let events = Arc::new(EventCollector::default());
    let env = LocalEnv::new(Arc::new(EchoOperator))
        .with_secret_resolver(resolver)
        .with_event_sink(events.clone());

    let mut spec = EnvironmentSpec::default();
    spec.credentials.push(CredentialRef::new(
        "anthropic-api-key",
        SecretSource::Vault {
            mount: "secret".into(),
            path: "data/anthropic".into(),
        },
        CredentialInjection::EnvVar {
            var_name: "ANTHROPIC_API_KEY".into(),
        },
    ));

    let err = env.run(simple_input("inject"), &spec).await.unwrap_err();
    match &err {
        EnvError::CredentialFailed(msg) => {
            assert!(msg.contains("anthropic-api-key"));
            assert!(!msg.contains(LEAKED_SECRET));
        }
        other => panic!("expected credential error, got {other}"),
    }

    let err_debug = format!("{err:?}");
    assert!(!err_debug.contains(LEAKED_SECRET));

    let audit = events.secret_access_events();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, SecretAccessOutcome::Failed);
    assert!(
        !audit[0]
            .reason
            .clone()
            .unwrap_or_default()
            .contains(LEAKED_SECRET)
    );
}
