//! The single abstraction for context transformation: [`Middleware`].
//!
//! Implemented by named structs for reusable middleware (budget guards,
//! compaction policies, telemetry recorders) or by async closures for
//! one-off transforms. Replaces `ContextOp`, `ErasedOp`, `Rule`, and
//! `Trigger` with a single trait and Vec ordering.

use crate::context::Context;
use crate::error::EngineError;
use std::future::Future;
use std::pin::Pin;

/// The single abstraction for context transformation.
///
/// Implemented by named structs for reusable middleware, or by async
/// closures for one-offs. Middleware runs in Vec order — no priority
/// numbers, no typed triggers. If you need conditional execution,
/// put the `if` inside your `process()`.
pub trait Middleware: Send + Sync {
    /// Transform context. Called before or after inference depending on
    /// which phase this middleware is registered in.
    fn process(&self, ctx: &mut Context) -> impl Future<Output = Result<(), EngineError>> + Send;

    /// Human-readable name for tracing and debugging.
    /// Defaults to the type name.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// Type-erased middleware for heterogeneous `Vec<Box<dyn ErasedMiddleware>>`.
///
/// The `Middleware` trait uses RPITIT (`impl Future`), which is not
/// object-safe. This companion trait erases the future type behind a
/// boxed future so middleware can be stored in a `Vec`.
pub trait ErasedMiddleware: Send + Sync {
    /// Process context, returning a boxed future.
    fn process_erased<'a>(
        &'a self,
        ctx: &'a mut Context,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>>;

    /// Human-readable name for tracing and debugging.
    fn name(&self) -> &str;
}

/// Blanket impl: any `Middleware` is automatically `ErasedMiddleware`.
impl<T: Middleware> ErasedMiddleware for T {
    fn process_erased<'a>(
        &'a self,
        ctx: &'a mut Context,
    ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>> {
        Box::pin(self.process(ctx))
    }

    fn name(&self) -> &str {
        Middleware::name(self)
    }
}

/// A middleware built from a closure that returns a boxed future.
///
/// Use [`middleware_fn`] to construct. This is the escape hatch when
/// a struct-based `Middleware` impl is too ceremonious for a one-off.
pub struct MiddlewareFn<F> {
    f: F,
    label: &'static str,
}

/// Create a middleware from a closure that borrows `&mut Context`.
///
/// ```ignore
/// pipeline.push_before(Box::new(middleware_fn("inject_sys", |ctx| {
///     Box::pin(async move {
///         ctx.push_message(Message::new(Role::System, Content::text("hi")));
///         Ok(())
///     })
/// })));
/// ```
pub fn middleware_fn<F>(label: &'static str, f: F) -> MiddlewareFn<F>
where
    F: for<'a> Fn(
            &'a mut Context,
        ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>>
        + Send
        + Sync,
{
    MiddlewareFn { f, label }
}

impl<F> Middleware for MiddlewareFn<F>
where
    F: for<'a> Fn(
            &'a mut Context,
        ) -> Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send + 'a>>
        + Send
        + Sync,
{
    fn process(&self, ctx: &mut Context) -> impl Future<Output = Result<(), EngineError>> + Send {
        (self.f)(ctx)
    }

    fn name(&self) -> &str {
        self.label
    }
}
