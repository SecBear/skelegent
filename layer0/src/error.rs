//! Error types for each protocol.

use thiserror::Error;

/// Operator execution errors.
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
#[non_exhaustive]
#[derive(Debug, Error)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
