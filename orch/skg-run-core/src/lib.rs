#![deny(missing_docs)]
//! Portable durable run/control primitives for Skelegent.
//!
//! This crate defines the backend-agnostic public nouns and control traits for
//! durable orchestration above Layer 0. It intentionally keeps lower-level store,
//! timer, lease, and driver seams optional and does not standardize checkpoint
//! payloads, replay history, worker leasing policy, or storage layout.

pub mod checkpoint;
pub mod command;
pub mod control;
pub mod deadline;
pub mod driver;
pub mod id;
pub mod kernel;
pub mod lease;
pub mod model;
pub mod observe;
pub mod store;
pub mod timer;
pub mod wait;

pub use checkpoint::{Checkpoint, CheckpointError, CheckpointStore};
pub use command::{DispatchPayload, OrchestrationCommand};
pub use control::{RunControlError, RunController, RunStarter};
pub use deadline::{PortableWakeDeadline, WakeDeadlineError};
pub use driver::{DriverError, DriverRequest, DriverResponse, RunDriver};
pub use id::{CheckpointId, RunId, WaitPointId};
pub use kernel::{KernelError, ResumeAction, RunEvent, RunKernel, RunTransition};
pub use lease::{LeaseClaim, LeaseError, LeaseGrant, LeaseStore};
pub use model::{RunOutcome, RunStatus, RunView};
pub use observe::{RunObserver, RunSubscription, RunUpdate};
pub use store::{
    BackendRunRef, PendingResume, PendingSignal, RunStore, RunStoreError, StoreRunRecord,
    WaitStore, WaitStoreError,
};
pub use timer::{ScheduledTimer, TimerStore, TimerStoreError};
pub use wait::{ResumeInput, WaitReason};
