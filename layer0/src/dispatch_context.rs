//! Unified execution context threaded through every dispatch boundary.
//!
//! [`DispatchContext`] carries correlation, identity, tracing, and typed
//! extension data through the entire dispatch chain — from orchestrator
//! middleware, through operator execution, into tool calls and effect
//! handlers.
//!
//! ## Why this exists
//!
//! Before DispatchContext, different components carried different subsets
//! of identity and correlation: OperatorInput had session but no trace ID;
//! ToolCallContext had operator_id but no session or auth; middleware saw
//! operator + input but no dispatch ID; EffectEmitter had a sender channel
//! but no identity.
//!
//! DispatchContext is the single type that unifies all of these. Every
//! dispatch boundary receives it, every middleware can read and extend it,
//! and every integration (OTel, MCP, Temporal) can extract what it needs.

use crate::id::{DispatchId, OperatorId};
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH CONTEXT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Execution context threaded through every dispatch boundary.
///
/// Created by the orchestrator at dispatch entry, cloned for child
/// dispatches (delegation), and passed by reference to operators,
/// middleware, tools, and effect handlers.
///
/// # Extending
///
/// Use [`extensions`](Self::extensions) / [`extensions_mut`](Self::extensions_mut)
/// to attach cross-cutting data (credentials, budget trackers, feature
/// flags) without changing the context type itself:
///
/// ```rust,ignore
/// ctx.extensions_mut().insert(MyBudgetTracker::new(1000));
/// // later, in a tool or middleware:
/// let budget = ctx.extensions().get::<MyBudgetTracker>().unwrap();
/// ```
#[derive(Clone)]
pub struct DispatchContext {
    /// Unique ID for this dispatch invocation.
    pub dispatch_id: DispatchId,

    /// Parent dispatch ID for hierarchical delegation.
    ///
    /// `None` for root dispatches. Set automatically by [`child`](Self::child).
    pub parent_id: Option<DispatchId>,

    /// Which operator is being invoked.
    pub operator_id: OperatorId,

    /// W3C Trace Context for distributed tracing.
    pub trace: TraceContext,

    /// Caller identity. `None` for unauthenticated dispatches.
    pub identity: Option<AuthIdentity>,

    /// Optional deadline for this dispatch. If set, dispatchers should
    /// attempt to complete before this instant.
    deadline: Option<tokio::time::Instant>,

    /// Typed extension map for cross-cutting concerns.
    extensions: Extensions,
}

impl DispatchContext {
    /// Create a new root context for a dispatch invocation.
    ///
    /// Uses a default (empty) trace context and no identity.
    /// Call builder methods to add tracing, auth, and extensions.
    pub fn new(dispatch_id: DispatchId, operator_id: OperatorId) -> Self {
        Self {
            dispatch_id,
            parent_id: None,
            operator_id,
            trace: TraceContext::default(),
            identity: None,
            deadline: None,
            extensions: Extensions::new(),
        }
    }

    /// Create a child context for delegated dispatch.
    ///
    /// Inherits trace context, identity, and extensions from the parent.
    /// Sets `parent_id` to the parent's `dispatch_id`.
    pub fn child(&self, dispatch_id: DispatchId, operator_id: OperatorId) -> Self {
        Self {
            dispatch_id,
            parent_id: Some(self.dispatch_id.clone()),
            operator_id,
            trace: self.trace.child_span(),
            identity: self.identity.clone(),
            deadline: self.deadline,
            extensions: self.extensions.clone(),
        }
    }

    /// Set the trace context.
    pub fn with_trace(mut self, trace: TraceContext) -> Self {
        self.trace = trace;
        self
    }

