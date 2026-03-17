#![deny(missing_docs)]
//! # skg-context-engine
//!
//! Composable context engine for skelegent agents.
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
//! [`Context::compile()`] produces a [`CompiledContext`]. The actual provider call
//! runs behind typed governance markers like [`InferBoundary`] and
//! [`StreamInferBoundary`], so rules and interventions can target the real
//! pre-inference boundary. The response is NOT automatically appended — that's a
//! separate [`AppendResponse`] context op.
//!
//! ## The ReAct Pattern
//!
//! [`react_loop()`] composes these primitives into a standard ReAct loop in ~50
//! lines. [`react_loop_structured()`] extends this with validated structured output.
//! Both are functions, not frameworks.
//! ## Rules — Reactive Participants
//!
//! Rules are [`ContextOp`]s with [`Trigger`]s. They fire automatically during
//! `Context::run()` and have the same `&mut Context` power as pipeline ops.
//! Budget guards, overwatch agents, telemetry recorders — all are just rules.
//!
//! ## Integration: Using Context as Your Conversation Store
//!
//! [`Context`] is the conversation store for any system that talks to an LLM.
//! Your application's domain data (shell history, file state, user prefs) feeds
//! INTO context — it doesn't replace it.
//!
//! **How domain context gets in:**
//! - [`ReactLoopConfig::system_prompt`] — static instructions baked into every turn
//! - [`Context::inject_system()`] / [`Context::inject_message()`] — programmatic injection
//! - [`Extensions`] — typed state (`HashMap<TypeId, Box<dyn Any>>`) accessible to rules
//!
//! **Wrapping `react_loop` as an [`Operator`](layer0::Operator):**
//!
//! To expose context-engine behind the object-safe `Operator` boundary, create a
//! struct that owns the provider, tools, and config, then delegates to [`react_loop()`]:
//!
//! ```rust,ignore
//! struct MyOperator<P: Provider> {
//!     provider: P,
//!     tools: ToolRegistry,
//!     operator_id: OperatorId,
//!     config: ReactLoopConfig,
//! }
//!
//! #[async_trait]
//! impl<P: Provider> Operator for MyOperator<P> {
//!     async fn execute(&self, input: OperatorInput, _emitter: &EffectEmitter) -> Result<OperatorOutput, OperatorError> {
//!         let mut ctx = Context::new();
//!         ctx.inject_message(Message::new(Role::User, input.message))
//!             .await
//!             .map_err(OperatorError::context_assembly)?;
//!         let dispatch_ctx = DispatchContext::new(DispatchId::new("my-op"), self.operator_id.clone());
//!         react_loop(&mut ctx, &self.provider, &self.tools, &dispatch_ctx, &self.config)
//!             .await
//!             .map_err(|e| OperatorError::non_retryable(e.to_string()))
//!     }
//! }
//! ```

pub mod assembly;
pub mod boundary;
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
pub use cognitive_operator::{CognitiveOperator, CognitiveOperatorConfig, map_engine_error};
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
