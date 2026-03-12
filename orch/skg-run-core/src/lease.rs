//! Optional worker-claim seams for multi-worker durable backends.

use crate::deadline::PortableWakeDeadline;
use crate::id::RunId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Request to claim exclusive processing rights for a durable run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseClaim {
    /// Run being claimed.
    pub run_id: RunId,
    /// Backend-defined worker or processor identifier.
    pub holder: String,
    /// Deadline after which the claim expires if not renewed.
    pub lease_until: PortableWakeDeadline,
}

impl LeaseClaim {
    /// Create a new durable lease claim request.
    pub fn new(
        run_id: RunId,
        holder: impl Into<String>,
        lease_until: PortableWakeDeadline,
    ) -> Self {
        Self {
            run_id,
            holder: holder.into(),
            lease_until,
        }
    }
}

/// Granted lease proving exclusive processing rights for a durable run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseGrant {
    /// Run currently claimed.
    pub run_id: RunId,
    /// Backend-defined worker or processor identifier.
    pub holder: String,
    /// Deadline after which the claim expires if not renewed.
    pub lease_until: PortableWakeDeadline,
    /// Opaque backend lease identifier.
    pub lease_id: String,
}

impl LeaseGrant {
    /// Create a new granted lease.
    pub fn new(
        run_id: RunId,
        holder: impl Into<String>,
        lease_until: PortableWakeDeadline,
        lease_id: impl Into<String>,
    ) -> Self {
        Self {
            run_id,
            holder: holder.into(),
            lease_until,
            lease_id: lease_id.into(),
        }
    }
}

/// Portable error surfaced by durable lease operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseError {
    /// The referenced lease no longer exists for renewal or release.
    #[error("lease not found for run {run_id}")]
    LeaseNotFound {
        /// Run whose lease was missing.
        run_id: RunId,
    },
    /// The operation conflicts with the current lease state.
    #[error("lease conflict: {0}")]
    Conflict(String),
    /// Backend-specific failure surfaced through the portable seam.
    #[error("lease backend error: {0}")]
    Backend(String),
}

/// Optional lease store for multi-worker durable execution.
#[async_trait]
pub trait LeaseStore: Send + Sync {
    /// Try to acquire exclusive processing rights for a run.
    async fn try_acquire_lease(&self, claim: LeaseClaim) -> Result<Option<LeaseGrant>, LeaseError>;

    /// Renew an existing lease until a later deadline.
    ///
    /// Returns [`LeaseError::LeaseNotFound`] when the referenced lease no longer exists.
    async fn renew_lease(
        &self,
        grant: &LeaseGrant,
        lease_until: PortableWakeDeadline,
    ) -> Result<LeaseGrant, LeaseError>;

    /// Release a previously granted lease.
    async fn release_lease(&self, grant: &LeaseGrant) -> Result<(), LeaseError>;
}
