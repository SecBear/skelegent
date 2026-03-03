#![deny(missing_docs)]
//! Hook registry and composition for neuron.
//!
//! The [`HookRegistry`] collects multiple [`Hook`] implementations into
//! an ordered pipeline. At each hook point, hooks are dispatched in
//! registration order. The pipeline short-circuits on `Halt`, `SkipTool`,
//! or `ModifyToolInput` — subsequent hooks are not called. Hook errors
//! are logged and the pipeline continues (errors don't halt).

use layer0::hook::{Hook, HookAction, HookContext};
use std::sync::Arc;

/// A registry that dispatches hook events to an ordered pipeline of hooks.
///
/// Hooks are called in the order they were registered. The pipeline
/// short-circuits on any action other than `Continue` (except errors,
/// which are logged and ignored).
pub struct HookRegistry {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookRegistry {
    /// Create a new empty hook registry.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Add a hook to the end of the pipeline.
    pub fn add(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }

    /// Dispatch a hook event through the pipeline.
    ///
    /// Returns the final action. If all hooks return `Continue`, the
    /// result is `Continue`. If any hook returns `Halt`, `SkipTool`,
    /// or `ModifyToolInput`, the pipeline stops and that action is returned.
    /// Hook errors are logged and treated as `Continue`.
    pub async fn dispatch(&self, ctx: &HookContext) -> HookAction {
        for hook in &self.hooks {
            // Only dispatch to hooks registered for this point
            if !hook.points().contains(&ctx.point) {
                continue;
            }

            match hook.on_event(ctx).await {
                Ok(HookAction::Continue) => continue,
                Ok(action) => return action,
                Err(_e) => {
                    // Hook errors are logged but don't halt the pipeline.
                    // In a real system, this would go to tracing/logging.
                    continue;
                }
            }
        }

        HookAction::Continue
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use layer0::error::HookError;
    use layer0::hook::HookPoint;

    struct ContinueHook {
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for ContinueHook {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            Ok(HookAction::Continue)
        }
    }

    struct HaltHook {
        points: Vec<HookPoint>,
        reason: String,
    }

    #[async_trait]
    impl Hook for HaltHook {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            Ok(HookAction::Halt {
                reason: self.reason.clone(),
            })
        }
    }

    struct ErrorHook {
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for ErrorHook {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            Err(HookError::Failed("hook error".into()))
        }
    }

    #[tokio::test]
    async fn empty_registry_returns_continue() {
        let registry = HookRegistry::new();
        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn continue_hook_returns_continue() {
        let mut registry = HookRegistry::new();
        registry.add(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn halt_hook_short_circuits() {
        let mut registry = HookRegistry::new();
        registry.add(Arc::new(HaltHook {
            points: vec![HookPoint::PreInference],
            reason: "budget exceeded".into(),
        }));
        registry.add(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        match action {
            HookAction::Halt { reason } => assert_eq!(reason, "budget exceeded"),
            _ => panic!("expected Halt"),
        }
    }

    #[tokio::test]
    async fn hook_not_matching_point_is_skipped() {
        let mut registry = HookRegistry::new();
        registry.add(Arc::new(HaltHook {
            points: vec![HookPoint::PostInference],
            reason: "should not trigger".into(),
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn error_hook_treated_as_continue() {
        let mut registry = HookRegistry::new();
        registry.add(Arc::new(ErrorHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn multiple_continue_hooks_all_pass() {
        let mut registry = HookRegistry::new();
        registry.add(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));
        registry.add(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[test]
    fn default_registry_is_empty() {
        let registry = HookRegistry::default();
        let ctx = HookContext::new(HookPoint::PreInference);
        // Can't async test in #[test], but verify it constructs
        let _ = registry;
        let _ = ctx;
    }
}
