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

mod kit;
mod runner;

pub use kit::Kit;
pub use runner::{ExecutionEvent, ExecutionTrace, KitError, OrchestratedRunner};

pub mod effects;
pub use skg_effects_core as effects_core;
pub use skg_effects_local as effects_local;
