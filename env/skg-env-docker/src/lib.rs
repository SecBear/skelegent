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

mod config;

pub use config::{DockerEnvConfig, PullPolicy, RetryConfig, ReusePolicy, Transport};

use async_trait::async_trait;
use bollard::Docker;
use layer0::environment::{Environment, EnvironmentSpec};
use layer0::error::EnvError;
use layer0::id::OperatorId;
use layer0::operator::{OperatorInput, OperatorOutput};
use layer0::secret::SecretAccessEvent;
use skg_secret::SecretResolver;
use std::sync::Arc;

/// Generated protobuf/gRPC types for the runner service.
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
    event_sink: Option<Arc<dyn EnvironmentEventSink>>,
    cfg: DockerEnvConfig,
}

#[async_trait]
impl Environment for DockerEnvironment {
    async fn run(
        &self,
        _input: OperatorInput,
        _spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError> {
        // 0) Derive execution plan (timeouts, resources, image selection)
        // 1) Resolve + materialize credentials (host-side)
        // 2) Ensure image present (optional pull)
        // 3) Create container with hardened config
        // 4) Start container
        // 5) Connect gRPC client (TCP port or UDS path)
        // 6) Call Execute(req) with deadline + idempotency_key
        //    — retries with exponential backoff on transient failures
        // 7) On deadline exceeded: cancel stream, kill container
        // 8) Always: stop+remove unless reuse policy says otherwise
        // 9) Return OperatorOutput
        todo!("DockerEnvironment::run — implement container lifecycle + gRPC call")
    }
}
