//! The universal primitive: [`ContextOp`].
//!
//! Every operation on context implements this trait. Operations are structs,
//! not closures — they can be inspected by type, matched by rules, and
//! composed without loss of information.

use crate::context::Context;
use crate::error::EngineError;
use async_trait::async_trait;

/// The universal operation primitive. Everything that transforms context
/// implements this: injection, compaction, tool dispatch, response appending,
/// budget checks, telemetry recording.
///
/// Rules are also `ContextOp`s — they have the same power as pipeline
/// operations, just different activation (reactive vs explicit).
#[async_trait]
pub trait ContextOp: Send + Sync {
    /// What this operation produces. Use `()` for side-effect-only ops.
    type Output: Send + 'static;

    /// Execute this operation against the given context.
    ///
    /// The operation has full `&mut` access to the context: it can read,
    /// insert, remove, reorder messages, modify extensions, push effects,
    /// update metrics — anything.
    async fn execute(&self, ctx: &mut Context) -> Result<Self::Output, EngineError>;
}

/// Type-erased version of [`ContextOp`] for storage in rules.
///
/// Rules need to store heterogeneous ops. This trait erases the `Output`
/// type (rules always produce `()`).
#[async_trait]
pub(crate) trait ErasedOp: Send + Sync {
    async fn execute_erased(&self, ctx: &mut Context) -> Result<(), EngineError>;
}

#[async_trait]
impl<T: ContextOp<Output = ()> + 'static> ErasedOp for T {
    async fn execute_erased(&self, ctx: &mut Context) -> Result<(), EngineError> {
        self.execute(ctx).await
    }
}
