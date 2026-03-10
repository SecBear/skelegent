#![deny(missing_docs)]
//! Docker-backed implementation of layer0's `Environment` trait.
//!
//! `DockerEnvironment` executes operators inside hardened Docker containers,
//! communicating over gRPC (tonic). The host creates/starts a container running
//! `skg-runner`, sends an `ExecuteRequest` via gRPC, and tears down the
//! container after execution.
//!
//! Key responsibilities:
//! - Resolve credentials on the host and inject as env vars at container creation
//! - Manage container lifecycle (pull, create, start, stop, remove)
//! - Communicate with the in-container runner over gRPC (TCP in v1, UDS in v2)
//! - Apply security hardening: seccomp, no-new-privileges, non-root, read-only rootfs
//! - Retry with exponential backoff using idempotency keys

mod cleanup;
mod client;
mod config;
mod lifecycle;
mod security;

pub use config::{DockerEnvConfig, PullPolicy, RetryConfig, ReusePolicy, Transport};

use async_trait::async_trait;
use bollard::Docker;
use layer0::environment::{
    CredentialInjection, Environment, EnvironmentSpec, IsolationBoundary,
};
use layer0::error::EnvError;
use layer0::id::OperatorId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::secret::SecretAccessEvent;
use skg_secret::SecretResolver;
use std::sync::Arc;
use std::time::Duration;

use cleanup::ContainerGuard;
use lifecycle::CreateContainerParams;

/// Generated protobuf/gRPC types for the runner service.
#[allow(missing_docs)]
pub mod proto {
    /// Runner service definitions (Execute, ExecuteStream, Health).
    pub mod runner {
        tonic::include_proto!("skg.runner.v1");
    }
    /// State store proxy definitions (Read, Write, List, Delete, Search).
    pub mod state_proxy {
        tonic::include_proto!("skg.state_proxy.v1");
    }
}

/// Sink for environment audit events.
///
/// Allows Docker mode to emit `SecretAccessEvent` audit records
/// for credential resolution attempts.
pub trait EnvironmentEventSink: Send + Sync {
    /// Emit an audit event for secret access activity.
    fn emit_secret_access(&self, event: SecretAccessEvent);
}

/// Docker-backed environment that executes operators in isolated containers.
///
/// Owns a bollard `Docker` client and delegates operator execution to a
/// `skg-runner` process inside the container via gRPC. Credentials are
/// resolved on the host and injected as environment variables at container
/// creation time.
pub struct DockerEnvironment {
    docker: Docker,
    operator: OperatorId,
    default_image: String,

    /// Host endpoint the container calls back to (state proxy, etc.).
    host_callback_base: url::Url,

    secret_resolver: Option<Arc<dyn SecretResolver>>,
    #[allow(dead_code)] // Used for audit logging in follow-up
    event_sink: Option<Arc<dyn EnvironmentEventSink>>,
    cfg: DockerEnvConfig,
}

impl DockerEnvironment {
    /// Start building a `DockerEnvironment` with required fields.
    pub fn builder(
        docker: Docker,
        operator: OperatorId,
        default_image: String,
        host_callback_base: url::Url,
    ) -> DockerEnvironmentBuilder {
        DockerEnvironmentBuilder {
            docker,
            operator,
            default_image,
            host_callback_base,
            secret_resolver: None,
            event_sink: None,
            cfg: None,
        }
    }

    /// Derive the container image from the spec, falling back to the default.
    fn resolve_image(&self, spec: &EnvironmentSpec) -> String {
        for boundary in &spec.isolation {
            if let IsolationBoundary::Container { image: Some(img) } = boundary {
                return img.clone();
            }
        }
        self.default_image.clone()
    }

    /// Resolve credentials from the spec, returning env var pairs.
    ///
    /// - `EnvVar` injection: resolved via the secret resolver and collected.
    /// - `File` injection: rejected with an explicit error (not supported in v1).
    /// - `Sidecar` injection: skipped (handled externally).
    async fn resolve_credentials(
        &self,
        spec: &EnvironmentSpec,
    ) -> Result<Vec<(String, String)>, EnvError> {
        let mut env_vars = Vec::new();

        for cred_ref in &spec.credentials {
            match &cred_ref.injection {
                CredentialInjection::EnvVar { var_name } => {
                    let resolver = self.secret_resolver.as_ref().ok_or_else(|| {
                        EnvError::CredentialFailed(format!(
                            "no secret resolver configured for credential '{}'",
                            cred_ref.name
                        ))
                    })?;

                    let lease = resolver
                        .resolve(&cred_ref.source)
                        .await
                        .map_err(|e| {
                            EnvError::CredentialFailed(format!(
                                "failed to resolve credential '{}': {e}",
                                cred_ref.name
                            ))
                        })?;

                    // Extract the secret value as a UTF-8 string for env var injection.
                    let value = lease.value.with_bytes(|bytes| {
                        String::from_utf8(bytes.to_vec()).map_err(|_| {
                            EnvError::CredentialFailed(format!(
                                "credential '{}' is not valid UTF-8",
                                cred_ref.name
                            ))
                        })
                    })?;

                    env_vars.push((var_name.clone(), value));
                }
                CredentialInjection::File { .. } => {
                    return Err(EnvError::CredentialFailed(
                        "file injection not supported in Docker v1".to_string(),
                    ));
                }
                // Sidecar injection is handled outside the environment.
                _ => {}
            }
        }

        Ok(env_vars)
    }

