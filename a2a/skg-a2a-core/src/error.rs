//! A2A protocol errors with JSON-RPC error codes.

use thiserror::Error;

/// A2A protocol errors, each carrying its JSON-RPC error code.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum A2aError {
    /// The requested task was not found.
    #[error("task not found: {task_id}")]
    TaskNotFound {
        /// The task ID that was not found.
        task_id: String,
    },
    /// The task cannot be canceled in its current state.
    #[error("task not cancelable: {task_id}")]
    TaskNotCancelable {
        /// The task ID.
        task_id: String,
    },
    /// Push notifications are not supported by this agent.
    #[error("push notifications not supported")]
    PushNotificationNotSupported,
    /// The requested operation is not supported.
    #[error("unsupported operation: {operation}")]
    UnsupportedOperation {
        /// The operation that was requested.
        operation: String,
    },
    /// The content type is not supported.
    #[error("content type not supported: {content_type}")]
    ContentTypeNotSupported {
        /// The unsupported content type.
        content_type: String,
    },
    /// The agent produced an invalid response.
    #[error("invalid agent response: {reason}")]
    InvalidAgentResponse {
        /// Why the response was invalid.
        reason: String,
    },
    /// JSON-RPC parse error.
    #[error("parse error: {reason}")]
    ParseError {
        /// Parse failure details.
        reason: String,
    },
    /// JSON-RPC invalid request.
    #[error("invalid request: {reason}")]
    InvalidRequest {
        /// Why the request was invalid.
        reason: String,
    },
    /// JSON-RPC method not found.
    #[error("method not found: {method}")]
    MethodNotFound {
        /// The method that was not found.
        method: String,
    },
    /// Internal error.
    #[error("internal error: {reason}")]
    Internal {
        /// Error details.
        reason: String,
    },
}

impl A2aError {
    /// Return the JSON-RPC error code for this error.
    pub fn code(&self) -> i32 {
        match self {
            Self::ParseError { .. } => -32700,
            Self::InvalidRequest { .. } => -32600,
            Self::MethodNotFound { .. } => -32601,
            Self::Internal { .. } => -32603,
            Self::TaskNotFound { .. } => -32001,
            Self::TaskNotCancelable { .. } => -32002,
            Self::PushNotificationNotSupported => -32003,
            Self::UnsupportedOperation { .. } => -32004,
            Self::ContentTypeNotSupported { .. } => -32005,
            Self::InvalidAgentResponse { .. } => -32006,
        }
    }

    /// Convert to a JSON-RPC error object.
    pub fn to_jsonrpc_error(&self) -> crate::jsonrpc::JsonRpcError {
        crate::jsonrpc::JsonRpcError {
            code: self.code(),
            message: self.to_string(),
            data: None,
        }
    }
}
