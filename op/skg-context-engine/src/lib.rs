#![deny(missing_docs)]
//! # skg-context-engine
//!
//! Mutable context substrate and reference runtime for skelegent agents.
//!
//! ## Primitives
//!
//! - [`Context`] — mutable message buffer with extensions, metrics, intents.
//! - [`Middleware`] / [`ErasedMiddleware`] — the single abstraction for context
//!   transformation. Named structs for reusable middleware, async closures for
//!   one-offs.
//! - [`Pipeline`] — ordered before-send / after-send middleware phases.
//! - [`Context::compile()`] → [`CompiledContext`] — snapshot to inference request.
//!
//! ## Reference Runtime
//!
//! - [`AgentOperator`] / [`AgentBuilder`] — reference operator + construction surface
//!   wrapping the runtime loops.
//! - [`react_loop()`] / [`stream_react_loop()`] — standard collected and streaming runtime entrypoints.

pub mod agent_operator;
pub mod builder;
pub mod compile;
pub mod context;
pub mod error;
pub mod middleware;
pub mod output;
pub mod pipeline;
pub mod runtime;
pub mod stream_runtime;

// Re-exports: primitives
pub use compile::{CompileConfig, CompiledContext, InferResult};
pub use context::{Context, Extensions, TurnMetrics};
pub use error::EngineError;
pub use middleware::{ErasedMiddleware, Middleware, MiddlewareFn, middleware_fn};
pub use output::{OutputError, OutputMode, OutputSchema, extract_json_block};
pub use pipeline::Pipeline;

// Re-exports: reference runtime
pub use agent_operator::{AgentOperator, map_engine_error};
pub use builder::{AgentBuilder, NoProvider, WithProvider};
pub use runtime::{
    ReactLoopConfig, ToolFilter, check_approval, check_exit, format_tool_error, react_loop,
    react_loop_structured,
};
pub use stream_runtime::stream_react_loop;
