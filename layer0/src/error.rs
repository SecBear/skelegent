//! Error types for each protocol.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ── ProtocolError ───────────────────────────────────────────────────────────

/// Uniform error type for all protocol boundaries (v2).
///
/// Every protocol trait returns `Result<T, ProtocolError>`. Each v1 error type
/// (`OperatorError`, `OrchError`) has a `From` impl that maps into this type
/// so downstream callers migrating one boundary at a time compile without
/// changes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Machine-readable error code.
    pub code: ErrorCode,
    /// Human-readable error message.
    pub message: String,
    /// Whether the caller should retry.
    pub retryable: bool,
    /// Optional structured details for logging/diagnostics.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub details: HashMap<String, String>,
}

/// Machine-readable error classification.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The requested resource was not found.
    NotFound,
    /// The input was invalid.
    InvalidInput,
    /// A transient unavailability (network, provider, sub-dispatch).
    Unavailable,
    /// A conflict prevented the operation (e.g. halted by policy).
    Conflict,
    /// An internal/unrecoverable error.
    Internal,
}

impl ProtocolError {
    /// Create a new protocol error.
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            details: HashMap::new(),
        }
    }

    /// Add a detail key-value pair.
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    /// Convenience: internal non-retryable error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message, false)
    }

    /// Convenience: not-found error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message, false)
    }

    /// Convenience: unavailable retryable error.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message, true)
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}

impl std::error::Error for ProtocolError {}

// ── OperatorError (v1 — deprecated) ────────────────────────────────────────

