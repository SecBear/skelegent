//! A2A protocol wire types.
//!
//! Hand-written Rust structs that serialize to JSON matching the A2A specification.
//! No transport dependencies — this is pure data types + serde.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// The role of a message sender in the A2A protocol.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum A2aRole {
    /// Unspecified role (proto3 default).
    #[serde(rename = "ROLE_UNSPECIFIED")]
    Unspecified,
    /// A human or upstream caller.
    #[serde(rename = "ROLE_USER", alias = "user")]
    User,
    /// An AI agent.
    #[serde(rename = "ROLE_AGENT", alias = "agent")]
    Agent,
}

// ---------------------------------------------------------------------------
// Part / PartContent
// ---------------------------------------------------------------------------

/// A single content unit in the A2A protocol.
///
/// Maps to the proto3 `Part` message. The content discriminator is
/// field-presence based (untagged serde).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Part {
    /// The content of this part.
    #[serde(flatten)]
    pub content: PartContent,

    /// Optional metadata key-value pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,

    /// Optional filename associated with this part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// Optional MIME type (e.g. `"image/png"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// Content of a single A2A part.
///
/// Matches the proto3 `oneof content` — exactly one field is populated.
/// Uses `#[serde(untagged)]` since A2A JSON uses field-presence discrimination.
///
/// **Variant order matters**: `Text` first (most common), then `Url`, then `Data`
/// last because `serde_json::Value` would match anything as a fallback.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PartContent {
    /// Plain text content.
    Text {
        /// The text string.
        text: String,
    },
    /// Raw binary data (base64-encoded in JSON).
    Raw {
        /// Base64-encoded binary data.
        raw: String,
    },
    /// URL pointing to content.
    Url {
        /// The URL.
        url: String,
    },
    /// Structured data (arbitrary JSON).
    Data {
        /// Arbitrary JSON data.
        data: serde_json::Value,
    },
}

