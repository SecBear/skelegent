//! The ReAct pattern as a composable function.
//!
//! `react_loop()` composes context engine primitives into the standard
//! ReAct (Reasoning + Acting) loop: infer \u2192 dispatch tools \u2192 repeat.
//! It is ~50 lines of composition, not a 3,000-line framework.
//!
//! `react_loop_structured()` extends this with structured output: the model
//! returns validated JSON via a tool call or text response, with automatic
//! retry on validation failure.
use crate::compile::CompileConfig;
use crate::context::Context;
use crate::error::EngineError;
use crate::ops::response::AppendResponse;
use crate::ops::tool::ExecuteTool;
use crate::output::{OutputError, OutputMode, OutputSchema};
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::duration::DurationMs;
use layer0::effect::Effect;
use layer0::operator::{ExitReason, OperatorMetadata, OperatorOutput};
use neuron_tool::{ToolCallContext, ToolDyn, ToolRegistry};
use neuron_turn::infer::InferResponse;
use neuron_turn::provider::Provider;
use neuron_turn::types::{StopReason, ToolSchema};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Predicate for dynamic tool availability.
///
/// Called each turn with the tool and current context. Return `true` to include
/// the tool in this turn's available set, `false` to hide it from the model.
pub type ToolFilter = Arc<dyn Fn(&dyn ToolDyn, &Context) -> bool + Send + Sync>;

/// Configuration for [`react_loop()`].
pub struct ReactLoopConfig {
    /// System prompt.
    pub system_prompt: String,
    /// Model to use.
    pub model: Option<String>,
    /// Max output tokens per inference call.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
    /// Optional per-turn tool filter.
    ///
    /// When set, tools are re-filtered before each inference call. Tools that
    /// fail the predicate are hidden from the model but remain in the registry
    /// for dispatch (in case the model calls a tool that was available in a
    /// previous turn and the response is still in-flight).
    pub tool_filter: Option<ToolFilter>,
}

impl Clone for ReactLoopConfig {
    fn clone(&self) -> Self {
        Self {
            system_prompt: self.system_prompt.clone(),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            tool_filter: self.tool_filter.clone(),
        }
    }
}

impl fmt::Debug for ReactLoopConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReactLoopConfig")
            .field("system_prompt", &self.system_prompt)
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field(
                "tool_filter",
                &self.tool_filter.as_ref().map(|_| "<filter>"),
            )
            .finish()
    }
}

impl ReactLoopConfig {
    /// Build a [`CompileConfig`] from this loop config and tools.
    ///
    /// When a [`tool_filter`](Self::tool_filter) is set, tools are filtered
    /// against the current context state.
    pub fn compile_config(&self, tools: &ToolRegistry, ctx: &Context) -> CompileConfig {
        let schemas = match &self.tool_filter {
            Some(filter) => {
                let filter = Arc::clone(filter);
                tool_schemas_filtered(tools, |tool| filter(tool, ctx))
            }
            None => tool_schemas(tools),
        };
        CompileConfig {
            system: Some(self.system_prompt.clone()),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            tools: schemas,
            extra: serde_json::Value::Null,
        }
    }
}