/// Operator execution errors.
///
/// **Deprecated:** Use [`ProtocolError`] instead. This type is retained for
/// backward compatibility and will be removed in a future release.
#[deprecated(note = "Use ProtocolError instead")]
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OperatorError {
    /// Provider/model failure.
    #[error("model error: {source}")]
    Model {
        /// The underlying provider error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        /// Whether the caller should retry.
        retryable: bool,
    },

    /// Sub-dispatch failure.
    #[error("sub-dispatch error in {operator}: {source}")]
    SubDispatch {
        /// Which operator/tool failed.
        operator: String,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Context assembly failed before the model call.
    #[error("context assembly: {source}")]
    ContextAssembly {
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The operator failed but retrying might succeed.
    /// The orchestrator's retry policy decides.
    #[error("retryable: {message}")]
    Retryable {
        /// Description.
        message: String,
        /// Optional underlying cause.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// The operator failed and retrying won't help.
    /// Budget exceeded, invalid input, safety refusal.
    #[error("non-retryable: {message}")]
    NonRetryable {
        /// Description.
        message: String,
        /// Optional underlying cause.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// A rule or policy halted execution — distinct from a permanent failure.
    #[error("halted: {reason}")]
    Halted {
        /// Why execution was halted.
        reason: String,
    },

    /// Catch-all. Include context.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[allow(deprecated)]
impl OperatorError {
    /// Non-retryable model error from a message.
    pub fn model(msg: impl Into<String>) -> Self {
        let s: String = msg.into();
        Self::Model {
            source: s.into(),
            retryable: false,
        }
    }

    /// Retryable model error from a source error.
    pub fn model_retryable(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Model {
            source: Box::new(source),
            retryable: true,
        }
    }

    /// Context assembly error from a source error.
    pub fn context_assembly(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::ContextAssembly {
            source: Box::new(source),
        }
    }

    /// Simple retryable error from a message.
    pub fn retryable(msg: impl Into<String>) -> Self {
        Self::Retryable {
            message: msg.into(),
            source: None,
        }
    }

    /// Simple non-retryable error from a message.
    pub fn non_retryable(msg: impl Into<String>) -> Self {
        Self::NonRetryable {
            message: msg.into(),
            source: None,
        }
    }

    /// Whether this error represents a condition that might succeed on retry.
    ///
    /// Returns `true` for `Model { retryable: true, .. }` and `Retryable { .. }`.
    /// All other variants are considered permanent failures.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Model { retryable, .. } => *retryable,
            Self::Retryable { .. } => true,
            _ => false,
        }
    }
}

/// Orchestration errors.
///
/// **Deprecated:** Use [`ProtocolError`] instead. This type is retained for
/// backward compatibility and will be removed in a future release.
#[deprecated(note = "Use ProtocolError instead")]
#[non_exhaustive]
#[derive(Debug, Error)]
#[allow(deprecated)]
pub enum OrchError {
    /// The requested operator was not found.
    #[error("operator not found: {0}")]
    OperatorNotFound(String),

    /// The requested workflow was not found.
    #[error("workflow not found: {0}")]
    WorkflowNotFound(String),

    /// Dispatching a turn failed.
    #[error("dispatch failed: {0}")]
    DispatchFailed(String),

    /// Signal delivery failed.
    #[error("signal delivery failed: {0}")]
    SignalFailed(String),

    /// An operator error propagated through orchestration.
    #[error("operator error: {0}")]
    OperatorError(#[from] OperatorError),

    /// An environment error propagated through orchestration.
    #[error("environment error: {0}")]
    EnvironmentError(EnvError),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[allow(deprecated)]
impl From<EnvError> for OrchError {
    fn from(e: EnvError) -> Self {
        match e {
            // Preserve the inner OperatorError so callers can match on it directly.
            EnvError::OperatorError(op) => OrchError::OperatorError(op),
            other => OrchError::EnvironmentError(other),
        }
    }
}

/// State errors.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum StateError {
    /// Key not found in the given scope.
    #[error("not found: {scope}/{key}")]
    NotFound {
        /// The scope that was searched.
        scope: String,
        /// The key that was not found.
        key: String,
    },

    /// A write operation failed.
    #[error("write failed: {0}")]
    WriteFailed(String),

    /// Serialization or deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Environment errors.
#[non_exhaustive]
#[derive(Debug, Error)]
#[allow(deprecated)]
pub enum EnvError {
    /// Failed to provision the execution environment.
    #[error("provisioning failed: {0}")]
    ProvisionFailed(String),

    /// The isolation boundary was violated.
    #[error("isolation violation: {0}")]
    IsolationViolation(String),

    /// Credential injection failed.
    #[error("credential injection failed: {0}")]
    CredentialFailed(String),

    /// A resource limit was exceeded.
    #[error("resource limit exceeded: {0}")]
    ResourceExceeded(String),

    /// An operator error propagated through the environment.
    #[error("operator error: {0}")]
    OperatorError(#[from] OperatorError),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

// ── From impls: v1 → ProtocolError ──────────────────────────────────────────

#[allow(deprecated)]
impl From<OperatorError> for ProtocolError {
    fn from(e: OperatorError) -> Self {
        match e {
            OperatorError::Model {
                retryable: true,
                source,
            } => ProtocolError::new(ErrorCode::Unavailable, source.to_string(), true)
                .with_detail("kind", "operator_error")
                .with_detail("variant", "model"),
            OperatorError::Model {
                retryable: false,
                source,
            } => ProtocolError::new(ErrorCode::Internal, source.to_string(), false),
            OperatorError::SubDispatch { operator, source } => {
                ProtocolError::new(ErrorCode::Unavailable, source.to_string(), false)
                    .with_detail("variant", "sub_dispatch")
                    .with_detail("operator", operator)
            }
            OperatorError::ContextAssembly { source } => {
                ProtocolError::new(ErrorCode::InvalidInput, source.to_string(), false)
            }
            OperatorError::Retryable { message, .. } => {
                ProtocolError::new(ErrorCode::Unavailable, message, true)
            }
            OperatorError::NonRetryable { message, .. } => {
                ProtocolError::new(ErrorCode::Internal, message, false)
            }
            OperatorError::Halted { reason } => {
                ProtocolError::new(ErrorCode::Conflict, reason, false)
            }
            OperatorError::Other(source) => {
                ProtocolError::new(ErrorCode::Internal, source.to_string(), false)
            }
        }
    }
}

#[allow(deprecated)]
impl From<OrchError> for ProtocolError {
    fn from(e: OrchError) -> Self {
        match e {
            OrchError::OperatorNotFound(name) => ProtocolError::new(
                ErrorCode::NotFound,
                format!("operator not found: {name}"),
                false,
            )
            .with_detail("variant", "operator_not_found")
            .with_detail("name", name),
            OrchError::WorkflowNotFound(name) => ProtocolError::new(
                ErrorCode::NotFound,
                format!("workflow not found: {name}"),
                false,
            ),
            OrchError::DispatchFailed(msg) => ProtocolError::new(
                ErrorCode::Unavailable,
                format!("dispatch failed: {msg}"),
                true,
            ),
            OrchError::SignalFailed(msg) => ProtocolError::new(
                ErrorCode::Unavailable,
                format!("signal delivery failed: {msg}"),
                true,
            ),
            OrchError::OperatorError(inner) => ProtocolError::from(inner),
            OrchError::EnvironmentError(inner) => ProtocolError::from(inner),
            OrchError::Other(source) => {
                ProtocolError::new(ErrorCode::Internal, source.to_string(), false)
            }
        }
    }
}

impl From<EnvError> for ProtocolError {
    fn from(e: EnvError) -> Self {
        match e {
            EnvError::ProvisionFailed(msg) => ProtocolError::new(
                ErrorCode::Unavailable,
                format!("provisioning failed: {msg}"),
                true,
            ),
            EnvError::IsolationViolation(msg) => ProtocolError::new(
                ErrorCode::Internal,
                format!("isolation violation: {msg}"),
                false,
            ),
            EnvError::CredentialFailed(msg) => ProtocolError::new(
                ErrorCode::Unavailable,
                format!("credential injection failed: {msg}"),
                true,
            ),
            EnvError::ResourceExceeded(msg) => ProtocolError::new(
                ErrorCode::Conflict,
                format!("resource limit exceeded: {msg}"),
                false,
            ),
            #[allow(deprecated)]
            EnvError::OperatorError(inner) => ProtocolError::from(inner),
            EnvError::Other(source) => {
                ProtocolError::new(ErrorCode::Internal, source.to_string(), false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(deprecated)]
    #[test]
    fn operator_error_retryable_classification() {
        assert!(OperatorError::model_retryable(std::io::Error::other("x")).is_retryable());
        assert!(OperatorError::retryable("transient").is_retryable());
        assert!(!OperatorError::model("permanent").is_retryable());
        assert!(!OperatorError::non_retryable("fatal").is_retryable());
        assert!(
            !OperatorError::Halted {
                reason: "stopped".into(),
            }
            .is_retryable()
        );
    }

    #[allow(deprecated)]
    #[test]
    fn operator_error_to_protocol_error_mapping() {
        let model_retry = OperatorError::model_retryable(std::io::Error::other("transient"));
        let pe: ProtocolError = model_retry.into();
        assert_eq!(pe.code, ErrorCode::Unavailable);
        assert!(pe.retryable);
        assert_eq!(pe.details.get("variant").map(String::as_str), Some("model"));

        let model_perm = OperatorError::model("permanent");
        let pe: ProtocolError = model_perm.into();
        assert_eq!(pe.code, ErrorCode::Internal);
        assert!(!pe.retryable);

        let ctx = OperatorError::context_assembly(std::io::Error::other("bad input"));
        let pe: ProtocolError = ctx.into();
        assert_eq!(pe.code, ErrorCode::InvalidInput);
        assert!(!pe.retryable);

        let halted = OperatorError::Halted {
            reason: "policy".into(),
        };
        let pe: ProtocolError = halted.into();
        assert_eq!(pe.code, ErrorCode::Conflict);
        assert!(!pe.retryable);
    }

    #[allow(deprecated)]
    #[test]
    fn orch_error_to_protocol_error_mapping() {
        let not_found = OrchError::OperatorNotFound("echo".into());
        let pe: ProtocolError = not_found.into();
        assert_eq!(pe.code, ErrorCode::NotFound);
        assert!(!pe.retryable);
        assert_eq!(pe.details.get("name").map(String::as_str), Some("echo"));

        let dispatch = OrchError::DispatchFailed("timeout".into());
        let pe: ProtocolError = dispatch.into();
        assert_eq!(pe.code, ErrorCode::Unavailable);
        assert!(pe.retryable);
    }

    #[test]
    fn protocol_error_display() {
        let pe = ProtocolError::internal("something broke");
        assert!(pe.to_string().contains("something broke"));
        assert!(pe.to_string().contains("Internal"));
    }

    #[test]
    fn protocol_error_serde_round_trip() {
        let pe =
            ProtocolError::new(ErrorCode::NotFound, "missing", false).with_detail("key", "value");
        let json = serde_json::to_string(&pe).unwrap();
        let back: ProtocolError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, ErrorCode::NotFound);
        assert_eq!(back.message, "missing");
        assert!(!back.retryable);
        assert_eq!(back.details.get("key").map(String::as_str), Some("value"));
    }
}