    /// Determine the gRPC container port from the transport config.
    fn container_port(&self) -> u16 {
        match &self.cfg.transport {
            Transport::Grpc { port } => *port,
            // v1 only supports TCP; UDS and HTTP are future work.
            Transport::GrpcUds { .. } => 50051,
            Transport::Http { port } => *port,
        }
    }
}

/// Builder for `DockerEnvironment`.
pub struct DockerEnvironmentBuilder {
    docker: Docker,
    operator: OperatorId,
    default_image: String,
    host_callback_base: url::Url,
    secret_resolver: Option<Arc<dyn SecretResolver>>,
    event_sink: Option<Arc<dyn EnvironmentEventSink>>,
    cfg: Option<DockerEnvConfig>,
}

impl DockerEnvironmentBuilder {
    /// Set the secret resolver for credential resolution.
    pub fn secret_resolver(mut self, resolver: Arc<dyn SecretResolver>) -> Self {
        self.secret_resolver = Some(resolver);
        self
    }

    /// Set the event sink for audit events.
    pub fn event_sink(mut self, sink: Arc<dyn EnvironmentEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// Override the default configuration.
    pub fn config(mut self, cfg: DockerEnvConfig) -> Self {
        self.cfg = Some(cfg);
        self
    }

    /// Build the `DockerEnvironment`.
    pub fn build(self) -> DockerEnvironment {
        DockerEnvironment {
            docker: self.docker,
            operator: self.operator,
            default_image: self.default_image,
            host_callback_base: self.host_callback_base,
            secret_resolver: self.secret_resolver,
            event_sink: self.event_sink,
            cfg: self.cfg.unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Environment for DockerEnvironment {
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError> {
        // 0) Derive image from spec (IsolationBoundary::Container { image }) or default
        let image = self.resolve_image(spec);
        let container_port = self.container_port();

        // 1) Resolve credentials on the host, collecting env var pairs
        let env_vars = self.resolve_credentials(spec).await?;

        // 2) Ensure the image is present locally (pull if needed per policy)
        lifecycle::ensure_image(&self.docker, &image, &self.cfg.pull_policy).await?;

        // 3) Build hardened host config with resource limits
        let host_config = security::hardened_host_config(spec.resources.as_ref());

        // Generate a session key for labeling and the gRPC request
        let session_key = input
            .session
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let host_callback_url = self.host_callback_base.to_string();

        // 4) Create the container
        let container_id = lifecycle::create_container(
            &self.docker,
            CreateContainerParams {
                image: &image,
                env_vars,
                container_port,
                host_config,
                operator_label: self.operator.as_str(),
                session_label: &session_key,
                host_callback_url: &host_callback_url,
            },
        )
        .await?;

        // 5) Install RAII guard — container is cleaned up even on panic/early return
        let should_remove = matches!(self.cfg.reuse_policy, ReusePolicy::Fresh);
        let _guard = ContainerGuard::new(self.docker.clone(), container_id.clone(), should_remove);

        // 6) Start the container
        lifecycle::start_container(&self.docker, &container_id).await?;

        // 7) Discover the ephemeral host port for gRPC
        let endpoint =
            lifecycle::discover_grpc_endpoint(&self.docker, &container_id, container_port).await?;

        // 8) Connect to the runner's gRPC service
        let connect_timeout = Duration::from_secs(30);
        let mut runner_client = client::connect_runner(&endpoint, connect_timeout).await?;

        // 9) Build the ExecuteRequest
        let idempotency_key = uuid::Uuid::new_v4().to_string();

        let input_bytes = serde_json::to_vec(&input).map_err(|e| {
            EnvError::ProvisionFailed(format!("failed to serialize OperatorInput: {e}"))
        })?;
        let spec_bytes = serde_json::to_vec(spec).map_err(|e| {
            EnvError::ProvisionFailed(format!("failed to serialize EnvironmentSpec: {e}"))
        })?;

        let request = proto::runner::ExecuteRequest {
            operator: self.operator.as_str().to_string(),
            input: input_bytes,
            spec: spec_bytes,
            idempotency_key,
            session_key,
        };

        // 10) Execute with timeout + retry
        let execute_future = client::execute_with_retry(
            &mut runner_client,
            request,
            &self.cfg.retry,
        );

        let response = tokio::time::timeout(self.cfg.default_timeout, execute_future)
            .await
            .map_err(|_| {
                EnvError::ProvisionFailed(format!(
                    "operator execution timed out after {:?}",
                    self.cfg.default_timeout
                ))
            })?
            // Flatten the inner Result
            ?;

        // 11) Deserialize OperatorOutput from the response bytes
        let output: OperatorOutput = serde_json::from_slice(&response.output).map_err(|e| {
            EnvError::ProvisionFailed(format!("failed to deserialize OperatorOutput: {e}"))
        })?;

        // _guard drops here → container stopped and removed (if Fresh policy)
        Ok(output)
    }
}