    /// Set the caller identity.
    pub fn with_identity(mut self, identity: AuthIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Set a deadline on this context.
    pub fn with_deadline(mut self, deadline: tokio::time::Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Set a timeout (converted to deadline from now).
    pub fn with_timeout(mut self, duration: std::time::Duration) -> Self {
        self.deadline = Some(tokio::time::Instant::now() + duration);
        self
    }

    /// Get the deadline, if any.
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.deadline
    }

    /// Get remaining time until deadline, or `None` if no deadline set.
    ///
    /// Returns `Duration::ZERO` if the deadline has already passed.
    pub fn remaining(&self) -> Option<std::time::Duration> {
        self.deadline.map(|d| {
            let now = tokio::time::Instant::now();
            if d > now { d - now } else { std::time::Duration::ZERO }
        })
    }

    /// Check if the deadline has passed.
    ///
    /// Returns `false` if no deadline is set.
    pub fn is_expired(&self) -> bool {
        self.deadline.is_some_and(|d| tokio::time::Instant::now() >= d)
    }

    /// Read-only access to extensions.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Mutable access to extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
}

impl fmt::Debug for DispatchContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DispatchContext")
            .field("dispatch_id", &self.dispatch_id)
            .field("parent_id", &self.parent_id)
            .field("operator_id", &self.operator_id)
            .field("trace", &self.trace)
            .field("identity", &self.identity)
            .field("deadline", &self.deadline)
            .field("extensions", &self.extensions)
            .finish()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TRACE CONTEXT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// W3C Trace Context compatible tracing identifiers.
///
/// Carries the minimum viable set for distributed tracing interop:
/// `trace_id` (shared across an entire trace), `span_id` (unique per
/// span), `trace_flags` (sampling), and optional `trace_state` (vendor
/// data).
///
/// # Protocol compatibility
///
/// These fields map directly to the W3C `traceparent` header:
/// `{version}-{trace_id}-{span_id}-{trace_flags}`
///
/// The OTel middleware (`skg-hook-otel` in extras) converts these to
/// proper OpenTelemetry spans. This type is the protocol-level
/// representation — no OTel dependency required.
///
/// See: <https://www.w3.org/TR/trace-context/>
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    /// 16-byte trace ID as 32-char lowercase hex string.
    ///
    /// Shared across all spans in a distributed trace. Empty string
    /// means "no trace" (equivalent to all-zeros).
    #[serde(default)]
    pub trace_id: String,

    /// 8-byte span ID as 16-char lowercase hex string.
    ///
    /// Unique per span within a trace. Empty string means "no span."
    #[serde(default)]
    pub span_id: String,

    /// Trace flags byte. Bit 0 = sampled.
    #[serde(default)]
    pub trace_flags: u8,

    /// Vendor-specific trace state (key=value pairs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_state: Option<String>,
}

impl TraceContext {
    /// Create a trace context with the given IDs.
    pub fn new(trace_id: impl Into<String>, span_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            span_id: span_id.into(),
            trace_flags: 0,
            trace_state: None,
        }
    }

    /// Create a child span context.
    ///
    /// Inherits `trace_id`, `trace_flags`, and `trace_state` from the parent.
    /// Generates a new `span_id` derived from the parent (simple counter-based;
    /// production systems should use the OTel middleware for proper span IDs).
    pub fn child_span(&self) -> Self {
        // Deterministic child: append "-c" to parent span_id and truncate.
        // Real span ID generation happens in the OTel middleware; this is
        // a placeholder that maintains the invariant "child != parent."
        let child_span = if self.span_id.is_empty() {
            String::new()
        } else {
            let mut s = self.span_id.clone();
            // Rotate last char to produce a different but deterministic ID.
            // This is NOT cryptographically random — it's a protocol placeholder.
            if let Some(last) = s.pop() {
                let next = match last {
                    '0'..='e' => (last as u8 + 1) as char,
                    _ => '0',
                };
                s.push(next);
            }
            s
        };

        Self {
            trace_id: self.trace_id.clone(),
            span_id: child_span,
            trace_flags: self.trace_flags,
            trace_state: self.trace_state.clone(),
        }
    }

    /// Format as a W3C `traceparent` header value.
    ///
    /// Returns `None` if trace_id or span_id is empty.
    pub fn as_traceparent(&self) -> Option<String> {
        if self.trace_id.is_empty() || self.span_id.is_empty() {
            return None;
        }
        Some(format!(
            "00-{}-{}-{:02x}",
            self.trace_id, self.span_id, self.trace_flags
        ))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AUTH IDENTITY
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Caller identity propagated through the dispatch chain.
///
/// Represents the authenticated principal that initiated the dispatch.
/// Middleware, operators, and tools can inspect this to make authorization
/// decisions.
///
/// # Design
///
/// Deliberately minimal: `subject` (who) + optional `claims` (what they
/// can do). The authentication mechanism (JWT, API key, mTLS, etc.) is
/// the A2A/HTTP layer's concern — by the time identity reaches
/// DispatchContext, it's already verified.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthIdentity {
    /// The authenticated principal (user ID, service account, agent ID).
    pub subject: String,

    /// Optional structured claims (roles, scopes, permissions).
    ///
    /// Format is protocol-specific. JWT claims, OAuth scopes, or
    /// custom authorization data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<serde_json::Value>,
}

