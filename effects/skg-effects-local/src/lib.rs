#![deny(missing_docs)]
//! Local effect handler implementation.
//!
//! Provides [`LocalEffectHandler`] — an in-process [`EffectHandler`] that
//! applies memory effects to a [`StateStore`], delivers signals via
//! [`Signalable`], and returns dispatch intents as [`EffectOutcome`] variants
//! for the caller to act on.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::effect::{Effect, EffectKind, Scope};
use layer0::error::{OrchError, StateError};
use layer0::middleware::{StoreStack, StoreWriteNext};
use layer0::operator::{OperatorInput, TriggerType};
use layer0::reducer::ReducerRegistry;
use layer0::state::{StateStore, StoreOptions};
use skg_effects_core::Signalable;
use skg_effects_core::{EffectHandler, EffectOutcome, Error, UnknownEffectPolicy};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Local handler that applies memory effects to a [`StateStore`] and
/// delivers signals via [`Signalable`].
///
/// Semantics:
/// - WriteMemory/DeleteMemory: executed directly against the supplied state.
/// - Delegate: returned as [`EffectOutcome::Delegate`] for the caller.
/// - Handoff: returned as [`EffectOutcome::Handoff`] for the caller.
/// - Signal: sent via [`Signalable::signal`].
///
/// Unknown/custom effects: ignored by default (warn logged). Configurable via
/// `unknown_policy`.
pub struct LocalEffectHandler<S: StateStore + ?Sized> {
    /// State backend used for memory effects.
    pub state: Arc<S>,
    /// Signaler used for signal effects.
    pub signaler: Option<Arc<dyn Signalable>>,
    /// Unknown effect handling policy.
    pub unknown_policy: UnknownEffectPolicy,
    /// Optional reducer registry. When set, `WriteMemory` reads the current
    /// value, applies the registered reducer for that key, and writes the
    /// merged result. When absent, writes overwrite unconditionally (default).
    pub reducer_registry: Option<Arc<ReducerRegistry>>,
    middleware: Option<StoreStack>,
}

impl<S: StateStore + ?Sized> LocalEffectHandler<S> {
    /// Create a new local effect handler with default policy `IgnoreAndWarn`.
    pub fn new(state: Arc<S>, signaler: Option<Arc<dyn Signalable>>) -> Self {
        Self {
            state,
            signaler,
            unknown_policy: UnknownEffectPolicy::IgnoreAndWarn,
            reducer_registry: None,
            middleware: None,
        }
    }

    /// Override the unknown/custom effect handling policy.
    pub fn with_unknown_policy(mut self, policy: UnknownEffectPolicy) -> Self {
        self.unknown_policy = policy;
        self
    }

    /// Attach a reducer registry. When set, each `WriteMemory` effect reads
    /// the current value, applies the registered reducer for the key, and
    /// writes the merged result. Keys without an explicit entry use the
    /// registry's default reducer (initially [`layer0::reducer::Overwrite`]).
    pub fn with_reducer_registry(mut self, registry: Arc<ReducerRegistry>) -> Self {
        self.reducer_registry = Some(registry);
        self
    }

    /// Attach a store middleware stack. Each `WriteMemory` effect is routed through
    /// the stack before reaching the state backend.
    ///
    /// A guard middleware can skip the write by not calling `next` and returning `Ok(())`.
    /// A transformer middleware can substitute the value before calling `next`.
    pub fn with_store_middleware(mut self, stack: StoreStack) -> Self {
        self.middleware = Some(stack);
        self
    }
}

// ── WriteTo: StoreWriteNext terminal ────────────────────────────────────────

/// Terminal that writes to the state store and sets `committed` to `true`.
struct WriteTo<S: StateStore + ?Sized> {
    store: Arc<S>,
    committed: Arc<AtomicBool>,
}

#[async_trait]
impl<S: StateStore + ?Sized + 'static> StoreWriteNext for WriteTo<S> {
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: Option<&StoreOptions>,
    ) -> Result<(), StateError> {
        let default_opts = StoreOptions::default();
        let opts = options.unwrap_or(&default_opts);
        self.store.write_hinted(scope, key, value, opts).await?;
        self.committed.store(true, Ordering::Release);
        Ok(())
    }
}