/// Run the ReAct (Reasoning + Acting) loop.
///
/// This is the ReAct *pattern* expressed as composition of context engine
/// primitives. It is NOT a framework — it’s a function you call. The context
/// engine handles hookability via rules.
///
/// The loop:
/// 1. Compile context → send to provider
/// 2. Append response to context
/// 3. If no tool calls → return (model is done)
/// 4. Check tool approval → if any tool requires approval, exit with
///    [`ExitReason::AwaitingApproval`] and [`Effect::ToolApprovalRequired`]
/// 5. Dispatch each tool call → append results to context
/// 6. Increment turn counter → go to 1
///
/// Budget guards, compaction, telemetry, overwatch — all fire automatically
/// via rules on the context. The loop doesn’t know about them.
///
/// When a [`ToolFilter`] is set on the config, tools are re-filtered before
/// each inference call based on the current context state.
pub async fn react_loop<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    config: &ReactLoopConfig,
) -> Result<OperatorOutput, EngineError> {
    loop {
        // Phase 1: Compile and infer (re-filter tools each turn)
        let compile_config = config.compile_config(tools, ctx);
        let compiled = ctx.compile(&compile_config);
        let result = compiled.infer(provider).await?;

        // Phase 2: Append response to context (this is a context op — rules fire)
        ctx.run(AppendResponse::new(result.response.clone()))
            .await?;

        // Count this inference as a completed turn
        ctx.metrics.turns_completed += 1;

        // Phase 3: Check if model is done
        if !result.has_tool_calls() {
            let exit = match result.response.stop_reason {
                StopReason::ContentFilter => ExitReason::SafetyStop {
                    reason: "content filter triggered".into(),
                },
                _ => ExitReason::Complete,
            };
            return Ok(make_output(result.response, exit, ctx));
        }

        // Phase 4: Check tool approval
        let tool_calls = result.response.tool_calls.clone();
        let needs_approval: Vec<_> = tool_calls
            .iter()
            .filter(|call| tools.get(&call.name).is_some_and(|t| t.requires_approval()))
            .collect();

        if !needs_approval.is_empty() {
            for call in &needs_approval {
                ctx.effects.push(Effect::ToolApprovalRequired {
                    tool_name: call.name.clone(),
                    call_id: call.id.clone(),
                    input: call.input.clone(),
                });
            }
            return Ok(make_output(
                result.response,
                ExitReason::AwaitingApproval,
                ctx,
            ));
        }

        // Phase 5: Dispatch tool calls
        for call in &tool_calls {
            let result_str = match ctx
                .run(ExecuteTool::new(
                    call.clone(),
                    tools.clone(),
                    tool_ctx.clone(),
                ))
                .await
            {
                Ok(s) => s,
                Err(e) => format!("Error: {e}"),
            };

            // Append tool result to context
            let result_msg =
                InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
            ctx.inject_message(result_msg).await?;
        }
    }
}

fn make_output(response: InferResponse, exit: ExitReason, ctx: &Context) -> OperatorOutput {
    let mut output = OperatorOutput::new(response.content, exit);
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = ctx.metrics.tokens_in;
    meta.tokens_out = ctx.metrics.tokens_out;
    meta.cost = ctx.metrics.cost;
    meta.turns_used = ctx.metrics.turns_completed;
    meta.duration = DurationMs::from_millis(ctx.metrics.elapsed_ms());
    output.metadata = meta;
    output.effects = ctx.effects.clone();
    output
}

/// Extract tool schemas from a registry, applying a filter.
fn tool_schemas_filtered(
    registry: &ToolRegistry,
    predicate: impl Fn(&dyn ToolDyn) -> bool,
) -> Vec<ToolSchema> {
    registry
        .iter()
        .filter(|tool| predicate(tool.as_ref()))
        .map(|tool| ToolSchema {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            input_schema: tool.input_schema(),
        })
        .collect()
}

/// Extract tool schemas from a registry.
fn tool_schemas(registry: &ToolRegistry) -> Vec<ToolSchema> {
    registry
        .iter()
        .map(|tool| ToolSchema {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            input_schema: tool.input_schema(),
        })
        .collect()
}

