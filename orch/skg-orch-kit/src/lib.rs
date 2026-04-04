#![deny(missing_docs)]
//! Unopinionated wiring kit for composing runnable Skelegent systems.
//!
//! This crate is intentionally "boring glue": it helps assemble and run
//! systems built from the `layer0` protocols without forcing a workflow DSL.
//!
//! Design goals (see `specs/06-composition-factory-and-glue.md`):
//! - register arbitrary agents/operators
//! - swap implementations via explicit selectors
//! - pluggable effect execution policy (WriteMemory/Delegate/Handoff/Signal)
//! - zero lock-in: callers can bypass defaults

#[cfg(test)]
mod budget;
mod compaction;
mod intervene;
mod kit;
mod observe;
mod runner;

pub use compaction::{
    CompactContext, CompactionCoordinationError, CompactionCoordinator, CompactionDecision,
    CompactionOperationError, CompactionOutcome, CompactionSnapshot, FlushBeforeCompact,
};
pub use intervene::{ContextIntervenor, InterventionSendError};
pub use kit::Kit;
pub use observe::{ContextObserver, Observation, ObservationBatch, ObservationTry};
pub use runner::{
    EffectAction, EffectMiddleware, EffectStack, ExecutionEvent, ExecutionTrace, KitError,
    OrchestratedRunner,
};

pub mod effects;
pub use skg_context_engine as context_engine;
pub use skg_effects_core as effects_core;
pub use skg_effects_local as effects_local;
