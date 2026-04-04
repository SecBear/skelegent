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

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
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
            EnvError::Other(source) => {
                ProtocolError::new(ErrorCode::Internal, source.to_string(), false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
