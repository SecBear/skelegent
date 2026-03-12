//! The ReAct pattern as a composable function.
//!
//! `react_loop()` composes context engine primitives into the standard
//! ReAct (Reasoning + Acting) loop: infer \u2192 dispatch tools \u2192 repeat.
//! It is ~50 lines of composition, not a 3,000-line framework.
//!
//! `react_loop_structured()` extends this with structured output: the model
//! returns validated JSON via a tool call or text response, with automatic
//! retry on validation failure.
use crate::boundary::InferBoundary;
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
use serde_json::Value;
use skg_tool::{ToolCallContext, ToolDyn, ToolRegistry};
use skg_turn::infer::{InferResponse, ToolCall};
use skg_turn::provider::Provider;
use skg_turn::types::{StopReason, ToolSchema};
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

/// Map a provider stop reason to an operator exit reason.
///
/// This is the decision point for determining why an agent loop ended.
/// The default mapping used by [`react_loop()`]:
/// - `StopReason::ContentFilter` → `ExitReason::SafetyStop`
/// - Everything else → `ExitReason::Complete`
///
/// Override this by writing your own loop and using a different mapping.
pub fn check_exit(stop_reason: &StopReason) -> ExitReason {
    match stop_reason {
        StopReason::ContentFilter => ExitReason::SafetyStop {
            reason: "content filter triggered".into(),
        },
        _ => ExitReason::Complete,
    }
}

/// Check which tool calls require approval and return the corresponding effects.
///
/// Returns a vec of `Effect::ToolApprovalRequired` for each tool call where
/// `tool.requires_approval()` is true. Returns an empty vec if no tools
/// require approval.
///
/// This is the decision point for human-in-the-loop approval. The caller
/// decides what to do with the effects (emit them, filter them, etc.).
pub fn check_approval(tool_calls: &[ToolCall], registry: &ToolRegistry) -> Vec<Effect> {
    tool_calls
        .iter()
        .filter(|call| {
            registry
                .get(&call.name)
                .is_some_and(|t| t.requires_approval())
        })
        .map(|call| Effect::ToolApprovalRequired {
            tool_name: call.name.clone(),
            call_id: call.id.clone(),
            input: call.input.clone(),
        })
        .collect()
}

/// Format a tool execution error as a string for the model.
///
/// Default: `format!("Error: {e}")`. This is the formatting used by
/// [`react_loop()`] when a tool call fails.
///
/// Override this by writing your own dispatch loop and formatting errors yourself.
pub fn format_tool_error(e: &EngineError) -> String {
    format!("Error: {e}")
}

async fn infer_once<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    config: &ReactLoopConfig,
    extra_tool: Option<&ToolSchema>,
) -> Result<crate::InferResult, EngineError> {
    ctx.enter_boundary::<InferBoundary>().await?;

    let mut compile_config = config.compile_config(tools, ctx);
    if let Some(schema) = extra_tool {
        compile_config.tools.push(schema.clone());
    }

    let compiled = ctx.compile(&compile_config);
    let result = compiled.infer(provider).await?;

    ctx.exit_boundary::<InferBoundary>().await?;
    Ok(result)
}

fn structured_exit_output(err: EngineError, ctx: &Context) -> Result<OperatorOutput, EngineError> {
    match err {
        EngineError::Exit { reason, .. } => Ok(make_context_output(Content::text(""), reason, ctx)),
        other => Err(other),
    }
}

fn make_context_output(message: Content, exit: ExitReason, ctx: &Context) -> OperatorOutput {
    let mut output = OperatorOutput::new(message, exit);
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = ctx.metrics.tokens_in;
    meta.tokens_out = ctx.metrics.tokens_out;
    meta.cost = ctx.metrics.cost;
    meta.turns_used = ctx.metrics.turns_completed;
    meta.duration = DurationMs::from_millis(ctx.metrics.elapsed_ms());
    output.metadata = meta;
    output.effects = ctx.effects().to_vec();
    output
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
        let result = match infer_once(ctx, provider, tools, config, None).await {
            Ok(result) => result,
            Err(err) => return structured_exit_output(err, ctx),
        };

        // Phase 2: Append response to context (this is a context op — rules fire)
        if let Err(err) = ctx.run(AppendResponse::new(result.response.clone())).await {
            return structured_exit_output(err, ctx);
        }

        // Count this inference as a completed turn
        ctx.metrics.turns_completed += 1;

        // Phase 3: Check if model is done
        if !result.has_tool_calls() {
            let exit = check_exit(&result.response.stop_reason);
            return Ok(make_output(result.response, exit, ctx));
        }

        // Phase 4: Check tool approval
        let tool_calls = result.response.tool_calls.clone();
        let approval_effects = check_approval(&tool_calls, tools);

        if !approval_effects.is_empty() {
            ctx.extend_effects(approval_effects);
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
                Err(EngineError::Exit { reason, .. }) => {
                    return Ok(make_context_output(Content::text(""), reason, ctx));
                }
                Err(e) => format_tool_error(&e),
            };

            // Append tool result to context
            let result_msg =
                InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
            if let Err(err) = ctx.inject_message(result_msg).await {
                return structured_exit_output(err, ctx);
            }
        }
    }
}

