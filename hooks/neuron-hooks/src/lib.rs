#![deny(missing_docs)]
//! Hook registry and composition for neuron.
//!
//! The [`HookRegistry`] collects multiple [`Hook`] implementations into
//! a kind-aware pipeline. Hooks are partitioned into three kinds
//! ([`HookKind`]) that run in three phases per dispatch call:
//!
//! 1. **Observers** — all run regardless; returned actions and errors are
//!    ignored (errors are logged via `tracing::warn`).
//! 2. **Transformers** — run in registration order; each sees the
//!    *modified* context produced by the previous transformer. A `Halt`
//!    from a transformer escalates immediately. Other returned actions
//!    accumulate (last writer wins per field).
//! 3. **Guardrails** — run in registration order against the *original*
//!    context (not the transformer-modified one). Short-circuit on the
//!    first `Halt` or `SkipDispatch`. Errors are logged and the pipeline
//!    continues.
//!
//! Within each phase, hooks execute in the order they were registered.

use layer0::hook::{Hook, HookAction, HookContext};
use std::sync::Arc;

/// How a hook composes with others of the same kind at the same point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// Short-circuits on `Halt` or `SkipDispatch`. For policy enforcement.
    Guardrail,
    /// Chains `Modify` actions — each sees the previous hook's modified
    /// context. A `Halt` from a `Transformer` escalates like a `Guardrail`.
    Transformer,
    /// All run regardless of actions returned. For logging and telemetry.
    Observer,
}

/// A registry that dispatches hook events through a kind-aware pipeline.
///
/// Hooks run in three phases: [`HookKind::Observer`] →
/// [`HookKind::Transformer`] → [`HookKind::Guardrail`]. Within each
/// phase, hooks fire in registration order.
pub struct HookRegistry {
    hooks: Vec<(Arc<dyn Hook>, HookKind)>,
}

impl HookRegistry {
    /// Create a new empty hook registry.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Add a hook with an explicit [`HookKind`].
    pub fn add(&mut self, hook: Arc<dyn Hook>, kind: HookKind) {
        self.hooks.push((hook, kind));
    }

    /// Convenience: add a [`HookKind::Guardrail`] hook.
    pub fn add_guardrail(&mut self, hook: Arc<dyn Hook>) {
        self.add(hook, HookKind::Guardrail);
    }

    /// Convenience: add a [`HookKind::Transformer`] hook.
    pub fn add_transformer(&mut self, hook: Arc<dyn Hook>) {
        self.add(hook, HookKind::Transformer);
    }

    /// Convenience: add a [`HookKind::Observer`] hook.
    pub fn add_observer(&mut self, hook: Arc<dyn Hook>) {
        self.add(hook, HookKind::Observer);
    }

    /// Dispatch a hook event through the three-phase pipeline.
    ///
    /// # Return value
    ///
    /// - If a transformer or guardrail returns `Halt`, that is returned
    ///   immediately.
    /// - If a guardrail returns `SkipDispatch`, that is returned immediately.
    /// - If any transformer produced a `ModifyDispatchInput` or
    ///   `ModifyDispatchOutput`, the last such modification (with its final
    ///   accumulated value) is returned.
    /// - Otherwise `Continue` is returned.
    ///
    /// Observer actions are always discarded. Errors from any phase are
    /// logged via `tracing::warn` and treated as `Continue`.
    pub async fn dispatch(&self, ctx: &HookContext) -> HookAction {
        // ── Phase 1: Observers ──────────────────────────────────────────
        // All observers run. Returned actions are discarded; errors logged.
        for (hook, kind) in &self.hooks {
            if *kind != HookKind::Observer {
                continue;
            }
            if !hook.points().contains(&ctx.point) {
                continue;
            }
            match hook.on_event(ctx).await {
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    hook_point = ?ctx.point,
                    kind = "observer",
                    error = %e,
                    "hook error (observer, continuing)"
                ),
            }
        }

