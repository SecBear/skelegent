//! Typed extraction from [`DispatchContext`].
//!
//! Provides axum-style dependency injection for tool functions. Implement
//! [`FromContext`] to declare what a tool needs from the dispatch context;
//! the runtime extracts it before the tool runs.
//!
//! # Example
//!
//! ```rust,ignore
//! use layer0::extract::{Ext, FromContext};
//!
//! fn my_tool(Ext(pool): Ext<PgPool>, args: Args) -> Result<Value, ToolError> {
//!     // pool is PgPool, cloned from DispatchContext extensions
//! }
//! ```

use crate::DispatchContext;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// REJECTION
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Error returned when extraction from [`DispatchContext`] fails.
///
/// Carries a human-readable message describing what was missing or
/// why extraction could not complete.
#[derive(Debug, thiserror::Error)]
#[error("extraction failed: {0}")]
pub struct Rejection(pub String);

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FROM CONTEXT TRAIT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract a typed value from a [`DispatchContext`].
///
/// Inspired by axum's `FromRequestParts`. Implement this trait to enable
/// typed dependency injection for tools: each parameter a tool declares
/// is extracted independently before the tool body runs.
///
/// # Implementing
///
/// ```rust,ignore
/// use layer0::extract::{FromContext, Rejection};
/// use layer0::DispatchContext;
///
/// struct MyService(Arc<InnerService>);
///
/// impl FromContext for MyService {
///     fn from_context(ctx: &DispatchContext) -> Result<Self, Rejection> {
///         ctx.extensions()
///             .get::<Arc<InnerService>>()
///             .cloned()
///             .map(MyService)
///             .ok_or_else(|| Rejection("InnerService not configured".into()))
///     }
/// }
/// ```
pub trait FromContext: Sized {
    /// Extract `Self` from the dispatch context.
    ///
    /// Return `Err(Rejection)` when the required data is absent or invalid.
    /// The message in `Rejection` is surfaced to the caller.
    fn from_context(ctx: &DispatchContext) -> Result<Self, Rejection>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXT<T> EXTRACTOR
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Newtype extractor that clones a value from [`Extensions`](crate::Extensions).
///
/// Use in tool function parameters to access typed application state:
///
/// ```rust,ignore
/// fn my_tool(Ext(pool): Ext<PgPool>, args: Args) -> Result<Value, ToolError> {
///     pool.query(…)
/// }
/// ```
///
/// The inner `T` must be registered via
/// [`DispatchContext::with_extension`](crate::DispatchContext::with_extension)
/// or [`Extensions::insert`](crate::Extensions::insert) before dispatch.
#[derive(Debug)]
pub struct Ext<T>(pub T);

impl<T: Clone + Send + Sync + 'static> FromContext for Ext<T> {
    fn from_context(ctx: &DispatchContext) -> Result<Self, Rejection> {
        ctx.extensions()
            .get::<T>()
            .cloned()
            .map(Ext)
            .ok_or_else(|| {
                Rejection(format!(
                    "missing extension: {}",
                    std::any::type_name::<T>()
                ))
            })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BUILT-IN EXTRACTORS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract the [`OperatorId`](crate::id::OperatorId) from the context.
///
/// Always succeeds: every `DispatchContext` carries an operator ID.
impl FromContext for crate::id::OperatorId {
    fn from_context(ctx: &DispatchContext) -> Result<Self, Rejection> {
        Ok(ctx.operator_id.clone())
    }
}

/// Extract the [`DispatchId`](crate::id::DispatchId) from the context.
///
/// Always succeeds: every `DispatchContext` carries a dispatch ID.
impl FromContext for crate::id::DispatchId {
    fn from_context(ctx: &DispatchContext) -> Result<Self, Rejection> {
        Ok(ctx.dispatch_id.clone())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{DispatchId, OperatorId};

    fn make_ctx() -> DispatchContext {
        DispatchContext::new(
            DispatchId::new("dispatch-1"),
            OperatorId::new("operator-1"),
        )
    }

    #[test]
    fn ext_extracts_from_extensions() {
        let ctx = make_ctx().with_extension(String::from("hello"));
        let Ext(val) = Ext::<String>::from_context(&ctx).expect("should extract");
        assert_eq!(val, "hello");
    }

    #[test]
    fn ext_rejects_missing() {
        let ctx = make_ctx();
        let err = Ext::<String>::from_context(&ctx).expect_err("should reject");
        // Error message must name the missing type so callers know what to configure.
        assert!(
            err.to_string().contains("alloc::string::String"),
            "expected type name in rejection, got: {err}"
        );
    }

    #[test]
    fn operator_id_extractable() {
        let ctx = make_ctx();
        let id = OperatorId::from_context(&ctx).expect("should extract");
        assert_eq!(id.as_str(), "operator-1");
    }

    #[test]
    fn dispatch_id_extractable() {
        let ctx = make_ctx();
        let id = DispatchId::from_context(&ctx).expect("should extract");
        assert_eq!(id.as_str(), "dispatch-1");
    }

    #[test]
    fn ensure_passes_when_present() {
        let ctx = make_ctx().with_extension(42u32);
        ctx.ensure::<u32>().expect("should pass");
    }

    #[test]
    fn ensure_fails_when_missing() {
        let ctx = make_ctx();
        let err = ctx.ensure::<u32>().expect_err("should fail");
        assert!(
            err.contains("u32"),
            "expected type name in error, got: {err}"
        );
    }
}
