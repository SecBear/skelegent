//! Dispatch capabilities for operator composition.
//!
//! [`Capabilities`] is the operator's window into the system it runs inside.
//! It provides optional access to a [`Dispatcher`] for invoking sibling
//! operators inline (blocking), and tracks composition depth to prevent
//! unbounded nesting.
//!
//! Operators that don't compose simply ignore capabilities. Operators that
//! need to delegate work use [`Capabilities::dispatcher()`] to get a handle
//! and call [`Dispatcher::dispatch()`] to invoke siblings.
//!
//! ## Two Dispatch Paths
//!
//! - **Direct call**: operator holds `Arc<dyn Operator>` and calls `execute()`
//!   directly. Fastest, no middleware, no environment routing. Use for
//!   tightly-coupled co-located operators.
//!
//! - **Via dispatcher**: operator calls `capabilities.dispatcher().dispatch()`.
//!   Goes through the orchestrator's middleware stack, environment routing,
//!   and potentially crosses network boundaries. Use when you need middleware
//!   visibility, budget tracking, or cross-environment dispatch.

use crate::error::OrchError;
use crate::id::OperatorId;
use crate::operator::{OperatorInput, OperatorOutput};
use async_trait::async_trait;
use std::sync::Arc;

/// A narrow dispatch interface for invoking sibling operators.
///
/// Implementations:
/// - Orchestrator adapter (wraps `Arc<dyn Orchestrator>`, full middleware)
/// - Direct adapter (wraps `Arc<dyn Operator>`, bypasses orchestrator)
/// - gRPC client (cross-container callback to host orchestrator)
/// - Test mock (records dispatches for assertions)
#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Dispatch to a sibling operator and block until it completes.
    ///
    /// Returns the child's output or an error. The dispatcher implementation
    /// decides how to route: in-process, through middleware, across gRPC, etc.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError>;
}

/// Runtime capabilities available to an operator during execution.
///
/// Provided by the orchestrator (or test harness) when calling
/// [`Operator::execute()`]. Operators that don't compose ignore this entirely.
///
/// # Depth Tracking
///
/// Every dispatch through [`Capabilities::dispatcher()`] increments depth.
/// When `depth >= max_depth`, the dispatcher returns
/// [`OrchError::DispatchFailed`] with a depth-exceeded message.
/// Use [`Capabilities::child()`] to create capabilities for a child
/// invocation with incremented depth.
pub struct Capabilities {
    /// Optional dispatcher for invoking sibling operators.
    dispatcher: Option<Arc<dyn Dispatcher>>,

    /// Current depth in the composition tree. Root = 0.
    depth: u32,

    /// Maximum allowed nesting depth. Default: 8.
    max_depth: u32,
}

impl Capabilities {
    /// Create capabilities with no dispatcher (standalone operator).
    pub fn none() -> Self {
        Self {
            dispatcher: None,
            depth: 0,
            max_depth: 8,
        }
    }

    /// Create capabilities with a dispatcher.
    pub fn with_dispatcher(dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            dispatcher: Some(dispatcher),
            depth: 0,
            max_depth: 8,
        }
    }

    /// Set the maximum nesting depth.
    pub fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Get the dispatcher, if available.
    pub fn dispatcher(&self) -> Option<&dyn Dispatcher> {
        self.dispatcher.as_deref()
    }

    /// Current composition depth (0 = root).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Maximum allowed depth.
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Whether further nesting is allowed.
    pub fn can_dispatch(&self) -> bool {
        self.depth < self.max_depth
    }

    /// Create child capabilities with incremented depth.
    ///
    /// The child shares the same dispatcher and max_depth but has
    /// `depth + 1`. Pass this to child operators (directly or via
    /// the dispatcher) to track nesting.
    pub fn child(&self) -> Self {
        Self {
            dispatcher: self.dispatcher.clone(),
            depth: self.depth + 1,
            max_depth: self.max_depth,
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::none()
    }
}

impl std::fmt::Debug for Capabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capabilities")
            .field("has_dispatcher", &self.dispatcher.is_some())
            .field("depth", &self.depth)
            .field("max_depth", &self.max_depth)
            .finish()
    }
}
