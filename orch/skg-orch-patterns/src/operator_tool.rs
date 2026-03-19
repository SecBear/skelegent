//! [`OperatorTool`] — wraps an [`Operator`] as a [`ToolDyn`] for agent-as-tool composition.
//!
//! Lets one LLM invoke a sub-agent as if it were an ordinary tool call. The
//! tool's `call` implementation dispatches the wrapped operator and surfaces
//! the operator's final text response as the tool result JSON.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use layer0::DispatchContext;
use layer0::content::Content;
use layer0::dispatch::Dispatcher;
use layer0::id::{DispatchId, OperatorId};
use layer0::operator::{OperatorInput, TriggerType};
use skg_tool::{ApprovalPolicy, ToolDyn, ToolError};

/// Monotonic counter used to generate unique child dispatch IDs.
static DISPATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_dispatch_id() -> DispatchId {
    let n = DISPATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    DispatchId::new(format!("op-tool-{n}"))
}

/// A [`ToolDyn`] that dispatches to an [`Operator`] for agent-as-tool composition.
///
/// Exposes the wrapped operator as `ask_{operator_id}` so an LLM can delegate
/// work to a sub-agent through the normal tool-use mechanism. The tool accepts
/// a `message` parameter and returns the operator's text output.
pub struct OperatorTool {
    /// Identifier of the operator to invoke.
    operator_id: OperatorId,
    /// Dispatcher used to route the invocation.
    dispatcher: Arc<dyn Dispatcher>,
    /// Human-readable description shown to the LLM.
    description: String,
    /// Pre-computed tool name: `ask_{operator_id}`.
    name: String,
    /// Pre-computed JSON input schema.
    schema: serde_json::Value,
}

impl OperatorTool {
    /// Create a new operator tool wrapping `operator_id`.
    pub fn new(
        operator_id: OperatorId,
        dispatcher: Arc<dyn Dispatcher>,
        description: impl Into<String>,
    ) -> Self {
        let name = format!("ask_{}", operator_id.as_str());
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        });
        Self {
            operator_id,
            dispatcher,
            description: description.into(),
            name,
            schema,
        }
    }
}

impl ToolDyn for OperatorTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn call(
        &self,
        input: serde_json::Value,
        ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        let dispatcher = Arc::clone(&self.dispatcher);
        let operator_id = self.operator_id.clone();

        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Build a child context targeting the wrapped operator.
        let child_ctx = ctx.child(next_dispatch_id(), operator_id);

        Box::pin(async move {
            let op_input = OperatorInput::new(Content::text(message), TriggerType::Task);

            let output = dispatcher
                .dispatch(&child_ctx, op_input)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
                .collect()
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            // Return the operator's text output as a JSON string value.
            let text = output.message.as_text().unwrap_or("").to_string();
            Ok(serde_json::json!({ "result": text }))
        })
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::None
    }
}
