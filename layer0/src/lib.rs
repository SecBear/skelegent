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
pub mod capability;
pub mod content;
pub mod context;
pub mod dispatch;
pub mod dispatch_context;
pub mod duration;
pub mod environment;
pub mod error;
pub mod event;
pub mod extract;
pub mod id;
pub mod intent;
pub mod lifecycle;
pub mod middleware;
pub mod operator;
pub mod reducer;
pub mod secret;
pub mod state;
pub mod wait;

#[cfg(feature = "test-utils")]
pub mod test_utils;

// ── Re-exports ───────────────────────────────────────────────────────────────

// Outcome family
pub use operator::{InterceptionKind, LimitReason, Outcome, TerminalOutcome, TransferOutcome};

// Intent
pub use intent::{
    HandoffContext, Intent, IntentKind, IntentMeta, MemoryScope, Scope, SignalPayload,
};

// ExecutionEvent
pub use event::{EventKind, EventMeta, EventSource, ExecutionEvent};

// Wait / Resume
pub use wait::{ResumeInput, WaitReason, WaitState};

// Capability discovery
pub use capability::{
    ApprovalFacts, AuthFacts, CapabilityDescriptor, CapabilityFilter, CapabilityId, CapabilityKind,
    CapabilityModality, CapabilitySource, ExecutionClass, SchedulingFacts, StreamingSupport,
};

// Uniform error
pub use error::{EnvError, ErrorCode, ProtocolError, StateError};

// Dispatch
pub use dispatch::{
    Artifact, CollectedDispatch, CollectedInvocation, DispatchEvent, DispatchHandle, DispatchSender,
    Dispatcher, InvocationHandle,
};
pub use dispatch_context::{AuthIdentity, DispatchContext, Extensions, TraceContext};
pub use duration::DurationMs;
pub use environment::{Environment, EnvironmentSpec};
pub use id::{DispatchId, OperatorId, SessionId, WorkflowId};
pub use lifecycle::CompactionPolicy;
pub use middleware::{
    DispatchMiddleware, DispatchNext, DispatchStack, ExecMiddleware, ExecNext, ExecStack,
    StoreMiddleware, StoreReadNext, StoreStack, StoreWriteNext,
};
pub use operator::{
    Operator, OperatorConfig, OperatorInput, OperatorMeta, OperatorMetadata, OperatorOutput,
    SubDispatchRecord, ToolMetadata,
};
pub use approval::{
    ApprovalReason, ApprovalRequest, ApprovalResponse, PendingToolCall, ToolCallAction,
    ToolCallDecision,
};
pub use content::{Content, ContentBlock};
pub use context::{Message, MessageMeta, Role};
pub use reducer::{AppendList, MergeObject, Overwrite, ReducerRegistry, StateReducer, Sum};
pub use secret::{SecretAccessEvent, SecretAccessOutcome, SecretSource};
pub use state::{
    ContentKind, Lifetime, MemoryLink, MemoryTier, SearchOptions, SearchResult, StateReader,
    StateStore, StoreOptions,
};

pub use extract::{Ext, FromContext, Rejection};
