//! Dispatch — the single invocation primitive.
//!
//! [`Dispatcher`] is the one way to invoke an operator. Orchestrators
//! implement it. Operators that compose hold `Arc<dyn Dispatcher>` as a
//! field (constructor injection).
//!
//! ## Why one trait
//!
//! Mature frameworks (Erlang, Akka, LangChain) converge on a single
//! invocation primitive. `pid ! Message`, `actorRef.tell()`,
//! `Runnable.invoke()`. There is no separate "orchestrator dispatch"
//! vs "operator dispatch" — one interface, used everywhere.
//!
//! ## Composition via constructor injection
//!
//! Operators that don't compose never see dispatch infrastructure.
//! Operators that do compose receive `Arc<dyn Dispatcher>` at
//! construction time:
//!
//! ```rust,ignore
//! struct CoordinatorOp {
//!     dispatcher: Arc<dyn Dispatcher>,
//!     provider: Arc<dyn Provider>,
//! }
//!
//! impl Operator for CoordinatorOp {
//!     async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
//!         // delegate to a sibling — goes through orchestrator middleware
//!         let child_output = self.dispatcher
//!             .dispatch(&OperatorId::new("summarizer"), child_input)
//!             .await
//!             .map_err(|e| OperatorError::NonRetryable(e.to_string()))?;
//!         // ...
//!     }
//! }
//! ```
//!
//! The orchestrator passes itself (it implements `Dispatcher`) at
//! registration time. No circular dependency — operators are registered
//! first, then the orchestrator wraps itself as `Arc<dyn Dispatcher>`
//! and injects it into operators that need it.
//!
//! ## Depth tracking
//!
//! Not a framework concern. Erlang and Akka don't limit message-passing
//! depth. If you need it, add a [`DispatchMiddleware`](crate::middleware::DispatchMiddleware)
//! that tracks call depth per session.

use crate::error::OrchError;
use crate::id::OperatorId;
use crate::operator::{OperatorInput, OperatorOutput};
use async_trait::async_trait;

/// The single invocation primitive for operators.
///
/// Every orchestrator implements this. Operators that need to invoke
/// siblings hold `Arc<dyn Dispatcher>` as a field.
///
/// The implementation decides routing: in-process, through middleware,
/// across gRPC, over HTTP. Callers don't know and don't care.
#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Invoke an operator by ID and block until it completes.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError>;
}
