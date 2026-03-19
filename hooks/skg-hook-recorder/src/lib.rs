#![deny(missing_docs)]
//! Universal operation recorder for skelegent middleware.
//!
//! Records every operation passing through middleware stacks into a
//! pluggable [`RecordSink`]. Enables replay, evaluation, and debugging.
//!
//! # Usage
//!
//! Attach a recorder to any middleware stack and inspect captured entries:
//!
//! ```rust,ignore
//! use skg_hook_recorder::{DispatchRecorder, InMemorySink};
//! use std::sync::Arc;
//!
//! let sink = Arc::new(InMemorySink::new());
//! let recorder = DispatchRecorder::new(sink.clone());
//!
//! let stack = DispatchStack::builder()
//!     .observe(Arc::new(recorder))
//!     .build();
//!
//! // ... dispatch through stack ...
//!
//! let entries = sink.entries().await;
//! assert_eq!(entries.len(), 2); // pre + post
//! ```

use async_trait::async_trait;
use opentelemetry::Context as OtelContext;
use opentelemetry::trace::TraceContextExt as _;
use serde::{Deserialize, Serialize};

pub mod dispatch;
pub mod embed;
pub mod exec;
pub mod infer;
pub mod secret;
pub mod sink;
pub mod store;

pub use dispatch::DispatchRecorder;
pub use embed::EmbedRecorder;
pub use exec::ExecRecorder;
pub use infer::InferRecorder;
pub use secret::SecretRecorder;
pub use sink::InMemorySink;
pub use store::StoreRecorder;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SCHEMA VERSION
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Schema version for [`RecordEntry`]. Increment when the entry shape changes
/// in a breaking way to enable consumers to detect and handle migrations.
pub const SCHEMA_VERSION: u64 = 1;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BOUNDARY
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Which middleware boundary produced this record entry.
///
/// Variants map to middleware boundaries across Layer 0 and Layer 1.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Boundary {
    /// A dispatch operation crossing `DispatchMiddleware`.
    Dispatch,
    /// A state write operation crossing `StoreMiddleware`.
    StoreWrite,
    /// A state read operation crossing `StoreMiddleware`.
    StoreRead,
    /// An environment execution crossing `ExecMiddleware`.
    Exec,
    /// An inference call crossing `InferMiddleware`.
    Infer,
    /// An embedding call crossing `EmbedMiddleware`.
    Embed,
    /// A secret resolution crossing `SecretMiddleware`.
    Secret,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PHASE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Whether the record was captured before or after calling `next`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Captured before calling `next` — input snapshot.
    Pre,
    /// Captured after calling `next` — outcome snapshot.
    Post,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RECORD CONTEXT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Correlation identifiers extracted from the dispatch context.
///
/// Present on all boundary records that pass through `DispatchMiddleware`.
/// Store and exec recorders that lack a dispatch context use empty strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordContext {
    /// W3C trace ID from [`layer0::dispatch_context::TraceContext::trace_id`].
    pub trace_id: String,
    /// The operator being dispatched, as a string.
    pub operator_id: String,
    /// The unique ID of this dispatch invocation.
    pub dispatch_id: String,
}

impl RecordContext {
    /// Create an empty context (used by exec/store recorders that lack a dispatch context).
    pub fn empty() -> Self {
        Self {
            trace_id: String::new(),
            operator_id: String::new(),
            dispatch_id: String::new(),
        }
    }
}

/// Extract a [`RecordContext`] from the ambient OpenTelemetry span.
///
/// When an active OTel span exists (e.g. set up by `OtelMiddleware`), this
/// populates `trace_id` and `dispatch_id` from the span context. The
/// `operator_id` is left empty because it is not available at the infer/embed
/// level — only the dispatch-level recorders have that information.
///
/// Falls back to [`RecordContext::empty`] when no valid span is active, which
/// is the common case in unit tests.
pub fn context_from_otel() -> RecordContext {
    let cx = OtelContext::current();
    let span = cx.span();
    let sc = span.span_context();
    if sc.is_valid() {
        RecordContext {
            trace_id: sc.trace_id().to_string(),
            operator_id: String::new(), // not available at infer/embed level
            dispatch_id: sc.span_id().to_string(),
        }
    } else {
        RecordContext::empty()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_from_otel_returns_empty_with_no_active_span() {
        // In tests there is no active OTel span, so context_from_otel()
        // must fall back to RecordContext::empty().
        let ctx = context_from_otel();
        assert_eq!(ctx, RecordContext::empty());
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RECORD ENTRY
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A single recorded operation captured by a recorder middleware.
///
/// Every operation that passes through a recorder produces two entries:
/// one at [`Phase::Pre`] (before calling `next`) and one at [`Phase::Post`]
/// (after `next` returns). The `Post` entry includes timing and any error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordEntry {
    /// Which middleware boundary this entry was captured at.
    pub boundary: Boundary,
    /// Whether this is a pre-call or post-call snapshot.
    pub phase: Phase,
    /// Correlation identifiers for the operation.
    pub context: RecordContext,
    /// Serialized payload — the input at `Pre`, outcome info at `Post`.
    pub payload_json: serde_json::Value,
    /// Wall-clock duration of the `next` call in milliseconds. Only set at [`Phase::Post`].
    pub duration_ms: Option<u64>,
    /// Error message if `next` returned an error. Only set at [`Phase::Post`].
    pub error: Option<String>,
    /// Schema version — always [`SCHEMA_VERSION`] for entries produced by this crate.
    pub version: u64,
}

impl RecordEntry {
    /// Create a pre-phase entry.
    pub fn pre(
        boundary: Boundary,
        context: RecordContext,
        payload_json: serde_json::Value,
    ) -> Self {
        Self {
            boundary,
            phase: Phase::Pre,
            context,
            payload_json,
            duration_ms: None,
            error: None,
            version: SCHEMA_VERSION,
        }
    }

    /// Create a post-phase entry.
    pub fn post(
        boundary: Boundary,
        context: RecordContext,
        payload_json: serde_json::Value,
        duration_ms: u64,
        error: Option<String>,
    ) -> Self {
        Self {
            boundary,
            phase: Phase::Post,
            context,
            payload_json,
            duration_ms: Some(duration_ms),
            error,
            version: SCHEMA_VERSION,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RECORD SINK TRAIT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A pluggable destination for recorded operation entries.
///
/// Implement this trait to direct recordings to any backend:
/// in-memory buffers, log streams, message queues, or databases.
///
/// The `record` method is called once per [`RecordEntry`]. It must not panic.
/// Implementations should handle errors internally (log and discard if necessary)
/// to avoid interfering with the middleware chain.
#[async_trait]
pub trait RecordSink: Send + Sync {
    /// Receive a single recorded entry.
    async fn record(&self, entry: RecordEntry);
}
