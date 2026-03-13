use crate::runner::{EffectInterpreter, KitError, LocalEffectInterpreter, OrchestratedRunner};
use layer0::dispatch::Dispatcher;
use layer0::state::StateStore;
use skg_effects_core::Signalable;
use std::sync::Arc;

/// Unopinionated wiring handle for assembling runnable systems.
///
/// This is intentionally small: it holds protocol implementations and provides
/// helpers for common local wiring. Callers can always bypass this and wire
/// directly against `layer0`.
#[derive(Clone)]
pub struct Kit {
    dispatcher: Arc<dyn Dispatcher>,
    signaler: Option<Arc<dyn Signalable>>,
    state: Option<Arc<dyn StateStore>>,
}

impl Kit {
    /// Create a new kit with the given dispatcher.
    pub fn new(dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            dispatcher,
            signaler: None,
            state: None,
        }
    }

    /// Attach a signaler for workflows that need signal delivery.
    pub fn with_signaler(mut self, signaler: Arc<dyn Signalable>) -> Self {
        self.signaler = Some(signaler);
        self
    }

    /// Attach a state backend for helpers that need to execute memory effects.
    pub fn with_state(mut self, state: Arc<dyn StateStore>) -> Self {
        self.state = Some(state);
        self
    }

    /// Access the configured dispatcher.
    pub fn dispatcher(&self) -> &Arc<dyn Dispatcher> {
        &self.dispatcher
    }

    /// Access the configured signaler, if any.
    pub fn signaler(&self) -> Option<&Arc<dyn Signalable>> {
        self.signaler.as_ref()
    }

    /// Access the configured state backend, if any.
    pub fn state(&self) -> Option<&Arc<dyn StateStore>> {
        self.state.as_ref()
    }

    /// Build a runner using the provided effect interpreter.
    pub fn runner_with_interpreter<E: EffectInterpreter>(
        &self,
        executor: Arc<E>,
    ) -> OrchestratedRunner<E> {
        OrchestratedRunner::new(
            Arc::clone(&self.dispatcher),
            self.signaler.clone(),
            executor,
        )
    }

    /// Build a local runner that interprets memory effects against the kit state backend.
    pub fn local_runner(
        &self,
    ) -> Result<OrchestratedRunner<LocalEffectInterpreter<dyn StateStore>>, KitError> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| KitError::Effect("local_runner requires a state backend".into()))?;
        Ok(OrchestratedRunner::new(
            Arc::clone(&self.dispatcher),
            self.signaler.clone(),
            Arc::new(LocalEffectInterpreter::new(Arc::clone(state))),
        ))
    }
}
