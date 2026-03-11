//! LocalOrchestrator — in-process orchestrator with a HashMap of operators.

use crate::error::OrchError;
use crate::id::OperatorId;
use crate::operator::{Operator, OperatorInput, OperatorOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// In-process orchestrator that dispatches operator invocations to registered operators.
/// Uses `Arc<dyn Operator>` for true concurrent dispatch via `tokio::spawn`.
pub struct LocalOrchestrator {
    operators: HashMap<String, Arc<dyn Operator>>,
}

impl LocalOrchestrator {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            operators: HashMap::new(),
        }
    }

    /// Register an operator with the orchestrator.
    pub fn register(&mut self, id: OperatorId, operator: Arc<dyn Operator>) {
        self.operators.insert(id.0, operator);
    }
}

impl Default for LocalOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl crate::dispatch::Dispatcher for LocalOrchestrator {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .operators
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?;
        op.execute(input).await.map_err(OrchError::OperatorError)
    }
}
