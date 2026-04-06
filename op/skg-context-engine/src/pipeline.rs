//! Ordered middleware phases for the agent loop: [`Pipeline`].
//!
//! A pipeline holds two ordered lists of middleware — `before_send` runs
//! before each inference call (context assembly), `after_send` runs after
//! each inference response is appended. Vec order IS priority.

use crate::context::Context;
use crate::error::EngineError;
use crate::middleware::ErasedMiddleware;

/// Ordered middleware phases for the agent loop.
///
/// Middleware runs in Vec order. No priority numbers, no typed triggers.
/// `before_send` is the pre-inference boundary; `after_send` fires after
/// the response is committed to context.
pub struct Pipeline {
    /// Runs before each inference call (context assembly).
    pub before_send: Vec<Box<dyn ErasedMiddleware>>,
    /// Runs after each inference response is appended.
    pub after_send: Vec<Box<dyn ErasedMiddleware>>,
}

impl Pipeline {
    /// Create an empty pipeline (no middleware).
    pub fn new() -> Self {
        Self {
            before_send: Vec::new(),
            after_send: Vec::new(),
        }
    }

    /// Add middleware to the before-send phase.
    pub fn push_before(&mut self, mw: Box<dyn ErasedMiddleware>) {
        self.before_send.push(mw);
    }

    /// Add middleware to the after-send phase.
    pub fn push_after(&mut self, mw: Box<dyn ErasedMiddleware>) {
        self.after_send.push(mw);
    }

    /// Run all before-send middleware in order.
    pub async fn run_before(&self, ctx: &mut Context) -> Result<(), EngineError> {
        for mw in &self.before_send {
            let _span = tracing::debug_span!("middleware::before", name = mw.name());
            mw.process_erased(ctx).await?;
        }
        Ok(())
    }

    /// Run all after-send middleware in order.
    pub async fn run_after(&self, ctx: &mut Context) -> Result<(), EngineError> {
        for mw in &self.after_send {
            let _span = tracing::debug_span!("middleware::after", name = mw.name());
            mw.process_erased(ctx).await?;
        }
        Ok(())
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountMiddleware {
        counter: Arc<AtomicU32>,
        label: &'static str,
    }

    impl Middleware for CountMiddleware {
        async fn process(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn name(&self) -> &str {
            self.label
        }
    }

    #[tokio::test]
    async fn pipeline_runs_before_and_after_in_order() {
        let before_count = Arc::new(AtomicU32::new(0));
        let after_count = Arc::new(AtomicU32::new(0));

        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(CountMiddleware {
            counter: before_count.clone(),
            label: "before_1",
        }));
        pipeline.push_before(Box::new(CountMiddleware {
            counter: before_count.clone(),
            label: "before_2",
        }));
        pipeline.push_after(Box::new(CountMiddleware {
            counter: after_count.clone(),
            label: "after_1",
        }));

        let mut ctx = Context::new();
        pipeline.run_before(&mut ctx).await.unwrap();
        pipeline.run_after(&mut ctx).await.unwrap();

        assert_eq!(before_count.load(Ordering::SeqCst), 2);
        assert_eq!(after_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pipeline_error_stops_chain() {
        struct FailMiddleware;
        impl Middleware for FailMiddleware {
            async fn process(&self, _ctx: &mut Context) -> Result<(), EngineError> {
                Err(EngineError::Halted {
                    reason: "test halt".into(),
                })
            }
            fn name(&self) -> &str {
                "fail"
            }
        }

        let counter = Arc::new(AtomicU32::new(0));
        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(FailMiddleware));
        pipeline.push_before(Box::new(CountMiddleware {
            counter: counter.clone(),
            label: "should_not_run",
        }));

        let mut ctx = Context::new();
        let result = pipeline.run_before(&mut ctx).await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn closure_middleware_works() {
        use crate::middleware::middleware_fn;

        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(middleware_fn("inject", |ctx| {
            Box::pin(async move {
                ctx.push_message(Message::new(Role::System, Content::text("injected")));
                Ok(())
            })
        })));

        let mut ctx = Context::new();
        pipeline.run_before(&mut ctx).await.unwrap();
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "injected");
    }
}
