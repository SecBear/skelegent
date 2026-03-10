//! Configuration types for `DockerEnvironment`.

use std::path::PathBuf;
use std::time::Duration;

/// Configuration for the Docker environment.
pub struct DockerEnvConfig {
    /// Transport mechanism for host↔runner communication.
    pub transport: Transport,
    /// Image pull strategy.
    pub pull_policy: PullPolicy,
    /// Container reuse strategy.
    pub reuse_policy: ReusePolicy,
    /// Default timeout for operator execution.
    pub default_timeout: Duration,
    /// Retry configuration with exponential backoff.
    pub retry: RetryConfig,
}

/// Transport mechanism between host and container runner.
pub enum Transport {
    /// gRPC over TCP (v1 default). Container port published to ephemeral host port.
    Grpc {
        /// Container port the runner listens on.
        port: u16,
    },
    /// gRPC over Unix domain socket (v2). Socket file mounted into container.
    GrpcUds {
        /// Path to the UDS socket file.
        socket_path: PathBuf,
    },
    /// HTTP/JSON convenience adapter for debugging/tooling.
    Http {
        /// Container port the HTTP adapter listens on.
        port: u16,
    },
}

/// Image pull policy for container images.
pub enum PullPolicy {
    /// Pull only if the image is not present locally.
    IfMissing,
    /// Always pull, even if present locally.
    Always,
    /// Never pull; fail if not present locally.
    Never,
}

/// Container reuse policy.
pub enum ReusePolicy {
    /// Always create a fresh container per execution.
    Fresh,
    /// Reuse containers within the same session.
    PerSession,
    /// Caller manages container lifecycle explicitly.
    Explicit,
}

/// Exponential backoff configuration for retries on transient failures.
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Initial backoff duration before first retry.
    pub initial_backoff: Duration,
    /// Maximum backoff duration.
    pub max_backoff: Duration,
    /// Backoff multiplier per retry.
    pub multiplier: f64,
}
