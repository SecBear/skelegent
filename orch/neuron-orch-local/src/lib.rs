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
use layer0::hook::{HookAction, HookContext, HookPayload, HookPoint};
use layer0::id::{AgentId, WorkflowId};
use layer0::operator::{Operator, OperatorInput, OperatorOutput};
use layer0::orchestrator::{Orchestrator, QueryPayload};
use neuron_hooks::HookRegistry;
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
    /// Optional hook registry for Pre/PostDispatch events.
    hooks: Option<Arc<HookRegistry>>,
}

impl LocalOrch {
    /// Create a new empty orchestrator.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            workflow_signals: RwLock::new(HashMap::new()),
            hooks: None,
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

    /// Attach a hook registry for Pre/PostDispatch events.
    pub fn with_hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.hooks = Some(hooks);
        self
    }
}

impl Default for LocalOrch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Orchestrator for LocalOrch {
    #[tracing::instrument(skip_all, fields(agent_id = %agent))]
    async fn dispatch(
        &self,
        agent: &AgentId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        let op = self
            .agents
            .get(agent.as_str())
            .ok_or_else(|| OrchError::AgentNotFound(agent.to_string()))?;

        if let Some(ref hooks) = self.hooks {
            let mut ctx = HookContext::new(HookPoint::PreDispatch);
            ctx.agent_id = Some(agent.clone());
            ctx.payload = Some(HookPayload::Dispatch {
                agent_id: agent.to_string(),
                input: serde_json::to_value(&input).ok(),
                output: None,
            });
            if let HookAction::Halt { reason } = hooks.dispatch(&ctx).await {
                return Err(OrchError::DispatchFailed(format!(
                    "halted by hook: {reason}"
                )));
            }
        }

        let result = op.execute(input).await.map_err(OrchError::OperatorError)?;

        if let Some(ref hooks) = self.hooks {
            let mut ctx = HookContext::new(HookPoint::PostDispatch);
            ctx.agent_id = Some(agent.clone());
            ctx.payload = Some(HookPayload::Dispatch {
                agent_id: agent.to_string(),
                input: None,
                output: serde_json::to_value(&result).ok(),
            });
            hooks.dispatch(&ctx).await;
        }

        Ok(result)
    }

    #[tracing::instrument(skip_all, fields(count = tasks.len()))]
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
        let workflows = self.workflow_signals.read().await;
        let count = workflows.get(target.as_str()).map(|v| v.len()).unwrap_or(0);
        Ok(json!({ "signals": count }))
    }
}
