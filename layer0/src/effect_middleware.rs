//! Middleware for cross-cutting effect concerns (logging, validation, audit).

use crate::dispatch_context::DispatchContext;
use crate::effect::Effect;
use crate::effect_log::EffectLog;
use async_trait::async_trait;
use std::sync::Arc;

// ── EffectAction ──────────────────────────────────────────────────────────────

/// Outcome of effect middleware processing.
///
/// Returned by [`EffectMiddleware::on_effect`] to indicate whether execution
/// should proceed with (a possibly modified) effect or be suppressed.
#[derive(Debug)]
pub enum EffectAction {
    /// Continue processing this effect (possibly modified).
    Continue(Box<Effect>),
    /// Skip this effect — do not execute or log it.
    ///
    /// Causes [`EffectStack::process`] to return `None` immediately.
    /// Layers after the skipping layer never run.
    Skip,
}

// ── EffectMiddleware ──────────────────────────────────────────────────────────

/// Middleware for cross-cutting effect concerns.
///
/// Runs before effect execution. Can observe, enrich, validate,
/// or suppress effects. Must **NOT** contain business logic — use
/// [`crate::effect_log::EffectLog`] or an [`crate::dispatch::DispatchHandle`]
/// for side effects; use an EffectHandler (effects crate) for business logic.
///
/// # Valid uses
/// Logging, enrichment, validation, audit, rate limiting, tracing.
///
/// # Not for
/// Business logic, saga coordination, deduplication.
#[async_trait]
pub trait EffectMiddleware: Send + Sync {
    /// Process an effect before execution.
    async fn on_effect(&self, effect: Effect, ctx: &DispatchContext) -> EffectAction;
}

// ── EffectStack ───────────────────────────────────────────────────────────────

/// Composable stack of effect middleware.
///
/// Layers are processed in insertion order (first added = outermost = first to
/// run). Any layer may return [`EffectAction::Skip`] to suppress the effect and
/// stop further processing.
pub struct EffectStack {
    layers: Vec<Box<dyn EffectMiddleware>>,
}

impl EffectStack {
    /// Create an empty stack.
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Add a middleware layer (builder-style).
    ///
    /// Layers are applied in the order they are pushed.
    pub fn push(mut self, middleware: impl EffectMiddleware + 'static) -> Self {
        self.layers.push(Box::new(middleware));
        self
    }

    /// Process an effect through all layers in order.
    ///
    /// Returns `Some(effect)` after all layers have run, or `None` if any
    /// layer returned [`EffectAction::Skip`].
    pub async fn process(&self, effect: Effect, ctx: &DispatchContext) -> Option<Effect> {
        let mut current = effect;
        for layer in &self.layers {
            match layer.on_effect(current, ctx).await {
                EffectAction::Continue(e) => current = *e,
                EffectAction::Skip => return None,
            }
        }
        Some(current)
    }
}

impl Default for EffectStack {
    fn default() -> Self {
        Self::new()
    }
}

// ── LoggingEffectMiddleware ───────────────────────────────────────────────────

/// Middleware that appends every durable effect to an [`EffectLog`].
///
/// Ephemeral effects (Log, Progress, Metric, Observation, Artifact) pass
/// through without being written to the log. See
/// [`crate::effect::EffectKind::is_durable`] for the classification.
///
/// Logging failures are silently ignored so a storage hiccup never
/// prevents effect execution.
pub struct LoggingEffectMiddleware<L: EffectLog> {
    log: Arc<L>,
}

impl<L: EffectLog> LoggingEffectMiddleware<L> {
    /// Create a new logging middleware backed by the given log.
    pub fn new(log: Arc<L>) -> Self {
        Self { log }
    }
}

#[async_trait]
impl<L: EffectLog + 'static> EffectMiddleware for LoggingEffectMiddleware<L> {
    async fn on_effect(&self, effect: Effect, _ctx: &DispatchContext) -> EffectAction {
        if effect.kind.is_durable() {
            // Ignore logging errors — a log failure must not block execution.
            let _ = self.log.append(&effect).await;
        }
        EffectAction::Continue(Box::new(effect))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch_context::DispatchContext;
    use crate::effect::{Effect, EffectKind, MemoryScope, Scope};
    use crate::effect_log::InMemoryEffectLog;
    use crate::id::{DispatchId, OperatorId, SessionId};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn make_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("d1"), OperatorId::new("op1"))
    }

    fn make_write() -> Effect {
        Effect::write_memory(
            Scope::Session(SessionId::new("s1")),
            "key-test".to_owned(),
            json!(1),
            MemoryScope::Session,
        )
    }

    fn make_log_effect() -> Effect {
        Effect::log("info", "test")
    }

    // We need two variants — one that continues, one that skips.
    struct ContinueMiddleware {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EffectMiddleware for ContinueMiddleware {
        async fn on_effect(&self, effect: Effect, _ctx: &DispatchContext) -> EffectAction {
            self.seen.lock().await.push(effect.meta.effect_id.clone());
            EffectAction::Continue(Box::new(effect))
        }
    }

    struct SkipMiddleware {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EffectMiddleware for SkipMiddleware {
        async fn on_effect(&self, effect: Effect, _ctx: &DispatchContext) -> EffectAction {
            self.seen.lock().await.push(effect.meta.effect_id.clone());
            EffectAction::Skip
        }
    }

    #[tokio::test]
    async fn effect_stack_processes_all_layers() {
        let seen1 = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen2 = Arc::new(Mutex::new(Vec::<String>::new()));

        let stack = EffectStack::new()
            .push(ContinueMiddleware {
                seen: seen1.clone(),
            })
            .push(ContinueMiddleware {
                seen: seen2.clone(),
            });

        let ctx = make_ctx();
        let effect = make_write();
        let eid = effect.meta.effect_id.clone();

        let result = stack.process(effect, &ctx).await;
        assert!(result.is_some());

        // Both layers must have seen the effect.
        assert_eq!(seen1.lock().await.as_slice(), [eid.as_str()]);
        assert_eq!(seen2.lock().await.as_slice(), [eid.as_str()]);
    }

    #[tokio::test]
    async fn effect_stack_skip_stops_processing() {
        let seen_after = Arc::new(Mutex::new(Vec::<String>::new()));

        let stack = EffectStack::new()
            .push(SkipMiddleware {
                seen: Arc::new(Mutex::new(Vec::new())),
            })
            .push(ContinueMiddleware {
                seen: seen_after.clone(),
            });

        let ctx = make_ctx();
        let result = stack.process(make_write(), &ctx).await;

        // Stack must return None when a layer skips.
        assert!(result.is_none());
        // The layer after the skip must never have run.
        assert!(
            seen_after.lock().await.is_empty(),
            "layer after Skip must not execute"
        );
    }

    #[tokio::test]
    async fn logging_middleware_logs_durable_only() {
        let log = Arc::new(InMemoryEffectLog::new());
        let mw = LoggingEffectMiddleware::new(log.clone());
        let ctx = make_ctx();

        // Durable: WriteMemory.
        let durable = make_write();
        let result = mw.on_effect(durable, &ctx).await;
        assert!(matches!(result, EffectAction::Continue(_)));

        // Ephemeral: Log.
        let ephemeral = make_log_effect();
        let result = mw.on_effect(ephemeral, &ctx).await;
        assert!(matches!(result, EffectAction::Continue(_)));

        // Only the durable effect must appear in the log.
        let entries = log.read(0, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            matches!(&entries[0].kind, EffectKind::WriteMemory { .. }),
            "only the WriteMemory effect should be logged"
        );
    }
}
