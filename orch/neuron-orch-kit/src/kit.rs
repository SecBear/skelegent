use crate::runner::{EffectInterpreter, KitError, LocalEffectInterpreter, OrchestratedRunner};
use layer0::orchestrator::Orchestrator;
use layer0::state::StateStore;
use std::sync::Arc;

/// Unopinionated wiring handle for assembling runnable systems.
///
/// This is intentionally small: it holds protocol implementations and provides
/// helpers for common local wiring. Callers can always bypass this and wire
/// directly against `layer0`.
#[derive(Clone)]
pub struct Kit {
    orch: Arc<dyn Orchestrator>,
    state: Option<Arc<dyn StateStore>>,
}

impl Kit {
    /// Create a new kit around an orchestrator implementation.
    pub fn new(orch: Arc<dyn Orchestrator>) -> Self {
        Self { orch, state: None }
    }

    /// Attach a state backend for helpers that need to execute memory effects.
    pub fn with_state(mut self, state: Arc<dyn StateStore>) -> Self {
        self.state = Some(state);
        self
    }

    /// Access the configured orchestrator.
    pub fn orchestrator(&self) -> &Arc<dyn Orchestrator> {
        &self.orch
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
        OrchestratedRunner::new(Arc::clone(&self.orch), executor)
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
            Arc::clone(&self.orch),
            Arc::new(LocalEffectInterpreter::new(Arc::clone(state))),
        ))
    }
}
