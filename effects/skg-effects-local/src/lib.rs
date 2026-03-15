#![deny(missing_docs)]
//! Local effect handler implementation.
//!
//! Provides [`LocalEffectHandler`] — an in-process [`EffectHandler`] that
//! applies memory effects to a [`StateStore`], delivers signals via
//! [`Signalable`], and returns dispatch intents as [`EffectOutcome`] variants
//! for the caller to act on.

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::effect::{Effect, Scope};
use layer0::error::{OrchError, StateError};
use layer0::middleware::{StoreStack, StoreWriteNext};
use layer0::operator::{OperatorInput, TriggerType};
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
    middleware: Option<StoreStack>,
}

impl<S: StateStore + ?Sized> LocalEffectHandler<S> {
    /// Create a new local effect handler with default policy `IgnoreAndWarn`.
    pub fn new(state: Arc<S>, signaler: Option<Arc<dyn Signalable>>) -> Self {
        Self {
            state,
            signaler,
            unknown_policy: UnknownEffectPolicy::IgnoreAndWarn,
            middleware: None,
        }
    }

    /// Override the unknown/custom effect handling policy.
    pub fn with_unknown_policy(mut self, policy: UnknownEffectPolicy) -> Self {
        self.unknown_policy = policy;
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
        match effect {
            Effect::WriteMemory {
                scope,
                key,
                value,
                tier,
                lifetime,
                content_kind,
                salience,
                ttl,
            } => {
                let opts = StoreOptions {
                    tier: *tier,
                    lifetime: *lifetime,
                    content_kind: content_kind.clone(),
                    salience: *salience,
                    ttl: *ttl,
                };
                if let Some(stack) = &self.middleware {
                    let committed = Arc::new(AtomicBool::new(false));
                    let terminal = WriteTo {
                        store: self.state.clone(),
                        committed: committed.clone(),
                    };
                    stack
                        .write_with(scope, key, value.clone(), Some(&opts), &terminal)
                        .await?;
                    if committed.load(Ordering::Acquire) {
                        Ok(EffectOutcome::Applied)
                    } else {
                        Ok(EffectOutcome::Skipped)
                    }
                } else {
                    self.state
                        .write_hinted(scope, key, value.clone(), &opts)
                        .await?;
                    Ok(EffectOutcome::Applied)
                }
            }
            Effect::DeleteMemory { scope, key } => {
                // StateStore::delete is idempotent by contract — missing key is Ok.
                self.state.delete(scope, key).await?;
                Ok(EffectOutcome::Applied)
            }
            Effect::Signal { target, payload } => match &self.signaler {
                Some(s) => {
                    s.signal(target, payload.clone()).await?;
                    Ok(EffectOutcome::Applied)
                }
                None => Err(Error::Dispatch(OrchError::DispatchFailed(
                    "signal requires a Signalable implementation".into(),
                ))),
            },
            Effect::Delegate { operator, input } => Ok(EffectOutcome::Delegate {
                operator: operator.clone(),
                input: (*input.clone()).clone(),
            }),
            Effect::Handoff { operator, state } => {
                let mut input =
                    OperatorInput::new(Content::text(state.to_string()), TriggerType::Task);
                input.metadata = serde_json::Value::Null;
                Ok(EffectOutcome::Handoff {
                    operator: operator.clone(),
                    input,
                })
            }
            // Known but non-executing effects: treat as unknown for policy handling.
            Effect::Log { .. } | Effect::Custom { .. } => match self.unknown_policy {
                UnknownEffectPolicy::IgnoreAndWarn => {
                    tracing::warn!("ignoring unsupported effect: {:?}", effect);
                    Ok(EffectOutcome::Skipped)
                }
                UnknownEffectPolicy::Error => Err(Error::UnknownEffect),
            },
            // Forward-compat: Effect is non_exhaustive; handle any future variants.
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