/// Run the ReAct loop with structured output validation.
///
/// Like [`react_loop()`], but the model must return structured output
/// matching the [`OutputSchema`]. The output is extracted and validated
/// after each inference call. On validation failure, the error is sent
/// back to the model for retry (up to [`OutputSchema::max_retries`]).
///
/// In [`OutputMode::ToolCall`] mode, a `return_result` tool is injected
/// into the compile config. The model calls this tool with the structured
/// result. Other function tool calls are dispatched normally.
///
/// In [`OutputMode::TextJson`] mode, the model returns JSON in its text
/// response. Tool calls are dispatched normally; structured output is
/// extracted only when the model returns text without tool calls.
///
/// Returns `(validated_value, operator_output)` on success.
pub async fn react_loop_structured<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    config: &ReactLoopConfig,
    output: &OutputSchema,
) -> Result<(Value, OperatorOutput), EngineError> {
    let output_tool_schema = if output.mode == OutputMode::ToolCall {
        Some(output.tool_schema())
    } else {
        None
    };

    let mut output_retries: u32 = 0;

    loop {
        // Phase 1: Compile and infer (re-filter tools each turn)
        let mut compile_config = config.compile_config(tools, ctx);
        if let Some(schema) = &output_tool_schema {
            compile_config.tools.push(schema.clone());
        }
        let compiled = ctx.compile(&compile_config);
        let result = compiled.infer(provider).await?;

        // Phase 2: Append response to context (rules fire)
        ctx.run(AppendResponse::new(result.response.clone()))
            .await?;
        ctx.metrics.turns_completed += 1;

        // Phase 3: Try to extract structured output
        match output.extract(&result.response) {
            Ok(value) => {
                let op_output = make_output(result.response, ExitReason::Complete, ctx);
                return Ok((value, op_output));
            }
            Err(OutputError::ValidationFailed { message, .. }) => {
                output_retries += 1;
                if output_retries > output.max_retries {
                    return Err(EngineError::Halted {
                        reason: format!(
                            "structured output validation failed after {} retries: {}",
                            output.max_retries, message
                        ),
                    });
                }
                // Send validation error back for retry
                if output.mode == OutputMode::ToolCall {
                    if let Some(call) = result
                        .response
                        .tool_calls
                        .iter()
                        .find(|c| c.name == output.tool_name)
                    {
                        let error_msg = InferResponse::tool_result_message(
                            &call.id,
                            &call.name,
                            format!("Validation error: {message}. Please fix and try again."),
                            true,
                        );
                        ctx.inject_message(error_msg).await?;
                    }
                } else {
                    let retry_msg = Message::new(
                        Role::User,
                        Content::text(format!(
                            "Your JSON output failed validation: {message}. Please output valid JSON."
                        )),
                    );
                    ctx.inject_message(retry_msg).await?;
                }
                // Dispatch any non-output tool calls in the same response
                let awaiting = dispatch_function_tools(
                    ctx,
                    &result.response,
                    tools,
                    tool_ctx,
                    &output.tool_name,
                )
                .await?;
                if awaiting {
                    return Err(EngineError::Halted {
                        reason: "tool approval required during structured output loop".into(),
                    });
                }
                continue;
            }
            Err(OutputError::NoOutput) => {
                // No structured output — check for function tool calls
                if result.has_tool_calls() {
                    let awaiting = dispatch_function_tools(
                        ctx,
                        &result.response,
                        tools,
                        tool_ctx,
                        &output.tool_name,
                    )
                    .await?;
                    if awaiting {
                        return Err(EngineError::Halted {
                            reason: "tool approval required during structured output loop".into(),
                        });
                    }
                    continue;
                }
                // No tool calls, no structured output — model is done without output
                let exit = match result.response.stop_reason {
                    StopReason::ContentFilter => ExitReason::SafetyStop {
                        reason: "content filter triggered".into(),
                    },
                    _ => ExitReason::Complete,
                };
                return Err(EngineError::Halted {
                    reason: format!(
                        "model completed without producing structured output (exit: {exit:?})"
                    ),
                });
            }
        }
    }
}

