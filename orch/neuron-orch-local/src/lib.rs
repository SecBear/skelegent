#![deny(missing_docs)]
//! In-process implementation of layer0's Orchestrator trait.
//!
//! Dispatches to registered operators via `HashMap<OperatorId, Arc<dyn Operator>>`.
//! Concurrent dispatch uses `tokio::spawn`. No durability — operators that fail
//! are not retried and state is not persisted. Workflow `signal` semantics and a
//! minimal `query` are implemented via an in-memory, per-workflow signal journal.

use async_trait::async_trait;
use layer0::effect::SignalPayload;
use layer0::error::OrchError;
use layer0::id::{OperatorId, WorkflowId};
use layer0::middleware::{DispatchNext, DispatchStack};
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-process orchestrator that dispatches to registered operators.
///
/// Uses `Arc<dyn Operator>` for true concurrent dispatch via `tokio::spawn`.
/// No durability, but tracks workflow signals in-memory for `signal`/`query`.
/// Suitable for development, testing, and single-process deployments.
pub struct LocalOrch {
    agents: HashMap<String, Arc<dyn Operator>>,
    // Per-workflow signal journal
    workflow_signals: RwLock<HashMap<String, Vec<SignalPayload>>>,
    /// Optional middleware stack for Pre/PostDispatch interception.
    middleware: Option<DispatchStack>,
}

impl LocalOrch {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            workflow_signals: RwLock::new(HashMap::new()),
            middleware: None,
        }
    }

    /// Register an operator with the orchestrator.
    pub fn register(&mut self, id: OperatorId, op: Arc<dyn Operator>) {
        self.agents.insert(id.to_string(), op);
    }

    /// Return the number of recorded signals for a workflow.
    pub async fn signal_count(&self, target: &WorkflowId) -> usize {
        let workflows = self.workflow_signals.read().await;
        workflows.get(target.as_str()).map(|v| v.len()).unwrap_or(0)
    }

    /// Attach a middleware stack for dispatch interception.
    pub fn with_middleware(mut self, stack: DispatchStack) -> Self {
        self.middleware = Some(stack);
        self
    }
}

impl Default for LocalOrch {
    fn default() -> Self {
        Self::new()
    }
}

/// Terminal dispatch: looks up the operator and calls `execute()`.
struct OperatorDispatch<'a> {
    agents: &'a HashMap<String, Arc<dyn Operator>>,
}

#[async_trait]
impl DispatchNext for OperatorDispatch<'_> {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .agents
            .get(operator.as_str())
            .ok_or_else(|| OrchError::OperatorNotFound(operator.to_string()))?;
        op.execute(input).await.map_err(OrchError::OperatorError)
    }
}

#[async_trait]
impl Orchestrator for LocalOrch {
    #[tracing::instrument(skip_all, fields(operator_id = %operator))]
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let terminal = OperatorDispatch {
            agents: &self.agents,
        };

        if let Some(ref stack) = self.middleware {
            stack.dispatch_with(operator, input, &terminal).await
        } else {
            terminal.dispatch(operator, input).await
        }
    }

    #[tracing::instrument(skip_all, fields(count = tasks.len()))]
    async fn dispatch_many(
        &self,
        tasks: Vec<(OperatorId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut handles = Vec::with_capacity(tasks.len());

        for (operator_id, input) in tasks {
            match self.agents.get(operator_id.as_str()) {
                Some(op) => {
                    let op = Arc::clone(op);
                    handles.push(tokio::spawn(async move {
                        op.execute(input).await.map_err(OrchError::OperatorError)
                    }));
                }
                None => {
                    let name = operator_id.to_string();
                    handles.push(tokio::spawn(
                        async move { Err(OrchError::OperatorNotFound(name)) },
                    ));
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

    async fn signal(&self, target: &WorkflowId, signal: SignalPayload) -> Result<(), OrchError> {
        let mut workflows = self.workflow_signals.write().await;
        workflows
            .entry(target.to_string())
            .or_default()
            .push(signal);
        Ok(())
    }

    async fn query(
        &self,
        target: &WorkflowId,
        _query: QueryPayload,
    ) -> Result<serde_json::Value, OrchError> {
        let workflows = self.workflow_signals.read().await;
        let count = workflows.get(target.as_str()).map(|v| v.len()).unwrap_or(0);
        Ok(json!({ "signals": count }))
    }
}
