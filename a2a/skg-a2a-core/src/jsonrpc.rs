//! JSON-RPC 2.0 envelope types for the A2A protocol.

use serde::{Deserialize, Serialize};

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,
    /// The method to invoke.
    pub method: String,
    /// Request identifier. Null for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    /// Method parameters.
    #[serde(default)]
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    /// Create a new JSON-RPC request.
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            id: Some(serde_json::Value::String(uuid::Uuid::new_v4().to_string())),
            params,
        }
    }

    /// Create a notification (no id, no response expected).
    pub fn notification(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            id: None,
            params,
        }
    }
}

/// A JSON-RPC 2.0 success response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,
    /// Request identifier echoed back.
    pub id: Option<serde_json::Value>,
    /// The result payload.
    pub result: serde_json::Value,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result,
        }
    }
}

/// A JSON-RPC 2.0 error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorResponse {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,
    /// Request identifier echoed back.
    pub id: Option<serde_json::Value>,
    /// The error.
    pub error: JsonRpcError,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Numeric error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured error data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcErrorResponse {
    /// Create an error response.
    pub fn error(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            error: JsonRpcError {
                code,
                message: message.into(),
                data: None,
            },
        }
    }
}

/// Well-known A2A JSON-RPC method names.
pub mod methods {
    /// Send a message to an agent.
    pub const SEND_MESSAGE: &str = "message/send";
    /// Send a streaming message.
    pub const SEND_STREAMING_MESSAGE: &str = "message/stream";
    /// Get a task by ID.
    pub const GET_TASK: &str = "tasks/get";
    /// Cancel a task.
    pub const CANCEL_TASK: &str = "tasks/cancel";
    /// Subscribe to task updates.
    pub const SUBSCRIBE_TO_TASK: &str = "tasks/subscribe";
    /// List tasks.
    pub const LIST_TASKS: &str = "tasks/list";
    /// Create push notification config.
    pub const CREATE_PUSH_CONFIG: &str = "tasks/pushNotificationConfigs/create";
    /// Get push notification config.
    pub const GET_PUSH_CONFIG: &str = "tasks/pushNotificationConfigs/get";
    /// List push notification configs.
    pub const LIST_PUSH_CONFIGS: &str = "tasks/pushNotificationConfigs/list";
    /// Delete push notification config.
    pub const DELETE_PUSH_CONFIG: &str = "tasks/pushNotificationConfigs/delete";
    /// Get the extended agent card.
    pub const GET_EXTENDED_AGENT_CARD: &str = "extendedAgentCard/get";
}
