//! LocalEnvironment — no isolation, just passthrough to the operator.

use crate::environment::EnvironmentSpec;
use crate::error::EnvError;
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
        self.operator
            .execute(input)
            .await
            .map_err(EnvError::OperatorError)
    }
}
