//! Secret management data types — the stability contract for credential resolution.
//!
//! These are data types only. The actual resolution traits (`SecretResolver`,
//! `AuthProvider`, `CryptoProvider`) live in separate crates (`neuron-secret`,
//! `neuron-auth`, `neuron-crypto`). Layer 0 defines the vocabulary; higher
//! layers define the behavior.

use serde::{Deserialize, Serialize};

/// Where a secret is stored. This describes the BACKEND, not the delivery mechanism.
///
/// Delivery is handled by [`crate::environment::CredentialInjection`] (env var, file, sidecar).
/// A secret can live in Vault (source) and be delivered as an env var (injection) —
/// these are orthogonal concerns.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecretSource {
    /// HashiCorp Vault.
    Vault {
        /// The Vault mount point (e.g., "secret", "kv").
        mount: String,
        /// Path within the mount (e.g., "data/api-keys/anthropic").
        path: String,
    },
    /// AWS Secrets Manager.
    AwsSecretsManager {
        /// The secret ID or ARN.
        secret_id: String,
        /// AWS region (uses default if None).
        region: Option<String>,
    },
    /// GCP Secret Manager.
    GcpSecretManager {
        /// GCP project ID.
        project: String,
        /// Secret ID within the project.
        secret_id: String,
    },
    /// Azure Key Vault.
    AzureKeyVault {
        /// The vault URL (e.g., `https://myvault.vault.azure.net`).
        vault_url: String,
        /// The secret name within the vault.
        secret_name: String,
    },
    /// OS keystore (macOS Keychain, Windows DPAPI, Linux Secret Service).
    OsKeystore {
        /// The service name used to store/retrieve the credential.
        service: String,
    },
    /// Kubernetes Secret.
    Kubernetes {
        /// The namespace containing the secret.
        namespace: String,
        /// The secret resource name.
        name: String,
        /// The key within the secret's data map.
        key: String,
    },
    /// Hardware token (YubiKey PIV, HSM via PKCS#11).
    Hardware {
        /// The slot identifier (e.g., "9a" for YubiKey PIV auth slot).
        slot: String,
    },
    /// Custom source for future backends.
    Custom {
        /// The backend provider identifier.
        provider: String,
        /// Backend-specific configuration.
        config: serde_json::Value,
    },
}

/// Outcome of a secret access attempt.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SecretAccessOutcome {
    /// Secret was successfully resolved.
    Resolved,
    /// Access was denied by policy.
    Denied,
    /// Resolution failed (backend error, timeout, etc.).
    Failed,
    /// A lease was renewed.
    Renewed,
    /// A lease was released/revoked.
    Released,
}

/// Lifecycle event emitted when a secret is accessed.
///
/// This is part of the observability vocabulary (like [`crate::lifecycle::BudgetEvent`]).
/// The orchestrator or a hook can subscribe to these events for audit logging,
/// compliance tracking, or anomaly detection.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretAccessEvent {
    /// The credential name (label, not the secret value).
    pub credential_name: String,
    /// Where resolution was attempted.
    pub source: SecretSource,
    /// What happened.
    pub outcome: SecretAccessOutcome,
    /// When it happened (Unix timestamp milliseconds).
    pub timestamp_ms: u64,
    /// Opaque lease identifier for renewal/revocation tracking.
    pub lease_id: Option<String>,
    /// Lease TTL in seconds, if applicable.
    pub lease_ttl_secs: Option<u64>,
    /// Sanitized failure reason (never contains secret material).
    pub reason: Option<String>,
    /// Workflow ID for correlation.
    pub workflow_id: Option<String>,
    /// Operator ID for correlation.
    pub operator_id: Option<String>,
    /// Trace ID for distributed tracing.
    pub trace_id: Option<String>,
}

impl SecretSource {
    /// Returns a short, telemetry-safe kind tag for this source variant.
    ///
    /// Safe to log, include in error messages, and use in metrics —
    /// never contains secret material.
    pub fn kind(&self) -> &'static str {
        #[allow(unreachable_patterns)]
        match self {
            SecretSource::Vault { .. } => "vault",
            SecretSource::AwsSecretsManager { .. } => "aws",
            SecretSource::GcpSecretManager { .. } => "gcp",
            SecretSource::AzureKeyVault { .. } => "azure",
            SecretSource::OsKeystore { .. } => "os_keystore",
            SecretSource::Kubernetes { .. } => "kubernetes",
            SecretSource::Hardware { .. } => "hardware",
            SecretSource::Custom { .. } => "custom",
            _ => "unknown",
        }
    }
}

impl SecretAccessEvent {
    /// Create a new secret access event with required fields.
    pub fn new(
        credential_name: impl Into<String>,
        source: SecretSource,
        outcome: SecretAccessOutcome,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            credential_name: credential_name.into(),
            source,
            outcome,
            timestamp_ms,
            lease_id: None,
            lease_ttl_secs: None,
            reason: None,
            workflow_id: None,
            operator_id: None,
            trace_id: None,
        }
    }
}