#[async_trait]
impl<S> EffectHandler for LocalEffectHandler<S>
where
    S: StateStore + ?Sized + 'static,
{
    async fn handle(
        &self,
        effect: &Effect,
        _ctx: &DispatchContext,
    ) -> Result<EffectOutcome, Error> {
        match &effect.kind {
            EffectKind::WriteMemory {
                scope,
                key,
                value,
                tier,
                lifetime,
                content_kind,
                salience,
                ttl,
                memory_scope: _,
            } => {
                let opts = StoreOptions {
                    tier: *tier,
                    lifetime: *lifetime,
                    content_kind: content_kind.clone(),
                    salience: *salience,
                    ttl: *ttl,
                };
                // If a reducer registry is configured, read the current value
                // and merge with the incoming value before writing. This keeps
                // the middleware path and the direct path consistent.
                let write_value = match &self.reducer_registry {
                    Some(registry) => {
                        let current = self
                            .state
                            .read(scope, key)
                            .await?
                            .unwrap_or(serde_json::Value::Null);
                        registry.reduce(key, &current, value)
                    }
                    None => value.clone(),
                };
                if let Some(stack) = &self.middleware {
                    let committed = Arc::new(AtomicBool::new(false));
                    let terminal = WriteTo {
                        store: self.state.clone(),
                        committed: committed.clone(),
                    };
                    stack
                        .write_with(scope, key, write_value, Some(&opts), &terminal)
                        .await?;
                    if committed.load(Ordering::Acquire) {
                        Ok(EffectOutcome::Applied)
                    } else {
                        Ok(EffectOutcome::Skipped)
                    }
                } else {
                    self.state
                        .write_hinted(scope, key, write_value, &opts)
                        .await?;
                    Ok(EffectOutcome::Applied)
                }
            }
            EffectKind::DeleteMemory { scope, key } => {
                // StateStore::delete is idempotent by contract — missing key is Ok.
                self.state.delete(scope, key).await?;
                Ok(EffectOutcome::Applied)
            }
            EffectKind::Signal { target, payload } => match &self.signaler {
                Some(s) => {
                    s.signal(target, payload.clone()).await?;
                    Ok(EffectOutcome::Applied)
                }
                None => Err(Error::Dispatch(OrchError::DispatchFailed(
                    "signal requires a Signalable implementation".into(),
                ))),
            },
            EffectKind::Delegate { operator, input } => Ok(EffectOutcome::Delegate {
                operator: operator.clone(),
                input: (*input.clone()).clone(),
            }),
            EffectKind::Handoff { operator, context } => {
                // Build the operator input from the structured HandoffContext.
                // context.task is the primary input; context.history seeds the
                // pre-assembled context; context.metadata becomes OperatorInput.metadata.
                let mut input = OperatorInput::new(context.task.clone(), TriggerType::Task);
                if let Some(hist) = &context.history {
                    input.context = Some(hist.clone());
                }
                if let Some(meta) = &context.metadata {
                    input.metadata = meta.clone();
                }
                Ok(EffectOutcome::Handoff {
                    operator: operator.clone(),
                    input,
                })
            }
            EffectKind::LinkMemory { scope, link } => {
                self.state.link(scope, link).await?;
                Ok(EffectOutcome::Applied)
            }
            EffectKind::UnlinkMemory {
                scope,
                from_key,
                to_key,
                relation,
            } => {
                self.state.unlink(scope, from_key, to_key, relation).await?;
                Ok(EffectOutcome::Applied)
            }
            // Custom effects: treat as unknown for policy handling.
            EffectKind::Custom { .. } => match self.unknown_policy {
                UnknownEffectPolicy::IgnoreAndWarn => {
                    tracing::warn!("ignoring unsupported effect: {:?}", effect);
                    Ok(EffectOutcome::Skipped)
                }
                UnknownEffectPolicy::Error => Err(Error::UnknownEffect),
            },
            // Forward-compat: Effect is #[non_exhaustive].
            // Progress, Artifact, and ToolApprovalRequired are caller-interpreted
            // effects routed via EffectEmitter → DispatchHandle (dispatch-channel
            // wiring, not EffectHandler). They intentionally fall through here.
            _ => match self.unknown_policy {
                UnknownEffectPolicy::IgnoreAndWarn => {
                    tracing::warn!("ignoring forward-compatible effect variant: {:?}", effect);
                    Ok(EffectOutcome::Skipped)
                }
                UnknownEffectPolicy::Error => Err(Error::UnknownEffect),
            },
        }
    }
}
