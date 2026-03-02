#![deny(missing_docs)]
//! In-process implementation of layer0's Orchestrator trait.
//!
//! Dispatches to registered agents via `HashMap<AgentId, Arc<dyn Operator>>`.
//! Concurrent dispatch uses `tokio::spawn`. No durability — operators that fail
//! are not retried and state is not persisted. Workflow `signal` semantics and a
//! minimal `query` are implemented via an in-memory, per-workflow signal journal.

use async_trait::async_trait;
use layer0::effect::SignalPayload;
use layer0::error::OrchError;
use layer0::id::{AgentId, WorkflowId};
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-process orchestrator that dispatches to registered agents.
///
/// Uses `Arc<dyn Operator>` for true concurrent dispatch via `tokio::spawn`.
/// No durability, but tracks workflow signals in-memory for `signal`/`query`.
/// Suitable for development, testing, and single-process deployments.
pub struct LocalOrch {
    agents: HashMap<String, Arc<dyn Operator>>,
    // Per-workflow signal journal
    workflow_signals: RwLock<HashMap<String, Vec<SignalPayload>>>,
    // Minimal query store placeholder for future extensions
    query_store: RwLock<HashMap<String, serde_json::Value>>,
}

impl LocalOrch {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            workflow_signals: RwLock::new(HashMap::new()),
            query_store: RwLock::new(HashMap::new()),
        }
    }

    /// Register an agent with the orchestrator.
    pub fn register(&mut self, id: AgentId, op: Arc<dyn Operator>) {
        self.agents.insert(id.to_string(), op);
    }

    /// Return the number of recorded signals for a workflow.
    pub async fn signal_count(&self, target: &WorkflowId) -> usize {
        let workflows = self.workflow_signals.read().await;
        workflows.get(target.as_str()).map(|v| v.len()).unwrap_or(0)
    }
}

impl Default for LocalOrch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Orchestrator for LocalOrch {
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .agents
            .get(agent.as_str())
            .ok_or_else(|| OrchError::AgentNotFound(agent.to_string()))?;
        op.execute(input).await.map_err(OrchError::OperatorError)
    }

    async fn dispatch_many(
        &self,
        tasks: Vec<(AgentId, OperatorInput)>,
    ) -> Vec<Result<OperatorOutput, OrchError>> {
        let mut handles = Vec::with_capacity(tasks.len());

        for (agent_id, input) in tasks {
            match self.agents.get(agent_id.as_str()) {
                Some(op) => {
                    let op = Arc::clone(op);
                    handles.push(tokio::spawn(async move {
                        op.execute(input).await.map_err(OrchError::OperatorError)
                    }));
                }
                None => {
                    let name = agent_id.to_string();
                    handles.push(tokio::spawn(
                        async move { Err(OrchError::AgentNotFound(name)) },
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
        // Lazily create the journal for unknown workflows and report minimal schema
        let mut workflows = self.workflow_signals.write().await;
        let entry = workflows.entry(target.to_string()).or_default();
        let count = entry.len();
        // Store the last query result for future extension (not used by tests)
        {
            let mut store = self.query_store.write().await;
            store.insert(target.to_string(), json!({ "signals": count }));
        }
        Ok(json!({ "signals": count }))
    }
}