/// Dispatch function tool calls, skipping the output tool.
///
/// If any tool requires approval, emits [`Effect::ToolApprovalRequired`]
/// effects on the context and returns `true` (caller should exit with
/// [`ExitReason::AwaitingApproval`]).
async fn dispatch_function_tools(
    ctx: &mut Context,
    response: &InferResponse,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    output_tool_name: &str,
) -> Result<bool, EngineError> {
    // Check for approval-required tools first
    let needs_approval: Vec<_> = response
        .tool_calls
        .iter()
        .filter(|call| call.name != output_tool_name)
        .filter(|call| tools.get(&call.name).is_some_and(|t| t.requires_approval()))
        .collect();

    if !needs_approval.is_empty() {
        for call in &needs_approval {
            ctx.effects.push(Effect::ToolApprovalRequired {
                tool_name: call.name.clone(),
                call_id: call.id.clone(),
                input: call.input.clone(),
            });
        }
        return Ok(true); // Caller should exit with AwaitingApproval
    }

    for call in &response.tool_calls {
        if call.name == output_tool_name {
            continue;
        }
        let result_str = match ctx
            .run(ExecuteTool::new(
                call.clone(),
                tools.clone(),
                tool_ctx.clone(),
            ))
            .await
        {
            Ok(s) => s,
            Err(e) => format!("Error: {e}"),
        };
        let result_msg =
            InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
        ctx.inject_message(result_msg).await?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputSchema;
    use layer0::id::AgentId;
    use neuron_tool::{ToolDyn, ToolError};
    use neuron_turn::test_utils::TestProvider;
    use serde_json::json;
    use std::pin::Pin;
    use std::sync::Arc;

    struct MockTool {
        name: &'static str,
    }

    impl ToolDyn for MockTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "mock tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCallContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Ok(json!("mock result")) })
        }
    }

    fn simple_config() -> ReactLoopConfig {
        ReactLoopConfig {
            system_prompt: "You are helpful.".into(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: None,
        }
    }

    fn city_validator(v: &Value) -> Result<Value, String> {
        if v.get("name").and_then(|n| n.as_str()).is_none() {
            return Err("missing 'name'".into());
        }
        if v.get("population").and_then(|p| p.as_u64()).is_none() {
            return Err("missing 'population'".into());
        }
        Ok(v.clone())
    }

    #[tokio::test]
    async fn structured_tool_call_success_first_try() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call(
            "return_result",
            "call_1",
            json!({ "result": { "name": "Tokyo", "population": 13960000_u64 } }),
        );

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Tokyo?")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, output) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        assert_eq!(value["name"], "Tokyo");
        assert_eq!(value["population"], 13960000_u64);
        assert!(matches!(output.exit_reason, ExitReason::Complete));
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn structured_tool_call_retry_then_success() {
        let provider = TestProvider::new();
        // First response: invalid (missing population)
        provider.respond_with_tool_call(
            "return_result",
            "call_1",
            json!({ "result": { "name": "Tokyo" } }),
        );
        // Second response: valid
        provider.respond_with_tool_call(
            "return_result",
            "call_2",
            json!({ "result": { "name": "Tokyo", "population": 13960000_u64 } }),
        );

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Tokyo?")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        assert_eq!(value["name"], "Tokyo");
        assert_eq!(value["population"], 13960000_u64);
        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn structured_tool_call_exceeds_retries() {
        let provider = TestProvider::new();
        // 4 invalid responses (max_retries = 3 means 4 total attempts)
        for i in 0..4 {
            provider.respond_with_tool_call(
                "return_result",
                &format!("call_{i}"),
                json!({ "result": { "name": "Tokyo" } }),
            );
        }

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Tokyo?")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let err = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, EngineError::Halted { .. }));
        assert!(
            err.to_string()
                .contains("validation failed after 3 retries")
        );
    }

    #[tokio::test]
    async fn structured_text_json_success() {
        let provider = TestProvider::new();
        provider.respond_with_text(r#"{"name": "Berlin", "population": 3645000}"#);

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Berlin?")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::text_json(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        assert_eq!(value["name"], "Berlin");
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn structured_model_completes_without_output() {
        let provider = TestProvider::new();
        provider.respond_with_text("I don't know the answer.");

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Tokyo?")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        // ToolCall mode: model returns text instead of calling return_result
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let err = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, EngineError::Halted { .. }));
        assert!(
            err.to_string()
                .contains("without producing structured output")
        );
    }

    #[tokio::test]
    async fn structured_function_tools_then_output() {
        let provider = TestProvider::new();
        // First: model calls a function tool
        provider.respond_with_tool_call("search", "call_1", json!({ "query": "Tokyo" }));
        // Second: model returns structured output
        provider.respond_with_tool_call(
            "return_result",
            "call_2",
            json!({ "result": { "name": "Tokyo", "population": 13960000_u64 } }),
        );

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("What is Tokyo?")))
            .await
            .unwrap();

        // Register a simple search tool
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "search" }));

        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        assert_eq!(value["name"], "Tokyo");
        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn structured_injects_output_tool_in_compile_config() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call(
            "return_result",
            "call_1",
            json!({ "result": { "ok": true } }),
        );

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("test")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), |v| Ok(v.clone()));

        let _ = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        // Verify the request included the return_result tool
        let request = provider.last_request().unwrap();
        assert!(request.tools.iter().any(|t| t.name == "return_result"));
    }

    #[tokio::test]
    async fn structured_text_json_does_not_inject_tool() {
        let provider = TestProvider::new();
        provider.respond_with_text(r#"{"ok": true}"#);

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("test")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let schema = OutputSchema::text_json(json!({}), |v| Ok(v.clone()));

        let _ = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        // Verify NO return_result tool in the request
        let request = provider.last_request().unwrap();
        assert!(!request.tools.iter().any(|t| t.name == "return_result"));
    }

    // === Dynamic tool filtering tests ===

    #[test]
    fn tool_filter_excludes_tools_from_schemas() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "allowed" }));
        tools.register(Arc::new(MockTool { name: "blocked" }));

        let config = ReactLoopConfig {
            system_prompt: "test".into(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: Some(Arc::new(|tool: &dyn ToolDyn, _ctx: &Context| {
                tool.name() != "blocked"
            })),
        };

        let ctx = Context::new();
        let compile_config = config.compile_config(&tools, &ctx);

        assert_eq!(compile_config.tools.len(), 1);
        assert_eq!(compile_config.tools[0].name, "allowed");
    }

    #[test]
    fn no_tool_filter_includes_all() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "a" }));
        tools.register(Arc::new(MockTool { name: "b" }));

        let config = simple_config();
        let ctx = Context::new();
        let compile_config = config.compile_config(&tools, &ctx);

        assert_eq!(compile_config.tools.len(), 2);
    }

    #[test]
    fn tool_registry_filtered() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "keep" }));
        tools.register(Arc::new(MockTool { name: "drop" }));

        let filtered = tools.filtered(|t| t.name() == "keep");
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get("keep").is_some());
        assert!(filtered.get("drop").is_none());
    }

    #[tokio::test]
    async fn react_loop_with_tool_filter() {
        let provider = TestProvider::new();
        // Model sees only "allowed" tool, calls it
        provider.respond_with_tool_call("allowed", "c1", json!({ "x": 1 }));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "allowed" }));
        tools.register(Arc::new(MockTool { name: "blocked" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let tool_ctx = ToolCallContext::new(AgentId::from("test"));
        let config = ReactLoopConfig {
            system_prompt: "test".into(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: Some(Arc::new(|tool: &dyn ToolDyn, _ctx: &Context| {
                tool.name() != "blocked"
            })),
        };

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config)
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        // Verify only "allowed" tool was in the request schemas
        let request = provider.last_request().unwrap();
        let tool_names: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"allowed"));
        assert!(!tool_names.contains(&"blocked"));
    }

    // === Tool approval tests ===

    struct ApprovalTool;

    impl ToolDyn for ApprovalTool {
        fn name(&self) -> &str {
            "dangerous_tool"
        }
        fn description(&self) -> &str {
            "requires approval"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCallContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Ok(json!("should not reach here")) })
        }
        fn requires_approval(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn react_loop_exits_on_approval_required() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("dangerous_tool", "c1", json!({ "cmd": "rm -rf /" }));

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ApprovalTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("delete everything")))
            .await
            .unwrap();

        let tool_ctx = ToolCallContext::new(AgentId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        // Should exit with AwaitingApproval
        assert_eq!(output.exit_reason, ExitReason::AwaitingApproval);

        // Should have the ToolApprovalRequired effect
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::ToolApprovalRequired {
                tool_name,
                call_id,
                input,
            } => {
                assert_eq!(tool_name, "dangerous_tool");
                assert_eq!(call_id, "c1");
                assert_eq!(input, &json!({ "cmd": "rm -rf /" }));
            }
            other => panic!("expected ToolApprovalRequired, got {other:?}"),
        }

        // Provider should have been called exactly once
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn react_loop_safe_tools_execute_normally() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("safe_tool", "c1", json!({ "x": 1 }));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "safe_tool" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let tool_ctx = ToolCallContext::new(AgentId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        // Normal completion — requires_approval defaults to false
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert!(output.effects.is_empty());
    }

    #[tokio::test]
    async fn react_loop_mixed_approval_and_safe_tools() {
        let provider = TestProvider::new();
        // Model calls both a safe tool and a dangerous tool
        provider.respond_with_tool_calls(vec![
            ("safe_tool", "c1", json!({ "x": 1 })),
            ("dangerous_tool", "c2", json!({ "cmd": "deploy" })),
        ]);

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "safe_tool" }));
        tools.register(Arc::new(ApprovalTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let tool_ctx = ToolCallContext::new(AgentId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        // Should exit with AwaitingApproval (approval check happens before dispatch)
        assert_eq!(output.exit_reason, ExitReason::AwaitingApproval);
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::ToolApprovalRequired { tool_name, .. } => {
                assert_eq!(tool_name, "dangerous_tool");
            }
            other => panic!("expected ToolApprovalRequired, got {other:?}"),
        }
    }
}
