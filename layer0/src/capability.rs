//! Capability discovery nouns for the v2 kernel track.
//!
//! This module defines the canonical read-only discovery surface:
//! [`CapabilitySource`] returns [`CapabilityDescriptor`] values, optionally
//! filtered by [`CapabilityFilter`].

use crate::error::ProtocolError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Stable identifier for a discoverable capability.
#[non_exhaustive]
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityId(pub String);

impl CapabilityId {
    /// Create a capability ID from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the inner identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CapabilityId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CapabilityId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Top-level semantic kind of a discoverable capability.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// Executable tool capability.
    Tool,
    /// Prompt template capability.
    Prompt,
    /// Readable resource capability.
    Resource,
    /// Agent capability.
    Agent,
    /// Service capability.
    Service,
}

/// Content modality accepted or produced by a capability.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityModality {
    /// Plain text payload.
    Text,
    /// JSON payload.
    Json,
    /// Binary payload.
    Binary,
    /// Structured but non-JSON domain payload.
    Structured,
    /// Extension modality not yet standardized by the kernel.
    Custom(String),
}

/// Streaming support facts for a capability.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingSupport {
    /// No streaming support.
    None,
    /// Supports streamed output.
    Output,
    /// Supports bi-directional streaming.
    Bidirectional,
}

/// Scheduling class fact for planner consumption.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    /// Can share execution windows with other shared capabilities.
    Shared,
    /// Acts as an execution barrier.
    Exclusive,
}

/// Serializable scheduling facts for planning.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulingFacts {
    /// Shared/exclusive execution class.
    pub execution_class: ExecutionClass,
    /// Whether execution order matters for this capability.
    pub ordering_sensitive: bool,
    /// Whether repeated identical invocations are safe.
    pub idempotent: bool,
    /// Whether active execution can be interrupted safely.
    pub interruptible: bool,
    /// Optional bounded concurrency hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

impl SchedulingFacts {
    /// Create scheduling facts.
    pub fn new(
        execution_class: ExecutionClass,
        ordering_sensitive: bool,
        idempotent: bool,
        interruptible: bool,
        max_concurrency: Option<u32>,
    ) -> Self {
        Self {
            execution_class,
            ordering_sensitive,
            idempotent,
            interruptible,
            max_concurrency,
        }
    }
}

/// Serializable approval requirements.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalFacts {
    /// No approval requirement.
    None,
    /// Always requires approval.
    Always,
    /// Runtime policy decides based on invocation context/input.
    RuntimePolicy,
}

/// Serializable auth/access facts.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AuthFacts {
    /// Open access.
    Open,
    /// Uses caller identity.
    Caller,
    /// Uses service identity with optional scopes.
    Service {
        /// Service scopes required by this capability.
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Custom auth scheme.
    Custom {
        /// Custom scheme identifier.
        scheme: String,
    },
}

/// Canonical discovery payload for all capability kinds.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// Stable capability identifier.
    pub id: CapabilityId,
    /// Semantic capability kind.
    pub kind: CapabilityKind,
    /// Human-readable capability name.
    pub name: String,
    /// Human-readable capability description.
    pub description: String,
    /// Optional input schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Optional output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Accepted modalities.
    #[serde(default)]
    pub accepts: Vec<CapabilityModality>,
    /// Produced modalities.
    #[serde(default)]
    pub produces: Vec<CapabilityModality>,
    /// Streaming support facts.
    pub streaming: StreamingSupport,
    /// Scheduling facts.
    pub scheduling: SchedulingFacts,
    /// Approval facts.
    pub approval: ApprovalFacts,
    /// Auth facts.
    pub auth: AuthFacts,
    /// Optional tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Extension bag for bridge/protocol-specific extras.
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

impl CapabilityDescriptor {
    /// Create a descriptor with canonical defaults for optional fields.
    pub fn new(
        id: impl Into<CapabilityId>,
        kind: CapabilityKind,
        name: impl Into<String>,
        description: impl Into<String>,
        scheduling: SchedulingFacts,
        approval: ApprovalFacts,
        auth: AuthFacts,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            name: name.into(),
            description: description.into(),
            input_schema: None,
            output_schema: None,
            accepts: Vec::new(),
            produces: Vec::new(),
            streaming: StreamingSupport::None,
            scheduling,
            approval,
            auth,
            tags: Vec::new(),
            extensions: Map::new(),
        }
    }

    /// Returns true when this descriptor satisfies `filter`.
    pub fn matches_filter(&self, filter: &CapabilityFilter) -> bool {
        if !filter.kinds.is_empty() && !filter.kinds.iter().any(|kind| kind == &self.kind) {
            return false;
        }

        if let Some(name_contains) = filter.name_contains.as_ref()
            && !self
                .name
                .to_ascii_lowercase()
                .contains(&name_contains.to_ascii_lowercase())
        {
            return false;
        }

        if let Some(requires_streaming) = filter.requires_streaming {
            let is_streaming = !matches!(self.streaming, StreamingSupport::None);
            if is_streaming != requires_streaming {
                return false;
            }
        }

        if let Some(requires_approval) = filter.requires_approval {
            let has_approval = !matches!(self.approval, ApprovalFacts::None);
            if has_approval != requires_approval {
                return false;
            }
        }

        if !filter.tags.is_empty()
            && !filter
                .tags
                .iter()
                .all(|tag| self.tags.iter().any(|candidate| candidate == tag))
        {
            return false;
        }

        true
    }
}

/// Read-only filter for capability discovery.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityFilter {
    /// Restrict results to these kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<CapabilityKind>,
    /// Case-insensitive name substring requirement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
    /// Require streaming (`true`) or non-streaming (`false`) capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_streaming: Option<bool>,
    /// Require approval (`true`) or no approval (`false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    /// Result must contain all listed tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Read-only discovery source for capability descriptors.
#[async_trait]
pub trait CapabilitySource: Send + Sync {
    /// List capability descriptors matching `filter`.
    async fn list(
        &self,
        filter: CapabilityFilter,
    ) -> Result<Vec<CapabilityDescriptor>, ProtocolError>;

    /// Fetch one descriptor by capability ID.
    async fn get(&self, id: &CapabilityId) -> Result<Option<CapabilityDescriptor>, ProtocolError>;
}