        // ── Phase 2: Transformers ───────────────────────────────────────
        // Each transformer sees the working context mutated by its
        // predecessors. A `Halt` from any transformer escalates immediately.
        //
        // `ModifyDispatchOutput` yields a `serde_json::Value`; we serialise it
        // to a JSON string and store it in `working_ctx.operator_result` so
        // subsequent transformers can read it for further chaining.
        let mut working_ctx = ctx.clone();
        let mut transformer_result: Option<HookAction> = None;

        for (hook, kind) in &self.hooks {
            if *kind != HookKind::Transformer {
                continue;
            }
            if !hook.points().contains(&working_ctx.point) {
                continue;
            }
            match hook.on_event(&working_ctx).await {
                Ok(HookAction::Continue) => {}
                Ok(HookAction::ModifyDispatchInput { new_input }) => {
                    working_ctx.operator_input = Some(new_input.clone());
                    transformer_result = Some(HookAction::ModifyDispatchInput { new_input });
                }
                Ok(HookAction::ModifyDispatchOutput { new_output }) => {
                    // Serialise Value → JSON string for chaining via operator_result.
                    working_ctx.operator_result = Some(new_output.to_string());
                    transformer_result = Some(HookAction::ModifyDispatchOutput { new_output });
                }
                Ok(HookAction::Halt { reason }) => {
                    return HookAction::Halt { reason };
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    hook_point = ?working_ctx.point,
                    kind = "transformer",
                    error = %e,
                    "hook error (transformer, continuing)"
                ),
            }
        }

        // ── Phase 3: Guardrails ─────────────────────────────────────────
        // Guardrails see the *original* ctx, not the transformer-modified
        // working context. Policy must be enforced against unmodified input.
        for (hook, kind) in &self.hooks {
            if *kind != HookKind::Guardrail {
                continue;
            }
            if !hook.points().contains(&ctx.point) {
                continue;
            }
            match hook.on_event(ctx).await {
                Ok(HookAction::Continue) => {}
                Ok(HookAction::Halt { reason }) => {
                    return HookAction::Halt { reason };
                }
                Ok(HookAction::SkipDispatch { reason }) => {
                    return HookAction::SkipDispatch { reason };
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    hook_point = ?ctx.point,
                    kind = "guardrail",
                    error = %e,
                    "hook error (guardrail, continuing)"
                ),
            }
        }

        // Return the last transformer modification (if any), else Continue.
        transformer_result.unwrap_or(HookAction::Continue)
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
    use std::sync::atomic::{AtomicBool, Ordering};

    // ── Shared test hooks ──────────────────────────────────────────────

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

    /// A transformer that appends a suffix to `ctx.operator_result`.
    ///
    /// Reads the raw string stored in `operator_result` (which is the JSON
    /// serialisation of the previous transformer's `Value`) and appends
    /// its suffix directly. This lets chaining tests verify that each
    /// transformer sees the prior transformer's output.
    struct AppendOutputTransformer {
        points: Vec<HookPoint>,
        suffix: &'static str,
    }

