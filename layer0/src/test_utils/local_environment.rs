//! LocalEnvironment — no isolation, just passthrough to the operator.

use crate::dispatch::EffectEmitter;
use crate::dispatch_context::DispatchContext;
use crate::environment::EnvironmentSpec;
use crate::error::EnvError;
use crate::id::{DispatchId, OperatorId};
use crate::operator::{Operator, OperatorInput, OperatorOutput};
use async_trait::async_trait;
use std::sync::Arc;

/// A passthrough environment that executes the operator directly with no isolation.
/// Used for local development and testing. The operator is provided at construction
/// time and stored internally — callers don't pass it on every run() call.
pub struct LocalEnvironment {
    operator: Arc<dyn Operator>,
}

impl LocalEnvironment {
    /// Create a new local environment wrapping the given operator.
    pub fn new(operator: Arc<dyn Operator>) -> Self {
        Self { operator }
    }
}

#[async_trait]
impl crate::environment::Environment for LocalEnvironment {
    async fn run(
        &self,
        input: OperatorInput,
        _spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError> {
        let ctx = DispatchContext::new(
            DispatchId::new("local-env"),
            OperatorId::new("local"),
        );
        self.operator
            .execute(input, &ctx, &EffectEmitter::noop())
            .await
            .map_err(EnvError::OperatorError)
    }
}