fn make_output(response: InferResponse, exit: ExitReason, ctx: &Context) -> OperatorOutput {
    make_context_output(response.content, exit, ctx)
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
        let result = infer_once(
            ctx,
            provider,
            tools,
            config,
            output_tool_schema.as_ref(),
        )
        .await?;

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
                let exit = check_exit(&result.response.stop_reason);
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
    // Check for approval-required tools first (excluding output tool)
    let function_calls: Vec<_> = response
        .tool_calls
        .iter()
        .filter(|call| call.name != output_tool_name)
        .cloned()
        .collect();
    let approval_effects = check_approval(&function_calls, tools);

    if !approval_effects.is_empty() {
        ctx.extend_effects(approval_effects);
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
            Err(e) => format_tool_error(&e),
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
    use crate::op::ContextOp;
    use crate::output::OutputSchema;
    use crate::rules::{BudgetGuard, BudgetGuardConfig};
    use crate::{InferBoundary, Rule};
    use async_trait::async_trait;
    use layer0::id::OperatorId;
    use serde_json::json;
    use skg_tool::{ToolDyn, ToolError};
    use skg_turn::provider::ProviderError;
    use skg_turn::test_utils::{TestProvider, error_provider_transient};
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

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

    struct HaltBeforeInference;

    #[async_trait]
    impl ContextOp for HaltBeforeInference {
        type Output = ();

        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Err(EngineError::Halted {
                reason: "blocked before inference".into(),
            })
        }
    }

    struct InjectInterventionMessage(&'static str);

    #[async_trait]
    impl ContextOp for InjectInterventionMessage {
        type Output = ();

        async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
            ctx.push_message(Message::new(Role::System, Content::text(self.0)));
            Ok(())
        }
    }

    struct PushMarker(&'static str);

    #[async_trait]
    impl ContextOp for PushMarker {
        type Output = ();

        async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
            ctx.push_message(Message::new(Role::System, Content::text(self.0)));
            Ok(())
        }
    }

    #[tokio::test]
    async fn react_loop_before_infer_boundary_rule_mutates_request_before_provider_call() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
            "mark before inference",
            100,
            PushMarker("before infer marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let request = provider.last_request().expect("provider should record request");
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.text_content() == "before infer marker"),
            "before rule must mutate context before request compilation and provider send"
        );
    }

    #[tokio::test]
    async fn react_loop_after_infer_boundary_rule_runs_after_success_only() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::after::<InferBoundary>(
            "mark after inference",
            100,
            PushMarker("after infer marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let request = provider.last_request().expect("provider should record request");
        assert!(
            !request
                .messages
                .iter()
                .any(|message| message.text_content() == "after infer marker"),
            "after rule must not mutate the request that was already sent to the provider"
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after infer marker"),
            "after rule must run after a successful provider call"
        );
    }

    #[tokio::test]
    async fn react_loop_after_infer_boundary_rule_does_not_run_on_provider_error() {
        let provider = error_provider_transient("boom");

        let mut ctx = Context::with_rules(vec![Rule::after::<InferBoundary>(
            "mark after inference",
            100,
            PushMarker("after infer marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let err = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap_err();

        assert!(matches!(err, EngineError::Provider(ProviderError::TransientError { .. })));
        assert!(
            !ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after infer marker"),
            "after rule must not run when provider inference fails"
        );
    }

    #[tokio::test]
    async fn react_loop_halts_before_provider_call_on_infer_boundary_rule() {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
            "halt before inference",
            100,
            HaltBeforeInference,
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let err = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap_err();

        assert!(matches!(err, EngineError::Halted { .. }));
        assert_eq!(provider.call_count(), 0);
    }

    async fn assert_budget_exit_before_provider_call(
        mutate_ctx: impl FnOnce(&mut Context),
        config: BudgetGuardConfig,
        expected_exit: ExitReason,
    ) {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
            "budget_guard",
            100,
            BudgetGuard::with_config(config),
        )]);
        mutate_ctx(&mut ctx);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, expected_exit);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn react_loop_returns_structured_max_turns_exit_before_provider_call() {
        assert_budget_exit_before_provider_call(
            |ctx| ctx.metrics.turns_completed = 1,
            BudgetGuardConfig {
                max_cost: None,
                max_turns: Some(1),
                max_duration: None,
                max_tool_calls: None,
            },
            ExitReason::MaxTurns,
        )
        .await;
    }

    #[tokio::test]
    async fn react_loop_returns_structured_budget_exhausted_exit_before_provider_call() {
        assert_budget_exit_before_provider_call(
            |ctx| ctx.metrics.cost = rust_decimal::Decimal::new(250, 2),
            BudgetGuardConfig {
                max_cost: Some(rust_decimal::Decimal::new(100, 2)),
                max_turns: None,
                max_duration: None,
                max_tool_calls: None,
            },
            ExitReason::BudgetExhausted,
        )
        .await;
    }

    #[tokio::test]
    async fn react_loop_returns_structured_timeout_exit_before_provider_call() {
        assert_budget_exit_before_provider_call(
            |ctx| ctx.metrics.start = Instant::now() - Duration::from_secs(5),
            BudgetGuardConfig {
                max_cost: None,
                max_turns: None,
                max_duration: Some(Duration::from_secs(1)),
                max_tool_calls: None,
            },
            ExitReason::Timeout,
        )
        .await;
    }


    #[tokio::test]
    async fn intervention_updates_context_before_provider_sees_next_inference() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let (itx, irx) = mpsc::channel::<Box<dyn crate::op::ErasedOp>>(4);
        let mut ctx = Context::new();
        ctx.with_intervention(irx);
        ctx.inject_message(Message::new(Role::User, Content::text("original")))
            .await
            .unwrap();

        itx.send(Box::new(InjectInterventionMessage("supervisor note")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(provider.call_count(), 1);

        let request = provider.last_request().expect("provider should record request");
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.text_content() == "supervisor note"),
            "intervention message must be present in the compiled request"
        );
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));
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

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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

    #[test]
    fn test_check_exit_content_filter() {
        let exit = check_exit(&StopReason::ContentFilter);
        match exit {
            ExitReason::SafetyStop { reason } => {
                assert_eq!(reason, "content filter triggered");
            }
            other => panic!("expected SafetyStop, got {other:?}"),
        }
    }

    #[test]
    fn test_check_exit_normal() {
        let exit = check_exit(&StopReason::EndTurn);
        assert_eq!(exit, ExitReason::Complete);
    }

    #[test]
    fn test_check_approval_none_required() {
        let tools = ToolRegistry::new();
        let tool_calls: Vec<skg_turn::infer::ToolCall> = vec![];
        let effects = check_approval(&tool_calls, &tools);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_check_approval_some_required() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ApprovalTool));
        tools.register(Arc::new(MockTool { name: "safe" }));

        let tool_calls = vec![
            skg_turn::infer::ToolCall {
                id: "c1".into(),
                name: "dangerous_tool".into(),
                input: json!({ "x": 1 }),
            },
            skg_turn::infer::ToolCall {
                id: "c2".into(),
                name: "safe".into(),
                input: json!({ "y": 2 }),
            },
        ];

        let effects = check_approval(&tool_calls, &tools);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ToolApprovalRequired {
                tool_name,
                call_id,
                input,
            } => {
                assert_eq!(tool_name, "dangerous_tool");
                assert_eq!(call_id, "c1");
                assert_eq!(input, &json!({ "x": 1 }));
            }
            other => panic!("expected ToolApprovalRequired, got {other:?}"),
        }
    }

    #[test]
    fn test_format_tool_error() {
        let err = EngineError::Halted {
            reason: "something broke".into(),
        };
        let formatted = format_tool_error(&err);
        assert!(formatted.starts_with("Error: "));
        assert!(formatted.contains("something broke"));
    }
}