impl AuthIdentity {
    /// Create a new identity with just a subject.
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            claims: None,
        }
    }

    /// Create an identity with claims.
    pub fn with_claims(subject: impl Into<String>, claims: serde_json::Value) -> Self {
        Self {
            subject: subject.into(),
            claims: Some(claims),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXTENSIONS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Type-keyed extension map for cross-cutting context data.
///
/// Stores `Arc<T>` values keyed by `TypeId`. This is the same pattern
/// as `http::Extensions` but uses `Arc` for cheap cloning (since
/// [`DispatchContext`] is cloned for child dispatches).
///
/// # Usage
///
/// ```rust
/// use layer0::dispatch_context::Extensions;
///
/// #[derive(Debug)]
/// struct BudgetTracker { remaining: u64 }
///
/// let mut ext = Extensions::new();
/// ext.insert(BudgetTracker { remaining: 1000 });
///
/// let tracker = ext.get::<BudgetTracker>().unwrap();
/// assert_eq!(tracker.remaining, 1000);
/// ```
#[derive(Clone, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Create an empty extension map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a typed value.
    ///
    /// If a value of the same type was already present, it is replaced
    /// and the old value is returned.
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<Arc<T>> {
        self.map
            .insert(TypeId::of::<T>(), Arc::new(val))
            .and_then(|old| Arc::downcast::<T>(old).ok())
    }

    /// Get a reference to a typed value.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.downcast_ref::<T>())
    }

    /// Get a cloned `Arc` reference to a typed value.
    ///
    /// Useful when you need to hold the value beyond the borrow lifetime.
    pub fn get_arc<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|arc| Arc::clone(arc).downcast::<T>().ok())
    }

    /// Check whether a value of the given type is present.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Remove a typed value, returning it if present.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<Arc<T>> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|arc| Arc::downcast::<T>(arc).ok())
    }

    /// Number of stored extensions.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions")
            .field("count", &self.map.len())
            .finish_non_exhaustive()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_new_and_child() {
        let root = DispatchContext::new(DispatchId::new("d-001"), OperatorId::new("summarizer"));
        assert_eq!(root.dispatch_id.as_str(), "d-001");
        assert!(root.parent_id.is_none());
        assert!(root.identity.is_none());

        let child = root.child(DispatchId::new("d-002"), OperatorId::new("sub-summarizer"));
        assert_eq!(child.dispatch_id.as_str(), "d-002");
        assert_eq!(child.parent_id.as_ref().unwrap().as_str(), "d-001");
        assert_eq!(child.operator_id.as_str(), "sub-summarizer");
    }

    #[test]
    fn context_with_identity() {
        let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::new("op"))
            .with_identity(AuthIdentity::new("user-123"));

        let id = ctx.identity.as_ref().unwrap();
        assert_eq!(id.subject, "user-123");
        assert!(id.claims.is_none());
    }

    #[test]
    fn context_with_trace() {
        let ctx = DispatchContext::new(DispatchId::new("d-001"), OperatorId::new("op")).with_trace(
            TraceContext::new("0af7651916cd43dd8448eb211c80319c", "b7ad6b7169203331"),
        );

        assert_eq!(ctx.trace.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(ctx.trace.span_id, "b7ad6b7169203331");
    }

    #[test]
    fn trace_context_traceparent() {
        let tc = TraceContext {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "b7ad6b7169203331".into(),
            trace_flags: 0x01,
            trace_state: None,
        };
        assert_eq!(
            tc.as_traceparent().unwrap(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );
    }

    #[test]
    fn trace_context_empty_returns_none() {
        let tc = TraceContext::default();
        assert!(tc.as_traceparent().is_none());
    }

    #[test]
    fn trace_child_span_differs() {
        let parent = TraceContext::new("trace-abc", "span-0000");
        let child = parent.child_span();
        assert_eq!(child.trace_id, "trace-abc");
        assert_ne!(child.span_id, parent.span_id);
    }

    #[test]
    fn extensions_insert_get_remove() {
        #[derive(Debug, PartialEq)]
        struct Budget(u64);

        #[derive(Debug, PartialEq)]
        struct Tag(String);

        let mut ext = Extensions::new();
        assert!(ext.is_empty());

        ext.insert(Budget(1000));
        ext.insert(Tag("test".into()));
        assert_eq!(ext.len(), 2);

        assert_eq!(ext.get::<Budget>().unwrap().0, 1000);
        assert_eq!(ext.get::<Tag>().unwrap().0, "test");
        assert!(ext.get::<String>().is_none());

        assert!(ext.contains::<Budget>());
        assert!(!ext.contains::<String>());

        let removed = ext.remove::<Budget>().unwrap();
        assert_eq!(removed.0, 1000);
        assert_eq!(ext.len(), 1);
        assert!(ext.get::<Budget>().is_none());
    }

    #[test]
    fn extensions_clone_is_shallow() {
        #[derive(Debug)]
        struct Heavy(Vec<u8>);

        let mut ext = Extensions::new();
        ext.insert(Heavy(vec![0; 1024]));

        let cloned = ext.clone();
        // Both point to the same Arc — shallow clone.
        assert_eq!(cloned.get::<Heavy>().unwrap().0.len(), 1024);
    }

    #[test]
    fn extensions_replace_returns_old() {
        let mut ext = Extensions::new();
        let old = ext.insert(42u64);
        assert!(old.is_none());

        let old = ext.insert(99u64);
        assert_eq!(*old.unwrap(), 42);
        assert_eq!(*ext.get::<u64>().unwrap(), 99);
    }

    #[test]
    fn auth_identity_with_claims() {
        let id = AuthIdentity::with_claims(
            "agent-007",
            serde_json::json!({"role": "admin", "scopes": ["read", "write"]}),
        );
        assert_eq!(id.subject, "agent-007");
        assert!(id.claims.is_some());
    }

    #[test]
    fn dispatch_context_debug_does_not_panic() {
        let ctx = DispatchContext::new(DispatchId::new("d-debug"), OperatorId::new("op-debug"));
        let debug = format!("{ctx:?}");
        assert!(debug.contains("d-debug"));
    }

    #[test]
    fn with_timeout_creates_future_deadline() {
        let ctx = DispatchContext::new(DispatchId::new("d-1"), OperatorId::new("op-1"))
            .with_timeout(std::time::Duration::from_secs(10));
        assert!(ctx.deadline().is_some());
        assert!(ctx.deadline().unwrap() > tokio::time::Instant::now());
    }

    #[test]
    fn remaining_returns_some_when_deadline_set() {
        let ctx = DispatchContext::new(DispatchId::new("d-2"), OperatorId::new("op-2"))
            .with_timeout(std::time::Duration::from_secs(60));
        let remaining = ctx.remaining();
        assert!(remaining.is_some());
        assert!(remaining.unwrap() > std::time::Duration::ZERO);
    }

    #[test]
    fn is_expired_false_for_future_deadline() {
        let ctx = DispatchContext::new(DispatchId::new("d-3"), OperatorId::new("op-3"))
            .with_timeout(std::time::Duration::from_secs(60));
        assert!(!ctx.is_expired());
    }

    #[test]
    fn child_inherits_parent_deadline() {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let parent = DispatchContext::new(DispatchId::new("d-parent"), OperatorId::new("op-p"))
            .with_deadline(deadline);
        let child = parent.child(DispatchId::new("d-child"), OperatorId::new("op-c"));
        assert_eq!(child.deadline(), Some(deadline));
    }

    #[test]
    fn no_deadline_remaining_returns_none() {
        let ctx = DispatchContext::new(DispatchId::new("d-4"), OperatorId::new("op-4"));
        assert!(ctx.deadline().is_none());
        assert!(ctx.remaining().is_none());
        assert!(!ctx.is_expired());
    }

    #[test]
    fn expired_deadline_remaining_returns_zero() {
        // A deadline in the past.
        let deadline = tokio::time::Instant::now() - std::time::Duration::from_millis(1);
        let ctx = DispatchContext::new(DispatchId::new("d-5"), OperatorId::new("op-5"))
            .with_deadline(deadline);
        assert!(ctx.is_expired());
        assert_eq!(ctx.remaining(), Some(std::time::Duration::ZERO));
    }
}
