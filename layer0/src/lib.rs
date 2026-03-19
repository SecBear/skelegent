//! # layer0 — Protocol traits for composable agentic AI systems
//!
//! This crate defines the four protocol boundaries plus two cross-cutting
//! protocol surfaces that compose to form any agentic AI system.
//!
//! ## The Protocols
//!
//! | Protocol | Trait | What it does |
//! |----------|-------|-------------|
//! | ① Operator | [`Operator`] | What one operator does per cycle |
//! | ② Dispatch | [`Dispatcher`] | The single invocation primitive |
//! | ③ State | [`StateStore`] | How data persists across turns |
//! | ④ Environment | [`Environment`] | Isolation, credentials, resources |
//!
//! ## Cross-Cutting Protocol Surface
//!
//! | Surface | Types | What it does |
//! |---------|-------|-------------|
//! | ⑤ Middleware | [`DispatchMiddleware`], [`StoreMiddleware`], [`ExecMiddleware`] | Interception + policy |
//! | ⑥ Message compaction metadata | [`CompactionPolicy`] | Advisory retention hints attached to messages |
//!
//! ## Design Principle
//!
//! Every protocol trait is operation-defined, not mechanism-defined.
//! [`Operator::execute`] means "cause this agent to process one cycle" —
//! not "make an API call" or "run a subprocess." This is what makes
//! implementations swappable: a Temporal workflow, a function call,
//! and a future system that doesn't exist yet all implement the same trait.
//!
//! ## Companion Documents
//!
//! - See `ARCHITECTURE.md` for design rationale
//!
//! ## Dependency Notes
//!
//! This crate depends on `serde_json::Value` for extension data fields
//! (metadata, tool inputs, custom payloads). This is an intentional choice:
//! JSON is the universal interchange format for agentic systems, and
//! `serde_json::Value` is the de facto standard in the Rust ecosystem.
//! The alternative (generic `T: Serialize`) would complicate trait object
//! safety without practical benefit.
//!
//! ## Future: Native Async Traits
//!
//! Protocol traits currently use `async-trait` (heap-allocated futures).
//! When Rust stabilizes `async fn in dyn Trait` with `Send` bounds,
//! these traits will migrate to native async. This will be a breaking
//! change in a minor version bump before v1.0.

#![deny(missing_docs)]

pub mod approval;
pub mod content;
pub mod context;
pub mod dispatch;
pub mod dispatch_context;
pub mod duration;
pub mod effect;
pub mod environment;
pub mod error;
pub mod id;
pub mod lifecycle;
pub mod middleware;
pub mod operator;
pub mod secret;
pub mod reducer;
pub mod state;

#[cfg(feature = "test-utils")]
pub mod test_utils;

// Re-exports for convenience
pub use approval::{
    ApprovalReason, ApprovalRequest, ApprovalResponse, PendingToolCall, ToolCallAction,
    ToolCallDecision,
};
pub use content::{Content, ContentBlock};
pub use context::{Message, MessageMeta, Role};
pub use dispatch::{
    Artifact, CollectedDispatch, DispatchEvent, DispatchHandle, DispatchSender, Dispatcher,
    EffectEmitter,
};
pub use dispatch_context::{AuthIdentity, DispatchContext, Extensions, TraceContext};
pub use duration::DurationMs;
pub use effect::{Effect, EffectKind, EffectMeta, MemoryScope, Scope, SignalPayload};
pub use environment::{Environment, EnvironmentSpec};
pub use error::{EnvError, OperatorError, OrchError, StateError};
pub use id::{DispatchId, OperatorId, SessionId, WorkflowId};
pub use lifecycle::CompactionPolicy;
pub use middleware::{
    DispatchMiddleware, DispatchNext, DispatchStack, ExecMiddleware, ExecNext, ExecStack,
    StoreMiddleware, StoreReadNext, StoreStack, StoreWriteNext,
};
pub use operator::{
    ExitReason, Operator, OperatorConfig, OperatorInput, OperatorMeta, OperatorMetadata,
    OperatorOutput, SubDispatchRecord, ToolMetadata,
};
pub use secret::{SecretAccessEvent, SecretAccessOutcome, SecretSource};
pub use state::{
    ContentKind, Lifetime, MemoryLink, MemoryTier, SearchOptions, SearchResult, StateReader,
    StateStore, StoreOptions,
};
pub use reducer::{AppendList, MergeObject, Overwrite, ReducerRegistry, StateReducer, Sum};
