#![deny(missing_docs)]
//! Portable durable run/control primitives for Skelegent.
//! 
//! This crate defines the backend-agnostic public nouns and control traits for
//! durable orchestration above Layer 0. It intentionally does not standardize
//! checkpoint payloads, replay history, worker leasing, or storage layout.

pub mod control;
pub mod id;
pub mod model;
pub mod wait;

pub use control::{RunControlError, RunController, RunStarter};
pub use id::{RunId, WaitPointId};
pub use model::{RunOutcome, RunStatus, RunView};
pub use wait::{ResumeInput, WaitReason};