impl Part {
    /// Create a text part.
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: PartContent::Text { text: s.into() },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }

    /// Create a raw binary part with a media type.
    pub fn raw(data: String, media_type: impl Into<String>) -> Self {
        Self {
            content: PartContent::Raw { raw: data },
            metadata: None,
            filename: None,
            media_type: Some(media_type.into()),
        }
    }

    /// Create a URL part with a media type.
    pub fn url(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            content: PartContent::Url { url: url.into() },
            metadata: None,
            filename: None,
            media_type: Some(media_type.into()),
        }
    }

    /// Create a structured data part.
    pub fn data(data: serde_json::Value) -> Self {
        Self {
            content: PartContent::Data { data },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// An A2A protocol message.
///
/// Maps to the proto3 `Message`. Prefixed with `A2a` to avoid collision with
/// `layer0::Content` / other message types.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aMessage {
    /// Unique message identifier (UUID v4).
    pub message_id: String,

    /// Context (conversation) this message belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// Task this message is associated with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,

    /// Sender role.
    pub role: A2aRole,

    /// Content parts.
    pub parts: Vec<Part>,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Protocol extension URIs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,

    /// IDs of tasks this message references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_task_ids: Vec<String>,
}

impl A2aMessage {
    /// Create a new message with a generated UUID.
    pub fn new(role: A2aRole, parts: Vec<Part>) -> Self {
        Self {
            message_id: Uuid::new_v4().to_string(),
            context_id: None,
            task_id: None,
            role,
            parts,
            metadata: None,
            extensions: Vec::new(),
            reference_task_ids: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TaskState / TaskStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of an A2A task.
///
/// Maps to the proto3 `TaskState` enum.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskState {
    /// Unspecified state (proto3 default, value 0).
    #[serde(rename = "TASK_STATE_UNSPECIFIED")]
    Unspecified,
    /// Task has been received but not yet started.
    #[serde(rename = "TASK_STATE_SUBMITTED", alias = "submitted")]
    Submitted,
    /// Task is actively being processed.
    #[serde(rename = "TASK_STATE_WORKING", alias = "working")]
    Working,
    /// Task finished successfully.
    #[serde(rename = "TASK_STATE_COMPLETED", alias = "completed")]
    Completed,
    /// Task failed.
    #[serde(rename = "TASK_STATE_FAILED", alias = "failed")]
    Failed,
    /// Task was canceled by the caller.
    #[serde(rename = "TASK_STATE_CANCELED", alias = "canceled")]
    Canceled,
    /// Agent needs more input from the caller.
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED", alias = "input_required")]
    InputRequired,
    /// Agent rejected the task.
    #[serde(rename = "TASK_STATE_REJECTED", alias = "rejected")]
    Rejected,
    /// Authentication is required before proceeding.
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED", alias = "auth_required")]
    AuthRequired,
}

impl TaskState {
    /// Returns `true` if this state is terminal (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::Rejected
        )
    }
}

/// Current status of an A2A task, combining state with optional context.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    /// The lifecycle state.
    pub state: TaskState,

    /// Optional message providing details about the status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<A2aMessage>,

    /// ISO 8601 timestamp of this status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl TaskStatus {
    /// Create a status with just a state and no message or timestamp.
    pub fn new(state: TaskState) -> Self {
        Self {
            state,
            message: None,
            timestamp: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

/// An artifact produced by an A2A task.
///
/// Prefixed with `A2a` to avoid collision with `layer0::dispatch::Artifact`.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aArtifact {
    /// Unique artifact identifier.
    pub artifact_id: String,

    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Content parts of this artifact.
    pub parts: Vec<Part>,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Protocol extension URIs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

impl A2aArtifact {
    /// Create a new artifact with a generated UUID.
    pub fn new(parts: Vec<Part>) -> Self {
        Self {
            artifact_id: Uuid::new_v4().to_string(),
            name: None,
            description: None,
            parts,
            metadata: None,
            extensions: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// A top-level A2A task.
///
/// Prefixed with `A2a` to avoid collision with other `Task` types.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aTask {
    /// Unique task identifier (UUID v4).
    pub id: String,

    /// Context (conversation) this task belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// Current task status.
    pub status: TaskStatus,

    /// Artifacts produced so far.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<A2aArtifact>,

    /// Message history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<A2aMessage>,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl A2aTask {
    /// Create a new task with a generated UUID.
    pub fn new(status: TaskStatus) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            context_id: None,
            status,
            artifacts: Vec::new(),
            history: Vec::new(),
            metadata: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming events
// ---------------------------------------------------------------------------

/// Server-sent event: task status changed.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusUpdateEvent {
    /// The task this event belongs to.
    pub task_id: String,

    /// Context (conversation) of the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// The new status.
    pub status: TaskStatus,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Server-sent event: artifact update (new or appended).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifactUpdateEvent {
    /// The task this event belongs to.
    pub task_id: String,

    /// Context (conversation) of the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// The artifact (or artifact chunk).
    pub artifact: A2aArtifact,

    /// If `true`, append parts to an existing artifact with the same ID.
    #[serde(default)]
    pub append: bool,

    /// If `true`, this is the last chunk for this artifact.
    #[serde(default)]
    pub last_chunk: bool,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A single frame in an A2A streaming response.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamResponse {
    /// Full task snapshot.
    #[serde(rename = "task")]
    Task {
        /// The task.
        task: A2aTask,
    },
    /// A standalone message (not tied to a task lifecycle update).
    #[serde(rename = "message")]
    Message {
        /// The message.
        message: A2aMessage,
    },
    /// Task status changed.
    #[serde(rename = "status_update")]
    StatusUpdate {
        /// The status update event.
        #[serde(flatten)]
        event: TaskStatusUpdateEvent,
    },
    /// Artifact produced or appended.
    #[serde(rename = "artifact_update")]
    ArtifactUpdate {
        /// The artifact update event.
        #[serde(flatten)]
        event: TaskArtifactUpdateEvent,
    },
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Configuration for a `SendMessage` request.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessageConfiguration {
    /// Output modes the caller can accept (e.g. `["text", "image"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_output_modes: Vec<String>,

    /// Push notification configuration for this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_push_notification_config: Option<TaskPushNotificationConfig>,

    /// Maximum number of history messages to include in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,

    /// If `true`, the server should return immediately with the task ID
    /// rather than waiting for completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_immediately: Option<bool>,
}

/// Request to send a message and create/continue a task.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    /// Tenant identifier (multi-tenant deployments).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// The message to send.
    pub message: A2aMessage,

    /// Optional configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response to a `SendMessage` request (non-streaming).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SendMessageResponse {
    /// The server created or updated a task.
    #[serde(rename = "task")]
    Task {
        /// The task.
        task: A2aTask,
    },
    /// The server replied with a message (no task lifecycle).
    #[serde(rename = "message")]
    Message {
        /// The reply message.
        message: A2aMessage,
    },
}

/// Request to retrieve an existing task.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskRequest {
    /// Tenant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Task ID to retrieve.
    pub id: String,

    /// Maximum number of history messages to include.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Request to cancel a running task.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelTaskRequest {
    /// Tenant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Task ID to cancel.
    pub id: String,

    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Request to subscribe to updates for an existing task.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeToTaskRequest {
    /// Tenant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Task ID to subscribe to.
    pub id: String,

    /// Maximum number of history messages to replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Request to list tasks, optionally filtered.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTasksRequest {
    /// Tenant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Context ID filter — only return tasks in this context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// Maximum number of tasks to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,

    /// Maximum number of history messages per task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Response to a `ListTasks` request.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksResponse {
    /// The matching tasks.
    pub tasks: Vec<A2aTask>,
}

impl ListTasksResponse {
    /// Create a new response with the given tasks.
    pub fn new(tasks: Vec<A2aTask>) -> Self {
        Self { tasks }
    }
}

// ---------------------------------------------------------------------------
// Push notifications / auth
// ---------------------------------------------------------------------------

/// Authentication information for push notification delivery.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationInfo {
    /// Authentication scheme (e.g. `"bearer"`).
    pub scheme: String,

    /// Credentials (e.g. a token string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// Configuration for push notifications on a task.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPushNotificationConfig {
    /// Tenant identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Config entry ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// The task to receive notifications for.
    pub task_id: String,

    /// Webhook URL to POST notifications to.
    pub url: String,

    /// Optional authentication token for the webhook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    /// Optional authentication info for the webhook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
}

// ---------------------------------------------------------------------------
// Agent Card types
// ---------------------------------------------------------------------------

/// Provider information for an agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    /// Provider's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Provider's organization name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

/// A skill (capability) advertised by an agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Unique skill identifier.
    pub id: String,

    /// Human-readable skill name.
    pub name: String,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Categorization tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Example prompts that trigger this skill.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,

    /// Input modes this skill accepts (e.g. `["text", "image"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,

    /// Output modes this skill produces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}

/// Network interface where an agent can be reached.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterface {
    /// Endpoint URL.
    pub url: String,

    /// Protocol binding (e.g. `"jsonrpc/http"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_binding: Option<String>,

    /// Tenant identifier for this interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// Protocol version supported at this interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
}

/// Capabilities an agent advertises.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,

    /// Whether the agent supports push notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,

    /// Supported protocol extensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,

    /// Whether the agent provides an extended agent card at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_agent_card: Option<bool>,
}

/// An A2A security scheme.
///
/// Uses a tagged enum to represent the `oneof` in the proto.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityScheme {
    /// API key authentication.
    #[serde(rename = "api_key")]
    ApiKey {
        /// Where to send the key (e.g. `"header"`, `"query"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#in: Option<String>,
        /// Parameter name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// HTTP authentication (e.g. Bearer, Basic).
    #[serde(rename = "http_auth")]
    HttpAuth {
        /// Auth scheme (e.g. `"bearer"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scheme: Option<String>,
        /// Bearer token format hint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_format: Option<String>,
    },
    /// OAuth 2.0.
    #[serde(rename = "oauth2")]
    OAuth2 {
        /// OAuth2 flows configuration (opaque JSON).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flows: Option<serde_json::Value>,
    },
    /// OpenID Connect.
    #[serde(rename = "openid_connect")]
    OpenIdConnect {
        /// OpenID Connect discovery URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        openid_connect_url: Option<String>,
    },
    /// Mutual TLS.
    #[serde(rename = "mtls")]
    Mtls,
}

/// A security requirement entry — maps scheme name to scopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    /// Scheme name (must match a key in the agent card's `security_schemes`).
    pub scheme: String,

    /// Required scopes for this scheme.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

/// An A2A Agent Card — the discovery document for an agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Agent name.
    pub name: String,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Network interfaces where the agent is reachable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_interfaces: Vec<AgentInterface>,

    /// Provider metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,

    /// Agent version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// URL to agent documentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,

    /// Agent capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AgentCapabilities>,

    /// Security schemes the agent supports, keyed by scheme name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_schemes: Option<serde_json::Map<String, serde_json::Value>>,

    /// Security requirements — each entry is an AND of schemes, entries are OR'd.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_requirements: Vec<SecurityRequirement>,

    /// Default input modes (e.g. `["text"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_input_modes: Vec<String>,

    /// Default output modes (e.g. `["text"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_output_modes: Vec<String>,

    /// Skills this agent provides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<AgentSkill>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_text_round_trip() {
        let part = Part::text("hello");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""text":"hello""#), "got: {json}");
        let back: Part = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn part_url_round_trip() {
        let part = Part::url("https://example.com/img.png", "image/png");
        let json = serde_json::to_string(&part).unwrap();
        assert!(
            json.contains(r#""url":"https://example.com/img.png""#),
            "got: {json}"
        );
        assert!(json.contains(r#""media_type":"image/png""#), "got: {json}");
        let back: Part = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn part_data_round_trip() {
        let part = Part::data(serde_json::json!({"key": "value"}));
        let json = serde_json::to_string(&part).unwrap();
        let back: Part = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn role_serialization() {
        assert_eq!(
            serde_json::to_string(&A2aRole::User).unwrap(),
            r#""ROLE_USER""#
        );
        assert_eq!(
            serde_json::to_string(&A2aRole::Agent).unwrap(),
            r#""ROLE_AGENT""#
        );
        // proto3 canonical deserializes
        let back: A2aRole = serde_json::from_str(r#""ROLE_AGENT""#).unwrap();
        assert_eq!(back, A2aRole::Agent);
        // old snake_case still deserializes (backward compat)
        let back: A2aRole = serde_json::from_str(r#""agent""#).unwrap();
        assert_eq!(back, A2aRole::Agent);
        let back: A2aRole = serde_json::from_str(r#""user""#).unwrap();
        assert_eq!(back, A2aRole::User);
    }

    #[test]
    fn task_state_terminal() {
        assert!(!TaskState::Unspecified.is_terminal());
        assert!(!TaskState::Submitted.is_terminal());
        assert!(!TaskState::Working.is_terminal());
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(!TaskState::InputRequired.is_terminal());
        assert!(TaskState::Rejected.is_terminal());
        assert!(!TaskState::AuthRequired.is_terminal());
    }

    #[test]
    fn task_state_round_trip() {
        for (state, expected_json) in [
            (TaskState::Unspecified, r#""TASK_STATE_UNSPECIFIED""#),
            (TaskState::Submitted, r#""TASK_STATE_SUBMITTED""#),
            (TaskState::Working, r#""TASK_STATE_WORKING""#),
            (TaskState::Completed, r#""TASK_STATE_COMPLETED""#),
            (TaskState::Failed, r#""TASK_STATE_FAILED""#),
            (TaskState::Canceled, r#""TASK_STATE_CANCELED""#),
            (TaskState::InputRequired, r#""TASK_STATE_INPUT_REQUIRED""#),
            (TaskState::Rejected, r#""TASK_STATE_REJECTED""#),
            (TaskState::AuthRequired, r#""TASK_STATE_AUTH_REQUIRED""#),
        ] {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, expected_json, "serialization mismatch for {state:?}");
            let back: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state, "round-trip failed for {json}");
        }
    }

    #[test]
    fn task_state_backward_compat() {
        // Old snake_case values must still deserialize
        let back: TaskState = serde_json::from_str(r#""submitted""#).unwrap();
        assert_eq!(back, TaskState::Submitted);
        let back: TaskState = serde_json::from_str(r#""working""#).unwrap();
        assert_eq!(back, TaskState::Working);
        let back: TaskState = serde_json::from_str(r#""input_required""#).unwrap();
        assert_eq!(back, TaskState::InputRequired);
        let back: TaskState = serde_json::from_str(r#""auth_required""#).unwrap();
        assert_eq!(back, TaskState::AuthRequired);
    }

    #[test]
    fn part_raw_round_trip() {
        let part = Part::raw("SGVsbG8gV29ybGQ=".into(), "application/octet-stream");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""raw":"SGVsbG8gV29ybGQ=""#), "got: {json}");
        let back: Part = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn message_new_generates_uuid() {
        let msg = A2aMessage::new(A2aRole::User, vec![Part::text("hi")]);
        assert!(!msg.message_id.is_empty());
        // Should be a valid UUID v4
        uuid::Uuid::parse_str(&msg.message_id).expect("message_id should be valid UUID");
    }

    #[test]
    fn task_round_trip() {
        let task = A2aTask::new(TaskStatus::new(TaskState::Working));
        let json = serde_json::to_string(&task).unwrap();
        let back: A2aTask = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, task.id);
        assert_eq!(back.status.state, TaskState::Working);
    }

    #[test]
    fn send_message_response_task_variant() {
        let task = A2aTask::new(TaskStatus::new(TaskState::Submitted));
        let resp = SendMessageResponse::Task { task };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""type":"task""#), "got: {json}");
        let back: SendMessageResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, SendMessageResponse::Task { .. }));
    }

    #[test]
    fn send_message_response_message_variant() {
        let msg = A2aMessage::new(A2aRole::Agent, vec![Part::text("done")]);
        let resp = SendMessageResponse::Message { message: msg };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""type":"message""#), "got: {json}");
    }

    #[test]
    fn optional_fields_omitted() {
        let part = Part::text("hello");
        let json = serde_json::to_string(&part).unwrap();
        assert!(
            !json.contains("metadata"),
            "metadata should be omitted: {json}"
        );
        assert!(
            !json.contains("filename"),
            "filename should be omitted: {json}"
        );
        assert!(
            !json.contains("media_type"),
            "media_type should be omitted: {json}"
        );
    }

    #[test]
    fn artifact_round_trip() {
        let artifact = A2aArtifact::new(vec![Part::text("result")]);
        let json = serde_json::to_string(&artifact).unwrap();
        let back: A2aArtifact = serde_json::from_str(&json).unwrap();
        assert_eq!(back.artifact_id, artifact.artifact_id);
        assert_eq!(back.parts.len(), 1);
    }

    #[test]
    fn stream_response_status_update() {
        let event = TaskStatusUpdateEvent {
            task_id: "t1".into(),
            context_id: None,
            status: TaskStatus::new(TaskState::Working),
            metadata: None,
        };
        let resp = StreamResponse::StatusUpdate { event };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""type":"status_update""#), "got: {json}");
        let back: StreamResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, StreamResponse::StatusUpdate { .. }));
    }

    #[test]
    fn security_scheme_round_trip() {
        let scheme = SecurityScheme::ApiKey {
            r#in: Some("header".into()),
            name: Some("X-Api-Key".into()),
        };
        let json = serde_json::to_string(&scheme).unwrap();
        assert!(json.contains(r#""type":"api_key""#), "got: {json}");
        let back: SecurityScheme = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, SecurityScheme::ApiKey { .. }));
    }
}
