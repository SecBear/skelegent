use layer0::{ApprovalFacts, ApprovalPolicy, CapabilityKind, ExecutionClass};
use serde_json::json;
use skg_tool::{DispatchContext, ToolConcurrencyHint, ToolDyn, ToolError, tool_descriptor};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct SharedAlwaysTool;

impl ToolDyn for SharedAlwaysTool {
    fn name(&self) -> &str {
        "shared_always"
    }

    fn description(&self) -> &str {
        "Shared tool with always-approval policy"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"q":{"type":"string"}}})
    }

    fn call(
        &self,
        _input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        Box::pin(async { Ok(json!({"ok":true})) })
    }

    fn concurrency_hint(&self) -> ToolConcurrencyHint {
        ToolConcurrencyHint::Shared
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::Always
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}}))
    }
}

struct ExclusiveConditionalTool;

impl ToolDyn for ExclusiveConditionalTool {
    fn name(&self) -> &str {
        "exclusive_conditional"
    }

    fn description(&self) -> &str {
        "Exclusive tool with runtime policy approval"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object"})
    }

    fn call(
        &self,
        _input: serde_json::Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn concurrency_hint(&self) -> ToolConcurrencyHint {
        ToolConcurrencyHint::Exclusive
    }

    fn approval_policy(&self) -> ApprovalPolicy {
        ApprovalPolicy::Conditional(Arc::new(|input| input.get("requires").is_some()))
    }
}

#[test]
fn tool_projection_maps_core_fields() {
    let descriptor = tool_descriptor(&SharedAlwaysTool);
    assert_eq!(descriptor.kind, CapabilityKind::Tool);
    assert_eq!(descriptor.id.as_str(), "shared_always");
    assert_eq!(descriptor.name, "shared_always");
    assert_eq!(
        descriptor.description,
        "Shared tool with always-approval policy"
    );
    assert_eq!(
        descriptor.input_schema,
        Some(json!({"type":"object","properties":{"q":{"type":"string"}}}))
    );
    assert_eq!(
        descriptor.output_schema,
        Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}}))
    );
}

#[test]
fn tool_projection_maps_parallel_and_approval_facts() {
    let shared = tool_descriptor(&SharedAlwaysTool);
    assert_eq!(shared.scheduling.execution_class, ExecutionClass::Shared);
    assert_eq!(shared.approval, ApprovalFacts::Always);

    let exclusive = tool_descriptor(&ExclusiveConditionalTool);
    assert_eq!(
        exclusive.scheduling.execution_class,
        ExecutionClass::Exclusive
    );
    assert_eq!(exclusive.approval, ApprovalFacts::RuntimePolicy);
}
