#![deny(missing_docs)]
//! Local effect executor implementation.

use async_trait::async_trait;
use layer0::content::Content;
use layer0::effect::{Effect, Scope};
use layer0::error::StateError;
use layer0::middleware::{StoreStack, StoreWriteNext};
use layer0::operator::{OperatorInput, TriggerType};
use layer0::orchestrator::Orchestrator;
use layer0::state::{StateStore, StoreOptions};
use neuron_effects_core::{EffectExecutor, Error, UnknownEffectPolicy};
use serde_json::json;
use std::sync::Arc;

/// Local executor that applies memory effects to a `StateStore` and
/// translates orchestration effects into `Orchestrator` calls.
///
/// Semantics:
/// - WriteMemory/DeleteMemory: executed directly against the supplied state.
/// - Delegate: immediate dispatch via `Orchestrator::dispatch`.
/// - Handoff: immediate dispatch via `Orchestrator::dispatch` with a metadata
///   flag set to mark semantic handoff. The flag is `{ "handoff": true }` on
///   the dispatched `OperatorInput`'s `metadata` field.
/// - Signal: sent via `Orchestrator::signal`.
///
/// Unknown/custom effects: ignored by default (warn logged). Configurable via
/// `unknown_policy`.
pub struct LocalEffectExecutor<S: StateStore + ?Sized, O: Orchestrator + ?Sized> {
    /// State backend used for memory effects.
    pub state: Arc<S>,
    /// Orchestrator used for delegation, handoff, and signals.
    pub orch: Arc<O>,
    /// Unknown effect handling policy.
    pub unknown_policy: UnknownEffectPolicy,
    middleware: Option<StoreStack>,
}

impl<S: StateStore + ?Sized, O: Orchestrator + ?Sized> LocalEffectExecutor<S, O> {
    /// Create a new local effect executor with default policy `IgnoreAndWarn`.
    pub fn new(state: Arc<S>, orch: Arc<O>) -> Self {
        Self {
            state,
            orch,
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

struct WriteTo<S: StateStore + ?Sized>(Arc<S>);

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
        self.0.write_hinted(scope, key, value, opts).await
    }
}

#[async_trait]
impl<S, O> EffectExecutor for LocalEffectExecutor<S, O>
where
    S: StateStore + ?Sized + 'static,
    O: Orchestrator + ?Sized + 'static,
{
    async fn execute(&self, effects: &[Effect]) -> Result<(), Error> {
        for effect in effects {
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
                    let terminal = WriteTo(self.state.clone());
                    if let Some(stack) = &self.middleware {
                        stack
                            .write_with(scope, key, value.clone(), Some(&opts), &terminal)
                            .await?;
                    } else {
                        self.state
                            .write_hinted(scope, key, value.clone(), &opts)
                            .await?;
                    }
                }
                Effect::DeleteMemory { scope, key } => {
                    // StateStore::delete is idempotent by contract — missing key is Ok.
                    self.state.delete(scope, key).await?;
                }
                Effect::Signal { target, payload } => {
                    self.orch.signal(target, payload.clone()).await?;
                }
                Effect::Delegate { agent, input } => {
                    self.orch.dispatch(agent, (*input.clone()).clone()).await?;
                }
                Effect::Handoff { agent, state } => {
                    // Serialize handoff state into the message body with a semantic flag.
                    let mut input =
                        OperatorInput::new(Content::text(state.to_string()), TriggerType::Task);
                    input.metadata = json!({ "handoff": true });
                    self.orch.dispatch(agent, input).await?;
                }
                // Known but non-executing effects: treat as unknown for policy handling.
                Effect::Log { .. } | Effect::Custom { .. } => match self.unknown_policy {
                    UnknownEffectPolicy::IgnoreAndWarn => {
                        tracing::warn!("ignoring unsupported effect: {:?}", effect);
                    }
                    UnknownEffectPolicy::Error => return Err(Error::UnknownEffect),
                },
                // Forward-compat: Effect is non_exhaustive; handle any future variants.
                _ => match self.unknown_policy {
                    UnknownEffectPolicy::IgnoreAndWarn => {
                        tracing::warn!("ignoring forward-compatible effect variant: {:?}", effect);
                    }
                    UnknownEffectPolicy::Error => return Err(Error::UnknownEffect),
                },
            }
        }
        Ok(())
    }
}
