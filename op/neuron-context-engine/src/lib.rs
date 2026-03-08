#![deny(missing_docs)]
//! # neuron-context-engine
//!
//! Composable context engine for neuron agents.
//!
//! Agentic programming has three pillars: **context**, **inference**, and
//! **infrastructure**. This crate makes context a first-class primitive with
//! universal hookability.
//!
//! ## The Three Phases
//!
//! Every agent follows three phases:
//!
//! 1. **Assembly** — build the context the model will see
//! 2. **Inference** — call the model (the irreducible network boundary)
//! 3. **Reaction** — branch on the response, dispatch tools, push effects
//!
//! ## Core Primitives
//!
//! - [`Context`] — the mutable substrate. Carries messages, extensions, metrics, effects, rules.
//! - [`ContextOp`] — the universal operation primitive. Everything implements this.
//! - [`Rule`] — a reactive participant. Same power as pipeline ops, different activation.
//! - Fluent methods on `Context` (`.inject_system()`, `.inject_message()`, etc.) dispatch through `Context::run()`.
//!
//! ## The Phase Boundary
//!
//! [`Context::compile()`] produces a [`CompiledContext`]. [`CompiledContext::infer()`]
//! crosses the network boundary. The response is NOT automatically appended —
//! that's a separate [`AppendResponse`] context op.
//!
//! ## The ReAct Pattern
//!
//! [`react_loop()`] composes these primitives into a standard ReAct loop in ~50
//! lines. It is a function, not a framework.
//!
//! ## Rules — Reactive Participants
//!
//! Rules are [`ContextOp`]s with [`Trigger`]s. They fire automatically during
//! `Context::run()` and have the same `&mut Context` power as pipeline ops.
//! Budget guards, overwatch agents, telemetry recorders — all are just rules.

pub mod assembly;
pub mod compile;
pub mod context;
pub mod error;
pub mod op;
pub mod ops;
pub mod react;
pub mod rule;
pub mod rules;

// Re-exports
pub use compile::{CompileConfig, CompiledContext, InferResult};
pub use context::{Context, Extensions, TurnMetrics};
pub use error::EngineError;
pub use op::ContextOp;
pub use ops::*;
pub use react::{ReactLoopConfig, react_loop};
pub use rule::{Rule, Trigger};
pub use rules::*;
