//! The Environment protocol — isolation, credentials, and resource constraints.

use crate::dispatch_context::DispatchContext;
use crate::{
    error::EnvError, operator::OperatorInput, operator::OperatorOutput, secret::SecretSource,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Protocol ④ — Environment
///
/// How an operator executes within an isolated context. Handles isolation,
/// credentials, and resource constraints. The environment mediates
/// between the caller and the execution context.
///
/// Implementations:
/// - LocalEnvironment: no isolation, direct execution (dev mode)
/// - DockerEnvironment: spin up container, execute, tear down
/// - K8sEnvironment: create pod with network policy, execute, delete
/// - WasmEnvironment: sandboxed Wasm runtime
///
/// The critical insight: the Environment owns or has access to whatever
/// it needs to execute an operator — the same pattern as Dispatcher.
/// `run()` takes only data (`OperatorInput` + `EnvironmentSpec`), not a
/// function reference. How the Environment resolves and invokes an Operator
/// is an internal concern.
///
/// For `LocalEnvironment`, the operator is stored as an `Arc<dyn Operator>`
/// field at construction time. For `DockerEnvironment`, container
/// configuration is stored — it serializes the `OperatorInput`, runs an
/// operator process inside the container, and deserializes the `OperatorOutput`.
/// Same trait, radically different isolation.
#[async_trait]
pub trait Environment: Send + Sync {
    /// Execute an operator within this environment's isolation boundary.
    ///
    /// The implementation:
    /// 1. Provisions any required isolation (container, sandbox, etc.)
    /// 2. Injects credentials according to the spec
    /// 3. Applies resource limits
    /// 4. Executes the operator (mechanism is internal to the implementation)
    /// 5. Captures the output
    /// 6. Tears down the isolation context
    async fn run(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError>;
}

/// Declarative specification for an execution environment.
/// This is serializable so it can live in config files (YAML, TOML).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvironmentSpec {
    /// Isolation boundaries to apply, outermost first.
    #[serde(default)]
    pub isolation: Vec<IsolationBoundary>,

    /// Credentials to make available inside the environment.
    #[serde(default)]
    pub credentials: Vec<CredentialRef>,

    /// Resource limits.
    pub resources: Option<ResourceLimits>,

    /// Network policy.
    pub network: Option<NetworkPolicy>,
}

/// A single isolation boundary. Multiple boundaries compose
/// (e.g., container + gVisor + network policy = defense in depth).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IsolationBoundary {
    /// OS process boundary.
    Process,
    /// Container (Docker, containerd, etc.).
    Container {
        /// Optional container image to use.
        image: Option<String>,
    },
    /// Syscall interception (gVisor runsc).
    Gvisor,
    /// Hardware-enforced VM (Kata Containers).
    MicroVm,
    /// WebAssembly sandbox.
    Wasm {
        /// Optional Wasm runtime to use.
        runtime: Option<String>,
    },
    /// Network-level isolation.
    NetworkPolicy {
        /// Network rules to apply.
        rules: Vec<NetworkRule>,
    },
    /// Future isolation types.
    Custom {
        /// The custom boundary type identifier.
        boundary_type: String,
        /// Configuration for this boundary.
        config: serde_json::Value,
    },
}

/// A reference to a credential that should be injected into the environment.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRef {
    /// Name of the credential (e.g., "anthropic-api-key").
    pub name: String,
    /// Where the secret is stored (backend).
    pub source: SecretSource,
    /// How to inject it.
    pub injection: CredentialInjection,
}

/// How a credential is injected into the environment.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialInjection {
    /// Set as environment variable.
    EnvVar {
        /// The environment variable name.
        var_name: String,
    },
    /// Mount as file.
    File {
        /// The file path to mount the credential at.
        path: String,
    },
    /// Inject via sidecar/proxy (agent never sees the secret).
    Sidecar,
}

/// Resource limits for the execution environment.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU limit, e.g. "1.0", "500m".
    pub cpu: Option<String>,
    /// Memory limit, e.g. "2Gi", "512Mi".
    pub memory: Option<String>,
    /// Disk limit, e.g. "10Gi".
    pub disk: Option<String>,
    /// GPU allocation, e.g. "1" or "nvidia.com/gpu: 1".
    pub gpu: Option<String>,
}

/// Network policy for the execution environment.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Default action for traffic not matching any rule.
    pub default: NetworkAction,
    /// Explicit rules.
    pub rules: Vec<NetworkRule>,
}

/// A single network rule.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRule {
    /// Domain or CIDR to match.
    pub destination: String,
    /// Port (optional, None = all ports).
    pub port: Option<u16>,
    /// Allow or deny.
    pub action: NetworkAction,
}

/// Network traffic action.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAction {
    /// Allow the traffic.
    Allow,
    /// Deny the traffic.
    Deny,
}

impl CredentialRef {
    /// Create a new credential reference.
    pub fn new(
        name: impl Into<String>,
        source: SecretSource,
        injection: CredentialInjection,
    ) -> Self {
        Self {
            name: name.into(),
            source,
            injection,
        }
    }
}

impl NetworkPolicy {
    /// Create a new network policy.
    pub fn new(default: NetworkAction, rules: Vec<NetworkRule>) -> Self {
        Self { default, rules }
    }
}

impl NetworkRule {
    /// Create a new network rule.
    pub fn new(destination: impl Into<String>, action: NetworkAction) -> Self {
        Self {
            destination: destination.into(),
            port: None,
            action,
        }
    }
}