    #[async_trait]
    impl Hook for AppendOutputTransformer {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
            let base = ctx.operator_result.as_deref().unwrap_or("");
            Ok(HookAction::ModifyDispatchOutput {
                new_output: serde_json::Value::String(format!("{}{}", base, self.suffix)),
            })
        }
    }

    /// A flag hook: records when it fires, returns Continue.
    struct FlagHook {
        points: Vec<HookPoint>,
        fired: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Hook for FlagHook {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            self.fired.store(true, Ordering::SeqCst);
            Ok(HookAction::Continue)
        }
    }

    /// Logs its label when it fires; returns Continue.
    struct LabelHook {
        points: Vec<HookPoint>,
        label: &'static str,
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl Hook for LabelHook {
        fn points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn on_event(&self, _ctx: &HookContext) -> Result<HookAction, HookError> {
            self.log.lock().unwrap().push(self.label);
            Ok(HookAction::Continue)
        }
    }

    // ── Existing behaviour tests ───────────────────────────────────────

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
        registry.add_guardrail(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn halt_hook_short_circuits() {
        let mut registry = HookRegistry::new();
        registry.add_guardrail(Arc::new(HaltHook {
            points: vec![HookPoint::PreInference],
            reason: "budget exceeded".into(),
        }));
        registry.add_guardrail(Arc::new(ContinueHook {
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
        registry.add_guardrail(Arc::new(HaltHook {
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
        registry.add_guardrail(Arc::new(ErrorHook {
            points: vec![HookPoint::PreInference],
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn multiple_continue_hooks_all_pass() {
        let mut registry = HookRegistry::new();
        registry.add_guardrail(Arc::new(ContinueHook {
            points: vec![HookPoint::PreInference],
        }));
        registry.add_guardrail(Arc::new(ContinueHook {
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

    // ── New tests: HookKind-aware dispatch ─────────────────────────────

    /// Two transformer hooks chain: the second receives the first's output.
    #[tokio::test]
    async fn transformer_hooks_chain() {
        let mut registry = HookRegistry::new();
        // First transformer appends "A" to the (empty) operator_result.
        registry.add_transformer(Arc::new(AppendOutputTransformer {
            points: vec![HookPoint::PostSubDispatch],
            suffix: "A",
        }));
        // Second transformer reads working_ctx.operator_result (= JSON repr of
        // "A") and appends "+B".
        registry.add_transformer(Arc::new(AppendOutputTransformer {
            points: vec![HookPoint::PostSubDispatch],
            suffix: "+B",
        }));

        let ctx = HookContext::new(HookPoint::PostSubDispatch);
        let action = registry.dispatch(&ctx).await;
        match action {
            HookAction::ModifyDispatchOutput { new_output } => {
                let s = new_output.as_str().expect("string Value");
                assert!(s.contains('A'), "expected 'A' in output, got: {s}");
                assert!(s.contains("+B"), "expected '+B' in output, got: {s}");
            }
            _ => panic!("expected ModifyDispatchOutput, got {:?}", action),
        }
    }

    /// An observer that errors must not prevent a subsequent guardrail
    /// from running and halting.
    #[tokio::test]
    async fn observer_error_does_not_prevent_guardrail() {
        let mut registry = HookRegistry::new();
        registry.add_observer(Arc::new(ErrorHook {
            points: vec![HookPoint::PreInference],
        }));
        registry.add_guardrail(Arc::new(HaltHook {
            points: vec![HookPoint::PreInference],
            reason: "policy".into(),
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(
            matches!(action, HookAction::Halt { .. }),
            "expected Halt, got {:?}",
            action
        );
    }

    /// The first guardrail to Halt must stop subsequent guardrails from
    /// running.
    #[tokio::test]
    async fn guardrail_short_circuits_on_halt() {
        let second_fired = Arc::new(AtomicBool::new(false));

        let mut registry = HookRegistry::new();
        registry.add_guardrail(Arc::new(HaltHook {
            points: vec![HookPoint::PreInference],
            reason: "first halts".into(),
        }));
        registry.add_guardrail(Arc::new(FlagHook {
            points: vec![HookPoint::PreInference],
            fired: second_fired.clone(),
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        let action = registry.dispatch(&ctx).await;
        assert!(matches!(action, HookAction::Halt { .. }));
        assert!(
            !second_fired.load(Ordering::SeqCst),
            "second guardrail must not fire after first halts"
        );
    }

    /// Hooks registered in reverse phase order must still execute in
    /// observer → transformer → guardrail phase order.
    #[tokio::test]
    async fn dispatch_order_observer_transformer_guardrail() {
        let log = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

        let mut registry = HookRegistry::new();
        // Register in reverse phase order; dispatch must reorder by kind.
        registry.add_guardrail(Arc::new(LabelHook {
            points: vec![HookPoint::PreInference],
            label: "guardrail",
            log: log.clone(),
        }));
        registry.add_transformer(Arc::new(LabelHook {
            points: vec![HookPoint::PreInference],
            label: "transformer",
            log: log.clone(),
        }));
        registry.add_observer(Arc::new(LabelHook {
            points: vec![HookPoint::PreInference],
            label: "observer",
            log: log.clone(),
        }));

        let ctx = HookContext::new(HookPoint::PreInference);
        registry.dispatch(&ctx).await;

        let log = log.lock().unwrap();
        assert_eq!(
            *log,
            vec!["observer", "transformer", "guardrail"],
            "expected observer → transformer → guardrail order"
        );
    }
}
