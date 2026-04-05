#![deny(missing_docs)]
//! # skg-context-engine
//!
//! Mutable context substrate and reference react loop for skelegent agents.
//!
//! ## Primitives (under redesign)
//!
//! - [`Context`] — mutable message buffer with extensions, metrics, rules,
//!   observation stream, and intervention channel.
//! - [`ContextOp`] — universal operation trait (`async fn execute(&self, &mut Context)`).
//! - [`Rule`] / [`Trigger`] — reactive hooks that fire at typed boundaries.
//! - [`InferBoundary`] / [`StreamInferBoundary`] — marker types for pre-send rules.
//! - [`Context::compile()`] → [`CompiledContext`] — snapshot to inference request.
//!
//! ## Reference Runtime (to be decomposed)
//!
//! - [`CognitiveOperator`] / [`CognitiveBuilder`] — reference operator wrapping
//!   the react loop. These are policy, not primitives, and will move to a
//!   reference/recipes layer.
//! - [`react_loop()`] / [`stream_react_loop()`] — standard ReAct execution.
//!
//! ## Deleted (moved to implementer responsibility)
//!
//! Budget guards, telemetry rules, state↔context bridging ops, and compaction
//! strategies have been removed from the engine. They are replaceable policies
//! per v2 spec.

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

// Re-exports: primitives
pub use boundary::{InferBoundary, StreamInferBoundary};
pub use compile::{CompileConfig, CompiledContext, InferResult};
pub use context::{Context, Extensions, TurnMetrics};
pub use error::EngineError;
pub use op::{ContextOp, ErasedOp};
pub use ops::*;
pub use output::{OutputError, OutputMode, OutputSchema, extract_json_block};
pub use rule::{Rule, Trigger};
pub use stream::{ContextEvent, ContextMutation};

// Re-exports: reference runtime (to be moved)
pub use builder::{CognitiveBuilder, NoProvider, WithProvider};
pub use cognitive_operator::{CognitiveOperator, map_engine_error};
pub use react::{
    ReactLoopConfig, ToolFilter, check_approval, check_exit, format_tool_error, react_loop,
    react_loop_structured,
};
pub use stream_react::stream_react_loop;
