//! Reference rule implementations.
//!
//! These are policy, not primitives. They ship with the reference runtime
//! and will move when the runtime is decomposed.

pub mod budget;

pub use budget::{BudgetGuard, BudgetGuardConfig};
