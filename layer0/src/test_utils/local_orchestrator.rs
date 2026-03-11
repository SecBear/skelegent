//! LocalOrchestrator — in-process orchestrator with a HashMap of operators.

use crate::effect::SignalPayload;
use crate::error::OrchError;
use crate::id::{OperatorId, WorkflowId};
use crate::operator::{Operator, OperatorInput, OperatorOutput};
use crate::orchestrator::{Orchestrator, QueryPayload};
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
impl Orchestrator for LocalOrchestrator {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .operators
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?;
        op.execute(input, &crate::dispatch::Capabilities::none()).await.map_err(OrchError::OperatorError)
    }

    async fn dispatch_many(
        &self,
        tasks: Vec<(OperatorId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut handles = Vec::with_capacity(tasks.len());

        for (id, input) in tasks {
            match self.operators.get(id.as_str()) {
                Some(operator) => {
                    let operator = Arc::clone(operator);
                    handles.push(tokio::spawn(async move {
                        operator
                            .execute(input, &crate::dispatch::Capabilities::none())
                            .await
                            .map_err(OrchError::OperatorError)
                    }));
                }
                None => {
                    let name = id.to_string();
                    handles.push(tokio::spawn(async move {
                        Err(OrchError::OperatorNotFound(name))
                    }));
                }
            }
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(OrchError::DispatchFailed(e.to_string()))),
            }
        }

        results
    }

    async fn signal(&self, _target: &WorkflowId, _signal: SignalPayload) -> Result<(), OrchError> {
        // LocalOrchestrator doesn't track running workflows
        Ok(())
    }

    async fn query(
        &self,
        _target: &WorkflowId,
        _query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError> {
        // LocalOrchestrator doesn't track running workflows
        Ok(serde_json::Value::Null)
    }
}
