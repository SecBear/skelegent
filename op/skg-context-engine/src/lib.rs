#![deny(missing_docs)]
//! # skg-context-engine
//!
//! Composable context engine for skelegent agents.
//!
//! The context engine provides a programmable surface for context assembly
//! right before inference. Everything is a [`ContextOp`] operating on a
//! mutable [`Context`]. Rules fire at typed boundaries (e.g.,
//! [`InferBoundary`]) giving implementers full control over what the model
//! sees.
//!
//! ## Core Primitives
//!
//! - [`Context`] — the mutable substrate. Carries messages, extensions,
//!   metrics, intents, rules, observation stream, intervention channel.
//! - [`ContextOp`] — the universal operation primitive.
//! - [`Rule`] / [`Trigger`] — reactive participants that fire at typed
//!   boundaries or on predicates.
//! - [`InferBoundary`] / [`StreamInferBoundary`] — marker types that make
//!   the pre-send interception point targetable by rules.
//! - [`Context::compile()`] → [`CompiledContext`] — snapshot context into
//!   an [`InferRequest`](skg_turn::infer::InferRequest).
//! - [`Context::run(op)`](Context::run) — execute an op, firing rules
//!   before and after.
//!
//! ## Reference Runtime
//!
//! [`react_loop()`] and [`stream_react_loop()`] compose primitives into a
//! standard ReAct loop. [`CognitiveOperator`] wraps the react loop behind
//! the [`Operator`](layer0::Operator) trait.

pub mod boundary;
pub mod builder;
pub mod cognitive_operator;
pub mod compile;
pub mod context;
pub mod error;
pub mod op;
pub mod ops;
pub mod output;
pub mod react;
pub mod rule;
pub mod rules;
pub mod stream;
pub mod stream_react;

// Re-exports
pub use boundary::{InferBoundary, StreamInferBoundary};
pub use builder::{CognitiveBuilder, NoProvider, WithProvider};
pub use cognitive_operator::{CognitiveOperator, map_engine_error};
pub use compile::{CompileConfig, CompiledContext, InferResult};
pub use context::{Context, Extensions, TurnMetrics};
pub use error::EngineError;
pub use op::{ContextOp, ErasedOp};
pub use ops::*;
pub use output::{OutputError, OutputMode, OutputSchema, extract_json_block};
pub use react::{
    ReactLoopConfig, ToolFilter, check_approval, check_exit, format_tool_error, react_loop,
    react_loop_structured,
};
pub use rule::{Rule, Trigger};
pub use rules::*;
pub use stream::{ContextEvent, ContextMutation};
pub use stream_react::stream_react_loop;
