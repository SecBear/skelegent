//! The ReAct pattern as a composable function.
//!
//! `react_loop()` composes context engine primitives into the standard
//! ReAct (Reasoning + Acting) loop: infer → dispatch tools → repeat.
//! It is ~50 lines of composition, not a 3,000-line framework.

use crate::compile::CompileConfig;
use crate::context::Context;
use crate::error::EngineError;
use crate::ops::response::AppendResponse;
use crate::ops::tool::ExecuteTool;
use layer0::duration::DurationMs;
use layer0::operator::{ExitReason, OperatorMetadata, OperatorOutput};
use neuron_tool::{ToolCallContext, ToolRegistry};
use neuron_turn::infer::InferResponse;
use neuron_turn::provider::Provider;
use neuron_turn::types::{StopReason, ToolSchema};

/// Configuration for [`react_loop()`].
#[derive(Debug, Clone)]
pub struct ReactLoopConfig {
    /// System prompt.
    pub system_prompt: String,
    /// Model to use.
    pub model: Option<String>,
    /// Max output tokens per inference call.
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f64>,
}

impl ReactLoopConfig {
    /// Build a [`CompileConfig`] from this loop config and tools.
    pub fn compile_config(&self, tools: &ToolRegistry) -> CompileConfig {
        CompileConfig {
            system: Some(self.system_prompt.clone()),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            tools: tool_schemas(tools),
            extra: serde_json::Value::Null,
        }
    }
}

/// Run the ReAct (Reasoning + Acting) loop.
///
/// This is the ReAct *pattern* expressed as ~50 lines composing context engine
/// primitives. It is NOT a framework — it's a function you call. The context
/// engine handles hookability via rules.
///
/// The loop:
/// 1. Compile context → send to provider
/// 2. Append response to context
/// 3. If no tool calls → return (model is done)
/// 4. Dispatch each tool call → append results to context
/// 5. Increment turn counter → go to 1
///
/// Budget guards, compaction, telemetry, overwatch — all fire automatically
/// via rules on the context. The loop doesn't know about them.
pub async fn react_loop<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    config: &ReactLoopConfig,
) -> Result<OperatorOutput, EngineError> {
    let compile_config = config.compile_config(tools);

    loop {
        // Phase 1: Compile and infer
        let compiled = ctx.compile(&compile_config);
        let result = compiled.infer(provider).await?;

        // Phase 2: Append response to context (this is a context op — rules fire)
        ctx.run(AppendResponse::new(result.response.clone()))
            .await?;

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

        // Phase 4: Dispatch tool calls
        let tool_calls = result.response.tool_calls.clone();
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

        // Phase 5: Increment turn counter
        ctx.metrics.turns_completed += 1;
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
