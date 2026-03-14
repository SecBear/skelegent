#![deny(missing_docs)]
//! A2A protocol wire types, Agent Card, and bidirectional conversions for skelegent.
//!
//! This crate defines the A2A protocol types as hand-written Rust structs that
//! serialize to JSON matching the A2A specification. It also provides conversions
//! between A2A types and skelegent's native types ([`layer0::Content`],
//! [`skg_run_core::RunStatus`], etc.).
//!
//! # Design
//!
//! Types are hand-written (not proto-generated) following skelegent's DIY-first
//! philosophy. This gives us ergonomic Rust APIs with embedded conversion logic
//! while maintaining wire-compatible JSON serialization.
//!
//! # Modules
//!
//! - [`types`] — A2A wire types (Part, Message, Task, Artifact, events, requests)
//! - [`card`] — Agent Card and discovery types
//! - [`convert`] — Bidirectional conversions to/from skelegent types
//! - [`jsonrpc`] — JSON-RPC 2.0 envelope types
//! - [`error`] — A2A protocol errors with JSON-RPC error codes

pub mod card;
pub mod convert;
pub mod error;
pub mod jsonrpc;
pub mod types;

// Re-export key types for convenience.
pub use card::{AgentCapabilities, AgentCard, AgentCardBuilder, AgentInterface, AgentSkill};
pub use error::A2aError;
pub use jsonrpc::{JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse};
pub use types::{
    A2aArtifact, A2aMessage, A2aRole, A2aTask, Part, PartContent, SendMessageRequest,
    SendMessageResponse, StreamResponse, SubscribeToTaskRequest, TaskState, TaskStatus,
};
