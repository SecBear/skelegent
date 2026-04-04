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
use crate::ops::tool::{ExecuteTool, format_tool_result};
use crate::output::{OutputError, OutputMode, OutputSchema};
use layer0::DispatchContext;
use layer0::approval::{ApprovalResponse, ToolCallAction};
use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::dispatch::Dispatcher;
use layer0::duration::DurationMs;
use layer0::id::OperatorId;
use layer0::intent::{HandoffContext, Intent, IntentKind};
use layer0::operator::{InterceptionKind, Outcome, TerminalOutcome, TransferOutcome};
#[cfg(test)]
use layer0::operator::LimitReason;
use layer0::operator::{OperatorMetadata, OperatorOutput};
use layer0::wait::WaitReason;
use serde_json::Value;
use skg_tool::{ToolConcurrencyHint, ToolDyn, ToolError, ToolRegistry};
use skg_turn::infer::{InferResponse, ToolCall};
use skg_turn::provider::Provider;
use skg_turn::types::{StopReason, ToolSchema};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Predicate for dynamic tool availability.
///
/// Called each turn with the tool and current context. Return `true` to include
/// the tool in this turn's available set, `false` to hide it from the model.
pub type ToolFilter = Arc<dyn Fn(&dyn ToolDyn, &Context) -> bool + Send + Sync>;

/// Formatter for tool results: receives `(tool_name, raw_output_value)` and returns the string
/// to inject into LLM context.
pub type ToolResultFormatter = Arc<dyn Fn(&str, &serde_json::Value) -> String + Send + Sync>;

/// Formatter for tool errors: receives `(tool_name, error_message)` and returns the string
/// to inject into LLM context.
pub type ToolErrorFormatter = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;

/// Dynamic system prompt resolver.
///
/// Called at the start of each dispatch with the [`DispatchContext`].
/// The returned string becomes the system prompt for that invocation.
/// Takes precedence over [`ReactLoopConfig::system_prompt`] when set.
pub type SystemPromptFn = Arc<dyn Fn(&DispatchContext) -> String + Send + Sync>;

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
    /// Optional custom formatter for tool results.
    ///
    /// Receives the tool name and the tool's raw output value.
    /// Returns the string to inject into the LLM context.
    /// Defaults to the built-in [`format_tool_result`] behavior when `None`.
    pub tool_result_formatter: Option<ToolResultFormatter>,
    /// Optional custom formatter for tool errors.
    ///
    /// Receives the tool name and the error message string.
    /// Returns the string to inject into the LLM context.
    /// Defaults to the built-in [`format_tool_error`] behavior when `None`.
    pub tool_error_formatter: Option<ToolErrorFormatter>,
    /// Optional dynamic system prompt resolver.
    ///
    /// When set, called at the start of each dispatch to generate the system prompt.
    /// Takes precedence over the static [`system_prompt`](Self::system_prompt) field.
    pub system_prompt_fn: Option<SystemPromptFn>,
    /// Maximum number of times to retry a tool call on [`ToolError::InvalidInput`].
    ///
    /// When a tool rejects its input, the error message and the tool's expected schema
    /// are fed back to the model as a structured retry message so it can correct the call.
    /// Each call ID tracks its own retry count independently, so multiple concurrent tool
    /// calls each get the full budget. After this many retries the standard
    /// error-formatting path is used instead.
    ///
    /// Default is `2`.
    pub max_tool_retries: u32,
    /// Provider-specific options forwarded to InferRequest.
    /// Keyed by provider name (e.g. "anthropic", "openai").
    pub provider_options: HashMap<String, serde_json::Value>,
}

impl Clone for ReactLoopConfig {
    fn clone(&self) -> Self {
        Self {
            system_prompt: self.system_prompt.clone(),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            tool_filter: self.tool_filter.clone(),
            tool_result_formatter: self.tool_result_formatter.clone(),
            tool_error_formatter: self.tool_error_formatter.clone(),
            system_prompt_fn: self.system_prompt_fn.clone(),
            max_tool_retries: self.max_tool_retries,
            provider_options: self.provider_options.clone(),
        }
    }
}

impl Default for ReactLoopConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            model: None,
            max_tokens: Some(4096),
            temperature: None,
            tool_filter: None,
            tool_result_formatter: None,
            tool_error_formatter: None,
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: HashMap::new(),
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
            .field(
                "tool_result_formatter",
                &self.tool_result_formatter.as_ref().map(|_| "<formatter>"),
            )
            .field(
                "tool_error_formatter",
                &self.tool_error_formatter.as_ref().map(|_| "<formatter>"),
            )
            .field(
                "system_prompt_fn",
                &self.system_prompt_fn.as_ref().map(|_| "Some(Fn)"),
            )
            .field("max_tool_retries", &self.max_tool_retries)
            .field("provider_options", &self.provider_options)
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
            provider_options: self.provider_options.clone(),
        }
    }
}

/// Map a provider stop reason to an operator outcome.
///
/// This is the decision point for determining why an agent loop ended.
/// The default mapping used by [`react_loop()`]:
/// - `StopReason::ContentFilter` → `Outcome::Intercepted { SafetyStop }`
/// - Everything else → `Outcome::Terminal { Completed }`
///
/// Override this by writing your own loop and using a different mapping.
pub fn check_exit(stop_reason: &StopReason) -> Outcome {
    match stop_reason {
        StopReason::ContentFilter => Outcome::Intercepted {
            interception: InterceptionKind::SafetyStop {
                reason: "content filter triggered".into(),
            },
        },
        _ => Outcome::Terminal {
            terminal: TerminalOutcome::Completed,
        },
    }
}

/// Return true when a tool result value is a [`HandoffTool`] sentinel.
///
/// A sentinel is identified by `{ "__handoff": true, ... }`. The check
/// is performed on the raw `serde_json::Value` before any formatting so
/// the orchestration layer can intercept it regardless of how the result
/// would otherwise appear to the model.
pub fn is_handoff_sentinel(value: &serde_json::Value) -> bool {
    value
        .get("__handoff")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Check which tool calls require approval and return the corresponding intents.
///
/// Returns a vec of `Intent::RequestApproval` for each tool call where
/// `tool.approval_policy().requires_approval(&input)` is true. Returns an empty
/// vec if no tools require approval.
///
/// This is the decision point for human-in-the-loop approval. The caller
/// decides what to do with the intents (emit them, filter them, etc.).
pub fn check_approval(tool_calls: &[ToolCall], registry: &ToolRegistry) -> Vec<Intent> {
    tool_calls
        .iter()
        .filter(|call| {
            registry
                .get(&call.name)
                .is_some_and(|t| t.approval_policy().requires_approval(&call.input))
        })
        .map(|call| {
            Intent::new(IntentKind::RequestApproval {
                tool_name: call.name.clone(),
                call_id: call.id.clone(),
                input: call.input.clone(),
            })
        })
        .collect()
}

/// Result of resolving an [`ApprovalResponse`] against pending tool calls.
#[derive(Debug)]
pub struct ResolvedApproval {
    /// Tool calls approved for dispatch (possibly with modified inputs).
    pub approved: Vec<ApprovedToolCall>,
    /// Tool calls rejected by the human.
    pub rejected: Vec<RejectedToolCall>,
}

/// A tool call that has been approved for dispatch.
#[derive(Debug, Clone)]
pub struct ApprovedToolCall {
    /// Provider-assigned call ID for correlation.
    pub call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Tool input (may have been modified by the human).
    pub tool_input: serde_json::Value,
}

/// A tool call that was rejected by the human.
#[derive(Debug, Clone)]
pub struct RejectedToolCall {
    /// Provider-assigned call ID for correlation.
    pub call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Human-supplied rejection reason.
    pub reason: String,
}

/// Reconstruct pending tool calls from intents and resolve against an [`ApprovalResponse`].
///
/// Extracts [`IntentKind::RequestApproval`] from the intents slice, matches them
/// against the approval response, and returns which calls to dispatch and which
/// were rejected.
///
/// Unknown `call_id`s referenced in the response (not present in `intents`) are
/// silently ignored — only pending calls participate in the resolution.
pub fn resolve_approval(intents: &[Intent], response: &ApprovalResponse) -> ResolvedApproval {
    // Collect pending tool calls from RequestApproval intents.
    let pending: Vec<(&str, &str, &serde_json::Value)> = intents
        .iter()
        .filter_map(|i| match &i.kind {
            IntentKind::RequestApproval {
                tool_name,
                call_id,
                input,
            } => Some((call_id.as_str(), tool_name.as_str(), input)),
            _ => None,
        })
        .collect();

    let mut approved = Vec::new();
    let mut rejected = Vec::new();

    match response {
        ApprovalResponse::ApproveAll => {
            for (call_id, tool_name, input) in &pending {
                approved.push(ApprovedToolCall {
                    call_id: (*call_id).to_string(),
                    tool_name: (*tool_name).to_string(),
                    tool_input: (*input).clone(),
                });
            }
        }
        ApprovalResponse::RejectAll { reason } => {
            for (call_id, tool_name, _) in &pending {
                rejected.push(RejectedToolCall {
                    call_id: (*call_id).to_string(),
                    tool_name: (*tool_name).to_string(),
                    reason: reason.clone(),
                });
            }
        }
        ApprovalResponse::Approve { call_ids } => {
            // Calls in the list are approved; all others are rejected.
            for (call_id, tool_name, input) in &pending {
                if call_ids.iter().any(|id| id.as_str() == *call_id) {
                    approved.push(ApprovedToolCall {
                        call_id: (*call_id).to_string(),
                        tool_name: (*tool_name).to_string(),
                        tool_input: (*input).clone(),
                    });
                } else {
                    rejected.push(RejectedToolCall {
                        call_id: (*call_id).to_string(),
                        tool_name: (*tool_name).to_string(),
                        reason: "not approved".into(),
                    });
                }
            }
        }
        ApprovalResponse::Reject { call_ids, reason } => {
            // Calls in the list are rejected; all others are approved.
            for (call_id, tool_name, input) in &pending {
                if call_ids.iter().any(|id| id.as_str() == *call_id) {
                    rejected.push(RejectedToolCall {
                        call_id: (*call_id).to_string(),
                        tool_name: (*tool_name).to_string(),
                        reason: reason.clone(),
                    });
                } else {
                    approved.push(ApprovedToolCall {
                        call_id: (*call_id).to_string(),
                        tool_name: (*tool_name).to_string(),
                        tool_input: (*input).clone(),
                    });
                }
            }
        }
        ApprovalResponse::Modify {
            call_id: target_id,
            new_input,
        } => {
            // Target call is approved with new_input; all other calls approved with original.
            for (call_id, tool_name, input) in &pending {
                let tool_input = if call_id == target_id {
                    new_input.clone()
                } else {
                    (*input).clone()
                };
                approved.push(ApprovedToolCall {
                    call_id: (*call_id).to_string(),
                    tool_name: (*tool_name).to_string(),
                    tool_input,
                });
            }
        }
        ApprovalResponse::Batch { decisions } => {
            // Per-call resolution. Calls with no decision are rejected (conservative).
            for (call_id, tool_name, input) in &pending {
                match decisions.iter().find(|d| d.call_id.as_str() == *call_id) {
                    Some(d) => match &d.action {
                        ToolCallAction::Approve => {
                            approved.push(ApprovedToolCall {
                                call_id: (*call_id).to_string(),
                                tool_name: (*tool_name).to_string(),
                                tool_input: (*input).clone(),
                            });
                        }
                        ToolCallAction::Reject { reason } => {
                            rejected.push(RejectedToolCall {
                                call_id: (*call_id).to_string(),
                                tool_name: (*tool_name).to_string(),
                                reason: reason.clone(),
                            });
                        }
                        ToolCallAction::Modify { new_input } => {
                            approved.push(ApprovedToolCall {
                                call_id: (*call_id).to_string(),
                                tool_name: (*tool_name).to_string(),
                                tool_input: new_input.clone(),
                            });
                        }
                        // Non-exhaustive: future actions default to reject.
                        _ => {
                            rejected.push(RejectedToolCall {
                                call_id: (*call_id).to_string(),
                                tool_name: (*tool_name).to_string(),
                                reason: "unrecognized action".into(),
                            });
                        }
                    },
                    // No decision for this call — reject conservatively.
                    None => {
                        rejected.push(RejectedToolCall {
                            call_id: (*call_id).to_string(),
                            tool_name: (*tool_name).to_string(),
                            reason: "no decision provided".into(),
                        });
                    }
                }
            }
        }
        // Non-exhaustive: unknown response variants reject all pending (defensive).
        _ => {
            for (call_id, tool_name, _) in &pending {
                rejected.push(RejectedToolCall {
                    call_id: (*call_id).to_string(),
                    tool_name: (*tool_name).to_string(),
                    reason: "unrecognized approval response".into(),
                });
            }
        }
    }

    ResolvedApproval { approved, rejected }
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

fn structured_exit_output(
    err: EngineError,
    ctx: &mut Context,
) -> Result<OperatorOutput, EngineError> {
    match err {
        EngineError::Exit { outcome, .. } => {
            Ok(make_context_output(Content::text(""), outcome, ctx))
        }
        other => Err(other),
    }
}

enum ToolDispatchOutcome {
    Continue,
    AwaitingApproval,
    Exit(Outcome),
}

fn make_context_output(message: Content, outcome: Outcome, ctx: &mut Context) -> OperatorOutput {
    let mut output = OperatorOutput::new(message, outcome);
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = ctx.metrics.tokens_in;
    meta.tokens_out = ctx.metrics.tokens_out;
    meta.cost = ctx.metrics.cost;
    meta.turns_used = ctx.metrics.turns_completed;
    meta.duration = DurationMs::from_millis(ctx.metrics.elapsed_ms());
    output.metadata = meta;
    output.intents = ctx.drain_intents();
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
///    `Outcome::Suspended` and `IntentKind::RequestApproval`
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
    dispatch_ctx: &DispatchContext,
    config: &ReactLoopConfig,
) -> Result<OperatorOutput, EngineError> {
    let mut tool_retry_counts: HashMap<String, u32> = HashMap::new();
    loop {
        // Enter InferBoundary: drains pending interventions + fires Before rules.
        // Compile AFTER so any context mutations (injected messages, tool changes)
        // appear in the request.
        if let Err(err) = ctx.enter_boundary::<InferBoundary>().await {
            return structured_exit_output(err, ctx);
        }

        // Phase 1: Compile and infer (re-filter tools each turn, after before rules)
        let compile_config = config.compile_config(tools, ctx);
        let compiled = ctx.compile(&compile_config);

        let infer_span = tracing::info_span!("infer", turn = ctx.metrics.turns_completed);
        let result = tracing::Instrument::instrument(compiled.infer(provider), infer_span).await?;

        // Exit InferBoundary: fires After rules
        if let Err(err) = ctx.exit_boundary::<InferBoundary>().await {
            return structured_exit_output(err, ctx);
        }

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
        let approval_intents = check_approval(&tool_calls, tools);

        if !approval_intents.is_empty() {
            ctx.extend_intents(approval_intents);
            return Ok(make_output(
                result.response,
                Outcome::Suspended {
                    reason: WaitReason::Approval,
                },
                ctx,
            ));
        }

        // Phase 5: Dispatch tool calls
        //
        // Concurrency policy: if ALL calls this turn have the Shared hint, run them
        // concurrently with `join_all`. A single Exclusive call (the safe default)
        // degrades the entire turn to sequential. Unknown tools default to Exclusive.
        // Note: the parallel path bypasses `ctx.run()` rule-firing per tool call.
        // Built-in budget guards target `InferBoundary` (not `ExecuteTool`), so they
        // still fire correctly.
        let all_shared = tool_calls.len() > 1
            && tool_calls.iter().all(|call| {
                tools
                    .get(&call.name)
                    .map(|t| t.concurrency_hint() == ToolConcurrencyHint::Shared)
                    .unwrap_or(false)
            });
        if all_shared {
            // Parallel path: run all tool.call() futures concurrently.
            // Results are returned in original call order (join_all preserves order),
            // then processed sequentially into ctx for metrics and message injection.
            let futures: Vec<_> = tool_calls
                .iter()
                .map(|call| {
                    let tool = tools.get(&call.name).cloned();
                    let call_name = call.name.clone();
                    let input = call.input.clone();
                    let d_ctx = dispatch_ctx.clone();
                    let span =
                        tracing::info_span!("tool_call", tool = %call_name, call_id = %call.id);
                    tracing::Instrument::instrument(
                        async move {
                            let start = std::time::Instant::now();
                            let result = match tool {
                                None => Err(EngineError::Halted {
                                    reason: format!("unknown tool: {call_name}"),
                                }),
                                Some(t) => t.call(input, &d_ctx).await.map_err(Into::into),
                            };
                            (start.elapsed(), result)
                        },
                        span,
                    )
                })
                .collect();
            let raw_results = futures_util::future::join_all(futures).await;
            for (call, (duration, tool_result)) in tool_calls.iter().zip(raw_results) {
                ctx.metrics.tool_calls_total += 1;
                let (result_str, is_error) = match tool_result {
                    Ok(value) => {
                        tracing::debug!(tool = %call.name, duration_ms = duration.as_millis() as u64, "tool succeeded");
                        if is_handoff_sentinel(&value) {
                            let target = value
                                .get("target")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let reason = value
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            ctx.push_intent(Intent::new(IntentKind::Handoff {
                                operator: OperatorId::from(target.as_str()),
                                context: HandoffContext {
                                    task: Content::text(reason),
                                    history: None,
                                    metadata: None,
                                },
                            }));
                            return Ok(make_context_output(
                                Content::text(""),
                                Outcome::Transfer {
                                    transfer: TransferOutcome::HandedOff,
                                },
                                ctx,
                            ));
                        }
                        let s = match &config.tool_result_formatter {
                            Some(f) => f(&call.name, &value),
                            None => format_tool_result(&value),
                        };
                        (s, false)
                    }
                    Err(e) => {
                        ctx.metrics.tool_calls_failed += 1;
                        tracing::warn!(tool = %call.name, duration_ms = duration.as_millis() as u64, error = %e, "tool failed");
                        if let EngineError::Tool(ToolError::InvalidInput(ref msg)) = e {
                            let count = tool_retry_counts.entry(call.id.clone()).or_insert(0);
                            if *count < config.max_tool_retries {
                                *count += 1;
                                let schema_str = tools
                                    .get(&call.name)
                                    .map(|t| t.input_schema().to_string())
                                    .unwrap_or_else(|| "unknown".to_string());
                                (
                                    format!(
                                        "Tool '{}' rejected the input: {}\nExpected schema: {}\nPlease fix the input and try again.",
                                        call.name, msg, schema_str
                                    ),
                                    true,
                                )
                            } else {
                                (
                                    match &config.tool_error_formatter {
                                        Some(f) => f(&call.name, &e.to_string()),
                                        None => format_tool_error(&e),
                                    },
                                    false,
                                )
                            }
                        } else {
                            (
                                match &config.tool_error_formatter {
                                    Some(f) => f(&call.name, &e.to_string()),
                                    None => format_tool_error(&e),
                                },
                                false,
                            )
                        }
                    }
                };
                // Append tool result to context
                let result_msg =
                    InferResponse::tool_result_message(&call.id, &call.name, result_str, is_error);
                if let Err(err) = ctx.inject_message(result_msg).await {
                    return structured_exit_output(err, ctx);
                }
            }
        } else {
            for call in &tool_calls {
                let tool_span =
                    tracing::info_span!("tool_call", tool = %call.name, call_id = %call.id);
                let start = std::time::Instant::now();
                let tool_result = tracing::Instrument::instrument(
                    ctx.run(
                        ExecuteTool::new(call.clone(), tools.clone(), dispatch_ctx.clone())
                            .maybe_with_dispatcher(
                                dispatch_ctx
                                    .extensions()
                                    .get::<Arc<dyn Dispatcher>>()
                                    .cloned(),
                            ),
                    ),
                    tool_span,
                )
                .await;
                let (result_str, is_error) = match tool_result {
                    Ok(value) => {
                        tracing::debug!(tool = %call.name, duration_ms = start.elapsed().as_millis() as u64, "tool succeeded");
                        // Detect HandoffTool sentinel BEFORE formatting — check the raw
                        // Value so the check is independent of any custom formatter.
                        if is_handoff_sentinel(&value) {
                            let target = value
                                .get("target")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let reason = value
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            ctx.push_intent(Intent::new(IntentKind::Handoff {
                                operator: OperatorId::from(target.as_str()),
                                context: HandoffContext {
                                    task: Content::text(reason),
                                    history: None,
                                    metadata: None,
                                },
                            }));
                            return Ok(make_context_output(
                                Content::text(""),
                                Outcome::Transfer {
                                    transfer: TransferOutcome::HandedOff,
                                },
                                ctx,
                            ));
                        }
                        let s = match &config.tool_result_formatter {
                            Some(f) => f(&call.name, &value),
                            None => format_tool_result(&value),
                        };
                        (s, false)
                    }
                    Err(e) => {
                        tracing::warn!(tool = %call.name, duration_ms = start.elapsed().as_millis() as u64, error = %e, "tool failed");
                        // InvalidInput: feed schema + error back so the model can correct the call.
                        if let EngineError::Tool(ToolError::InvalidInput(ref msg)) = e {
                            let count = tool_retry_counts.entry(call.id.clone()).or_insert(0);
                            if *count < config.max_tool_retries {
                                *count += 1;
                                let schema_str = tools
                                    .get(&call.name)
                                    .map(|t| t.input_schema().to_string())
                                    .unwrap_or_else(|| "unknown".to_string());
                                (
                                    format!(
                                        "Tool '{}' rejected the input: {}\nExpected schema: {}\nPlease fix the input and try again.",
                                        call.name, msg, schema_str
                                    ),
                                    true,
                                )
                            } else {
                                (
                                    match &config.tool_error_formatter {
                                        Some(f) => f(&call.name, &e.to_string()),
                                        None => format_tool_error(&e),
                                    },
                                    false,
                                )
                            }
                        } else {
                            (
                                match &config.tool_error_formatter {
                                    Some(f) => f(&call.name, &e.to_string()),
                                    None => format_tool_error(&e),
                                },
                                false,
                            )
                        }
                    }
                };
                // Append tool result to context
                let result_msg =
                    InferResponse::tool_result_message(&call.id, &call.name, result_str, is_error);
                if let Err(err) = ctx.inject_message(result_msg).await {
                    return structured_exit_output(err, ctx);
                }
            }
        }
    }
}

fn make_output(response: InferResponse, outcome: Outcome, ctx: &mut Context) -> OperatorOutput {
    make_context_output(response.content, outcome, ctx)
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
            extra: None,
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
            extra: None,
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
/// Returns `(Some(validated_value), operator_output)` on success. Returns `(None,
/// operator_output)` when the loop exits before producing a validated value — the caller
/// **must** inspect [`OperatorOutput::outcome`] to determine whether the exit is
/// resumable (`Outcome::Suspended`) or terminal (budget, timeout, safety, etc.).
pub async fn react_loop_structured<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    dispatch_ctx: &DispatchContext,
    config: &ReactLoopConfig,
    output: &OutputSchema,
) -> Result<(Option<Value>, OperatorOutput), EngineError> {
    let output_tool_schema = if output.mode == OutputMode::ToolCall {
        Some(output.tool_schema())
    } else {
        None
    };

    let mut output_retries: u32 = 0;
    let mut tool_retry_counts: HashMap<String, u32> = HashMap::new();

    loop {
        // Enter InferBoundary: drains pending interventions + fires Before rules.
        // Compile AFTER so any context mutations appear in the request.
        if let Err(err) = ctx.enter_boundary::<InferBoundary>().await {
            match structured_exit_output(err, ctx) {
                Ok(output) => return Ok((None, output)),
                Err(other) => return Err(other),
            }
        }

        // Phase 1: Compile and infer (re-filter tools each turn, after before rules)
        let mut compile_config = config.compile_config(tools, ctx);
        if let Some(schema) = &output_tool_schema {
            compile_config.tools.push(schema.clone());
        }
        let compiled = ctx.compile(&compile_config);

        let result = compiled.infer(provider).await?;

        // Exit InferBoundary: fires After rules
        if let Err(err) = ctx.exit_boundary::<InferBoundary>().await {
            match structured_exit_output(err, ctx) {
                Ok(output) => return Ok((None, output)),
                Err(other) => return Err(other),
            }
        }

        // Phase 2: Append response to context (rules fire)
        if let Err(err) = ctx.run(AppendResponse::new(result.response.clone())).await {
            match structured_exit_output(err, ctx) {
                Ok(output) => return Ok((None, output)),
                Err(other) => return Err(other),
            }
        }
        ctx.metrics.turns_completed += 1;

        // Phase 3: Try to extract structured output
        match output.extract(&result.response) {
            Ok(value) => {
                let op_output = make_output(
                    result.response,
                    Outcome::Terminal {
                        terminal: TerminalOutcome::Completed,
                    },
                    ctx,
                );
                return Ok((Some(value), op_output));
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
                let dispatch = dispatch_function_tools(
                    ctx,
                    &result.response,
                    tools,
                    dispatch_ctx,
                    &output.tool_name,
                    config,
                    &mut tool_retry_counts,
                )
                .await?;
                match dispatch {
                    ToolDispatchOutcome::Continue => continue,
                    ToolDispatchOutcome::AwaitingApproval => {
                        let op_output = make_output(
                            result.response,
                            Outcome::Suspended {
                                reason: WaitReason::Approval,
                            },
                            ctx,
                        );
                        return Ok((None, op_output));
                    }
                    ToolDispatchOutcome::Exit(outcome) => {
                        let op_output = make_context_output(Content::text(""), outcome, ctx);
                        return Ok((None, op_output));
                    }
                }
            }
            Err(OutputError::NoOutput) => {
                // No structured output — check for function tool calls
                if result.has_tool_calls() {
                    let dispatch = dispatch_function_tools(
                        ctx,
                        &result.response,
                        tools,
                        dispatch_ctx,
                        &output.tool_name,
                        config,
                        &mut tool_retry_counts,
                    )
                    .await?;
                    match dispatch {
                        ToolDispatchOutcome::Continue => continue,
                        ToolDispatchOutcome::AwaitingApproval => {
                            let op_output = make_output(
                                result.response,
                                Outcome::Suspended {
                                    reason: WaitReason::Approval,
                                },
                                ctx,
                            );
                            return Ok((None, op_output));
                        }
                        ToolDispatchOutcome::Exit(outcome) => {
                            let op_output = make_context_output(Content::text(""), outcome, ctx);
                            return Ok((None, op_output));
                        }
                    }
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
/// If any tool requires approval, stores [`IntentKind::RequestApproval`]
/// intents in the context and returns `AwaitingApproval` (caller should exit with
/// `Outcome::Suspended`).
async fn dispatch_function_tools(
    ctx: &mut Context,
    response: &InferResponse,
    tools: &ToolRegistry,
    dispatch_ctx: &DispatchContext,
    output_tool_name: &str,
    config: &ReactLoopConfig,
    tool_retry_counts: &mut HashMap<String, u32>,
) -> Result<ToolDispatchOutcome, EngineError> {
    // Check for approval-required tools first (excluding output tool)
    let function_calls: Vec<_> = response
        .tool_calls
        .iter()
        .filter(|call| call.name != output_tool_name)
        .cloned()
        .collect();
    let approval_intents = check_approval(&function_calls, tools);

    if !approval_intents.is_empty() {
        ctx.extend_intents(approval_intents);
        return Ok(ToolDispatchOutcome::AwaitingApproval);
    }

    for call in &response.tool_calls {
        if call.name == output_tool_name {
            continue;
        }
        let (result_str, is_error) = match ctx
            .run(
                ExecuteTool::new(call.clone(), tools.clone(), dispatch_ctx.clone())
                    .maybe_with_dispatcher(
                        dispatch_ctx
                            .extensions()
                            .get::<Arc<dyn Dispatcher>>()
                            .cloned(),
                    ),
            )
            .await
        {
            Ok(value) => {
                let s = match &config.tool_result_formatter {
                    Some(f) => f(&call.name, &value),
                    None => format_tool_result(&value),
                };
                (s, false)
            }
            Err(EngineError::Exit { outcome, .. }) => {
                return Ok(ToolDispatchOutcome::Exit(outcome));
            }
            Err(e) => {
                // InvalidInput: feed schema + error back so the model can correct the call.
                if let EngineError::Tool(ToolError::InvalidInput(ref msg)) = e {
                    let count = tool_retry_counts.entry(call.id.clone()).or_insert(0);
                    if *count < config.max_tool_retries {
                        *count += 1;
                        let schema_str = tools
                            .get(&call.name)
                            .map(|t| t.input_schema().to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        (
                            format!(
                                "Tool '{}' rejected the input: {}\nExpected schema: {}\nPlease fix the input and try again.",
                                call.name, msg, schema_str
                            ),
                            true,
                        )
                    } else {
                        (
                            match &config.tool_error_formatter {
                                Some(f) => f(&call.name, &e.to_string()),
                                None => format_tool_error(&e),
                            },
                            false,
                        )
                    }
                } else {
                    (
                        match &config.tool_error_formatter {
                            Some(f) => f(&call.name, &e.to_string()),
                            None => format_tool_error(&e),
                        },
                        false,
                    )
                }
            }
        };
        let result_msg =
            InferResponse::tool_result_message(&call.id, &call.name, result_str, is_error);
        if let Err(err) = ctx.inject_message(result_msg).await {
            match err {
                EngineError::Exit { outcome, .. } => return Ok(ToolDispatchOutcome::Exit(outcome)),
                other => return Err(other),
            }
        }
    }
    Ok(ToolDispatchOutcome::Continue)
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
    use layer0::{DispatchContext, DispatchId};
    use serde_json::json;
    use skg_tool::{ToolConcurrencyHint, ToolDyn, ToolError};
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
            _ctx: &DispatchContext,
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
            tool_result_formatter: None,
            tool_error_formatter: None,
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: std::collections::HashMap::new(),
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        let request = provider
            .last_request()
            .expect("provider should record request");
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        let request = provider
            .last_request()
            .expect("provider should record request");
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            EngineError::Provider(ProviderError::TransientError { .. })
        ));
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap_err();

        assert!(matches!(err, EngineError::Halted { .. }));
        assert_eq!(provider.call_count(), 0);
    }

    async fn assert_budget_exit_before_provider_call(
        mutate_ctx: impl FnOnce(&mut Context),
        config: BudgetGuardConfig,
        expected_outcome: Outcome,
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(output.outcome, expected_outcome);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
    }

    async fn assert_structured_budget_exit_before_provider_call(
        mutate_ctx: impl FnOnce(&mut Context),
        config: BudgetGuardConfig,
        expected_outcome: Outcome,
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::text_json(json!({}), |v| Ok(v.clone()));
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

        assert!(value.is_none());
        assert_eq!(output.outcome, expected_outcome);
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
            Outcome::Limited {
                limit: LimitReason::MaxTurns,
            },
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
            Outcome::Limited {
                limit: LimitReason::BudgetExhausted,
            },
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
            Outcome::Limited {
                limit: LimitReason::Timeout,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn react_loop_structured_returns_budget_exit_before_provider_call() {
        assert_structured_budget_exit_before_provider_call(
            |ctx| ctx.metrics.turns_completed = 1,
            BudgetGuardConfig {
                max_cost: None,
                max_turns: Some(1),
                max_duration: None,
                max_tool_calls: None,
            },
            Outcome::Limited {
                limit: LimitReason::MaxTurns,
            },
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
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(provider.call_count(), 1);

        let request = provider
            .last_request()
            .expect("provider should record request");
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, output) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        let value = value.expect("structured loop should return a validated value");
        assert_eq!(value["name"], "Tokyo");
        assert_eq!(value["population"], 13960000_u64);
        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        let value = value.expect("structured loop should return a validated value");
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let err = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::text_json(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        let value = value.expect("structured loop should return a validated value");
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        // ToolCall mode: model returns text instead of calling return_result
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let err = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
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

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), city_validator);

        let (value, _) = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &schema,
        )
        .await
        .unwrap();

        let value = value.expect("structured loop should return a validated value");
        assert_eq!(value["name"], "Tokyo");
        assert_eq!(provider.call_count(), 2);
    }

    #[tokio::test]
    async fn structured_function_tools_preserve_awaiting_approval_exit() {
        let provider = TestProvider::new();
        provider.respond_with_tool_calls(vec![
            ("safe_tool", "c1", json!({ "query": "status" })),
            ("dangerous_tool", "c2", json!({ "cmd": "deploy" })),
        ]);

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "safe_tool" }));
        tools.register(Arc::new(ApprovalTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
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

        assert!(
            value.is_none(),
            "approval pause must not fabricate structured output"
        );
        assert_eq!(
            output.outcome,
            Outcome::Suspended {
                reason: WaitReason::Approval,
            }
        );
        assert_eq!(output.intents.len(), 1);
        match &output.intents[0].kind {
            IntentKind::RequestApproval {
                tool_name,
                call_id,
                input,
            } => {
                assert_eq!(tool_name, "dangerous_tool");
                assert_eq!(call_id, "c2");
                assert_eq!(input, &json!({ "cmd": "deploy" }));
            }
            other => panic!("expected RequestApproval, got {other:?}"),
        }

        // Safe tool should not run once the loop pauses for approval.
        assert_eq!(provider.call_count(), 1);
        assert_eq!(ctx.metrics.tool_calls_total, 0);
    }

    struct ExitBeforeToolDispatch;

    #[async_trait]
    impl ContextOp for ExitBeforeToolDispatch {
        type Output = ();

        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Err(EngineError::Exit {
                outcome: Outcome::Limited {
                    limit: LimitReason::Timeout,
                },
                detail: "tool dispatch paused".into(),
            })
        }
    }

    #[tokio::test]
    async fn structured_function_tool_dispatch_preserves_structured_exit() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("search", "call_1", json!({ "query": "Tokyo" }));

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "search" }));

        let mut ctx = Context::with_rules(vec![Rule::before::<ExecuteTool>(
            "exit before tool dispatch",
            100,
            ExitBeforeToolDispatch,
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
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

        assert!(value.is_none());
        assert_eq!(
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::Timeout,
            }
        );
        assert_eq!(provider.call_count(), 1);
        assert_eq!(ctx.metrics.tool_calls_total, 0);
        assert!(
            !ctx.messages()
                .iter()
                .any(|message| message.text_content().contains("mock result")),
            "tool result should not be injected after a structured exit"
        );
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::tool_call(json!({}), |v| Ok(v.clone()));

        let _ = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
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
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let schema = OutputSchema::text_json(json!({}), |v| Ok(v.clone()));

        let _ = react_loop_structured(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
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
            tool_result_formatter: None,
            tool_error_formatter: None,
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: std::collections::HashMap::new(),
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

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            system_prompt: "test".into(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: Some(Arc::new(|tool: &dyn ToolDyn, _ctx: &Context| {
                tool.name() != "blocked"
            })),
            tool_result_formatter: None,
            tool_error_formatter: None,
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: std::collections::HashMap::new(),
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
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
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Ok(json!("should not reach here")) })
        }
        fn approval_policy(&self) -> skg_tool::ApprovalPolicy {
            skg_tool::ApprovalPolicy::Always
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

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        // Should exit with AwaitingApproval
        assert_eq!(
            output.outcome,
            Outcome::Suspended {
                reason: WaitReason::Approval,
            }
        );

        // Intents now flow through Context into output.intents
        assert!(!output.intents.is_empty());

        // Provider should have been called exactly once

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

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        // Normal completion — approval_policy defaults to ApprovalPolicy::None
        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(output.intents.is_empty());
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

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        // Should exit with AwaitingApproval (approval check happens before dispatch)
        assert_eq!(
            output.outcome,
            Outcome::Suspended {
                reason: WaitReason::Approval,
            }
        );
        // Intents now flow through Context into output.intents
        assert!(!output.intents.is_empty());
    }

    #[tokio::test]
    async fn react_loop_approval_effects_appear_in_output() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("dangerous_tool", "c1", json!({ "cmd": "rm -rf /" }));

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ApprovalTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("delete everything")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Suspended {
                reason: WaitReason::Approval,
            }
        );
        assert!(
            !output.intents.is_empty(),
            "approval intents must appear in OperatorOutput"
        );

        // Verify the intent is the expected RequestApproval variant
        match &output.intents[0].kind {
            IntentKind::RequestApproval {
                tool_name, call_id, ..
            } => {
                assert_eq!(tool_name, "dangerous_tool");
                assert_eq!(call_id, "c1");
            }
            other => panic!("expected RequestApproval, got {other:?}"),
        }
    }

    struct AlwaysFailingTool;

    impl ToolDyn for AlwaysFailingTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "always fails"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Err(ToolError::ExecutionFailed("boom".into())) })
        }
    }

    /// Failed tool calls must count toward the tool budget, otherwise a tool
    /// that always fails causes an infinite loop when `max_tool_calls` is set.
    #[tokio::test]
    async fn react_loop_failed_tool_calls_count_toward_budget() {
        let provider = TestProvider::new();
        // Provide more responses than needed; budget guard must halt before all are consumed.
        provider.respond_with_tool_call("fail", "c1", json!({}));
        provider.respond_with_tool_call("fail", "c2", json!({}));
        provider.respond_with_tool_call("fail", "c3", json!({}));
        provider.respond_with_text("should never reach");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysFailingTool));

        let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
            "budget_guard",
            100,
            BudgetGuard::with_config(BudgetGuardConfig {
                max_cost: None,
                max_turns: None,
                max_duration: None,
                max_tool_calls: Some(3),
            }),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::BudgetExhausted,
            },
            "loop must exit with BudgetExhausted after 3 failed tool calls"
        );
        // Provider was called exactly 3 times (once per tool-call response consumed).
        assert_eq!(provider.call_count(), 3);
        assert_eq!(ctx.metrics.tool_calls_total, 3);
        assert_eq!(ctx.metrics.tool_calls_failed, 3);
    }

    #[test]
    fn test_check_exit_content_filter() {
        let outcome = check_exit(&StopReason::ContentFilter);
        match outcome {
            Outcome::Intercepted {
                interception: InterceptionKind::SafetyStop { reason },
            } => {
                assert_eq!(reason, "content filter triggered");
            }
            other => panic!("expected Intercepted(SafetyStop), got {other:?}"),
        }
    }

    #[test]
    fn test_check_exit_normal() {
        let outcome = check_exit(&StopReason::EndTurn);
        assert_eq!(
            outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
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

        let intents = check_approval(&tool_calls, &tools);
        assert_eq!(intents.len(), 1);
        match &intents[0].kind {
            IntentKind::RequestApproval {
                tool_name,
                call_id,
                input,
            } => {
                assert_eq!(tool_name, "dangerous_tool");
                assert_eq!(call_id, "c1");
                assert_eq!(input, &json!({ "x": 1 }));
            }
            other => panic!("expected RequestApproval, got {other:?}"),
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

    // Regression test: effects pushed to ctx before a budget-exhausted exit must
    // appear in the output. Previously make_context_output hardcoded vec![] and
    // silently dropped them.
    #[tokio::test]
    async fn react_loop_budget_exit_preserves_pending_effects() {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<InferBoundary>(
            "budget_guard",
            100,
            BudgetGuard::with_config(BudgetGuardConfig {
                max_cost: None,
                max_turns: Some(1),
                max_duration: None,
                max_tool_calls: None,
            }),
        )]);
        // Pre-seed an intent that must survive the exit path.
        ctx.push_intent(Intent::new(IntentKind::Custom {
            name: "sentinel".into(),
            payload: serde_json::json!({"content": "sentinel-intent"}),
        }));
        // Trip the budget guard: turns_completed already at limit.
        ctx.metrics.turns_completed = 1;
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::MaxTurns,
            }
        );
        assert_eq!(
            output.intents.len(),
            1,
            "intent pushed before budget exit must not be dropped"
        );
        match &output.intents[0].kind {
            IntentKind::Custom { name, payload } => {
                assert_eq!(name, "sentinel");
                assert_eq!(payload["content"], "sentinel-intent");
            }
            other => panic!("expected Custom intent, got {other:?}"),
        }
        // Provider must not have been called — the budget guard fired before inference.
        assert_eq!(provider.call_count(), 0);
    }

    // --- resolve_approval unit tests ---

    #[test]
    fn resolve_approve_all() {
        use layer0::approval::ApprovalResponse;
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({ "x": 1 }),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({ "x": 2 }),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_c".into(),
                call_id: "call_3".into(),
                input: json!({ "x": 3 }),
            }),
        ];
        let result = resolve_approval(&intents, &ApprovalResponse::ApproveAll);
        assert_eq!(result.approved.len(), 3, "all three should be approved");
        assert_eq!(result.rejected.len(), 0);
        let ids: Vec<_> = result.approved.iter().map(|a| a.call_id.as_str()).collect();
        assert!(ids.contains(&"call_1") && ids.contains(&"call_2") && ids.contains(&"call_3"));
    }

    #[test]
    fn resolve_reject_all() {
        use layer0::approval::ApprovalResponse;
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({}),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({}),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_c".into(),
                call_id: "call_3".into(),
                input: json!({}),
            }),
        ];
        let result = resolve_approval(
            &intents,
            &ApprovalResponse::RejectAll {
                reason: "too dangerous".into(),
            },
        );
        assert_eq!(result.approved.len(), 0);
        assert_eq!(result.rejected.len(), 3, "all three should be rejected");
        assert!(result.rejected.iter().all(|r| r.reason == "too dangerous"));
    }

    #[test]
    fn resolve_partial() {
        use layer0::approval::ApprovalResponse;
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({}),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({}),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_c".into(),
                call_id: "call_3".into(),
                input: json!({}),
            }),
        ];
        let result = resolve_approval(
            &intents,
            &ApprovalResponse::Approve {
                call_ids: vec!["call_1".into(), "call_2".into()],
            },
        );
        assert_eq!(result.approved.len(), 2);
        assert_eq!(result.rejected.len(), 1);
        let approved_ids: Vec<_> = result.approved.iter().map(|a| a.call_id.as_str()).collect();
        assert!(approved_ids.contains(&"call_1") && approved_ids.contains(&"call_2"));
        assert_eq!(result.rejected[0].call_id, "call_3");
    }

    #[test]
    fn resolve_modify() {
        use layer0::approval::ApprovalResponse;
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({ "path": "/etc/passwd" }),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({ "path": "/tmp/safe" }),
            }),
        ];
        let new_input = json!({ "path": "/tmp/approved" });
        let result = resolve_approval(
            &intents,
            &ApprovalResponse::Modify {
                call_id: "call_1".into(),
                new_input: new_input.clone(),
            },
        );
        assert_eq!(result.approved.len(), 2);
        assert_eq!(result.rejected.len(), 0);
        let modified = result
            .approved
            .iter()
            .find(|a| a.call_id == "call_1")
            .unwrap();
        assert_eq!(
            modified.tool_input, new_input,
            "call_1 should use new_input"
        );
        let unchanged = result
            .approved
            .iter()
            .find(|a| a.call_id == "call_2")
            .unwrap();
        assert_eq!(
            unchanged.tool_input,
            json!({ "path": "/tmp/safe" }),
            "call_2 input unchanged"
        );
    }

    #[test]
    fn resolve_batch() {
        use layer0::approval::{ApprovalResponse, ToolCallAction, ToolCallDecision};
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({ "x": 1 }),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({ "x": 2 }),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_c".into(),
                call_id: "call_3".into(),
                input: json!({ "x": 3 }),
            }),
        ];
        let new_input = json!({ "x": 99 });
        let result = resolve_approval(
            &intents,
            &ApprovalResponse::Batch {
                decisions: vec![
                    ToolCallDecision {
                        call_id: "call_1".into(),
                        action: ToolCallAction::Approve,
                    },
                    ToolCallDecision {
                        call_id: "call_2".into(),
                        action: ToolCallAction::Reject {
                            reason: "no".into(),
                        },
                    },
                    ToolCallDecision {
                        call_id: "call_3".into(),
                        action: ToolCallAction::Modify {
                            new_input: new_input.clone(),
                        },
                    },
                ],
            },
        );
        assert_eq!(result.approved.len(), 2, "call_1 and call_3 approved");
        assert_eq!(result.rejected.len(), 1, "call_2 rejected");
        assert!(result.approved.iter().any(|a| a.call_id == "call_1"));
        let modified = result
            .approved
            .iter()
            .find(|a| a.call_id == "call_3")
            .unwrap();
        assert_eq!(modified.tool_input, new_input);
        assert_eq!(result.rejected[0].call_id, "call_2");
        assert_eq!(result.rejected[0].reason, "no");
    }

    #[test]
    fn resolve_unknown_call_id() {
        use layer0::approval::ApprovalResponse;
        // Response includes a call_id not present in pending — silently ignored.
        let intents = vec![
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_a".into(),
                call_id: "call_1".into(),
                input: json!({}),
            }),
            Intent::new(IntentKind::RequestApproval {
                tool_name: "tool_b".into(),
                call_id: "call_2".into(),
                input: json!({}),
            }),
        ];
        let result = resolve_approval(
            &intents,
            &ApprovalResponse::Approve {
                call_ids: vec!["call_1".into(), "call_ghost".into()],
            },
        );
        // call_ghost is not in pending, ignored. call_1 approved, call_2 not in list → rejected.
        assert_eq!(result.approved.len(), 1);
        assert_eq!(result.approved[0].call_id, "call_1");
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].call_id, "call_2");
    }

    // === Formatter hook tests ===

    #[tokio::test]
    async fn custom_tool_result_formatter() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("search", "c1", json!({ "q": "Tokyo" }));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "search" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            system_prompt: String::new(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: None,
            tool_result_formatter: Some(Arc::new(|tool_name: &str, _value: &serde_json::Value| {
                format!("<tool_result tool=\"{tool_name}\">{_value}</tool_result>")
            })),
            tool_error_formatter: None,
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: std::collections::HashMap::new(),
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|m| m.text_content().contains("<tool_result tool=\"search\">")),
            "custom formatter output must appear in context"
        );
    }

    #[tokio::test]
    async fn custom_tool_error_formatter() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("fail", "c1", json!({}));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysFailingTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            system_prompt: String::new(),
            model: None,
            max_tokens: None,
            temperature: None,
            tool_filter: None,
            tool_result_formatter: None,
            tool_error_formatter: Some(Arc::new(|tool_name: &str, error: &str| {
                format!("<error tool=\"{tool_name}\">{error}</error>")
            })),
            system_prompt_fn: None,
            max_tool_retries: 2,
            provider_options: std::collections::HashMap::new(),
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|m| m.text_content().contains("<error tool=\"fail\">")),
            "custom error formatter output must appear in context"
        );
    }

    #[test]
    fn default_formatter_unchanged() {
        // Verify format_tool_result still matches its documented behavior.
        let str_val = serde_json::Value::String("hello world".into());
        assert_eq!(format_tool_result(&str_val), "hello world");

        let json_val = json!({ "key": "value" });
        let formatted = format_tool_result(&json_val);
        assert!(formatted.contains("key") && formatted.contains("value"));

        // Verify ReactLoopConfig::default() leaves both formatters as None.
        let config = ReactLoopConfig::default();
        assert!(
            config.tool_result_formatter.is_none(),
            "default config must not set tool_result_formatter"
        );
        assert!(
            config.tool_error_formatter.is_none(),
            "default config must not set tool_error_formatter"
        );
    }

    // === InvalidInput retry tests ===

    /// Tool that always returns `ToolError::InvalidInput`.
    struct AlwaysInvalidInputTool;

    impl ToolDyn for AlwaysInvalidInputTool {
        fn name(&self) -> &str {
            "picky"
        }
        fn description(&self) -> &str {
            "always rejects input with InvalidInput"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": { "q": { "type": "string" } },
                "required": ["q"]
            })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Err(ToolError::InvalidInput("missing required field 'q'".into())) })
        }
    }

    #[tokio::test]
    async fn invalid_input_injects_retry_message_with_schema() {
        // When a tool returns InvalidInput the react loop injects a structured retry
        // message that includes the error and the tool's expected schema, rather than
        // falling through to the generic error formatter.
        let provider = TestProvider::new();
        provider.respond_with_tool_call("picky", "c1", json!({}));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysInvalidInputTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            max_tool_retries: 2,
            ..ReactLoopConfig::default()
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(
            ctx.messages().iter().any(|m| {
                let t = m.text_content();
                t.contains("rejected the input")
                    && t.contains("Expected schema")
                    && t.contains("Please fix")
            }),
            "retry message with schema must appear in context"
        );
    }

    #[tokio::test]
    async fn invalid_input_max_retries_zero_uses_error_formatter() {
        // When max_tool_retries = 0, InvalidInput immediately falls through to the
        // standard error-formatting path — no retry message is injected.
        let provider = TestProvider::new();
        provider.respond_with_tool_call("picky", "c1", json!({}));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysInvalidInputTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            max_tool_retries: 0,
            ..ReactLoopConfig::default()
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(
            !ctx.messages()
                .iter()
                .any(|m| m.text_content().contains("Expected schema")),
            "no retry schema text expected when max_tool_retries is 0"
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|m| m.text_content().contains("Error:")),
            "standard error format must be used when retries are disabled"
        );
    }

    #[tokio::test]
    async fn invalid_input_retry_count_exhausted_uses_error_formatter() {
        // When the same call_id exceeds max_tool_retries the fallback error formatter
        // is used and no further retry messages are injected.
        let provider = TestProvider::new();
        // Same call_id used across two turns: first retries, second exhausts the budget.
        provider.respond_with_tool_call("picky", "dup", json!({}));
        provider.respond_with_tool_call("picky", "dup", json!({}));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysInvalidInputTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        // max_tool_retries = 1: first InvalidInput retries, second falls through.
        let config = ReactLoopConfig {
            max_tool_retries: 1,
            ..ReactLoopConfig::default()
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        let messages = ctx.messages();
        assert!(
            messages
                .iter()
                .any(|m| m.text_content().contains("Expected schema")),
            "first InvalidInput must inject retry message with schema"
        );
        assert!(
            messages.iter().any(|m| m.text_content().contains("Error:")),
            "second InvalidInput (budget exhausted) must use standard error formatter"
        );
    }

    #[tokio::test]
    async fn non_invalid_input_errors_not_retried() {
        // ExecutionFailed and other non-InvalidInput errors must use the existing
        // error-formatting path and never inject retry messages.
        let provider = TestProvider::new();
        provider.respond_with_tool_call("fail", "c1", json!({}));
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AlwaysFailingTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let config = ReactLoopConfig {
            max_tool_retries: 2,
            ..ReactLoopConfig::default()
        };

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &config)
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert!(
            !ctx.messages()
                .iter()
                .any(|m| m.text_content().contains("Expected schema")),
            "non-InvalidInput errors must not inject retry schema messages"
        );
    }

    // --- Handoff sentinel tests ---

    struct HandoffSentinelTool;

    impl ToolDyn for HandoffSentinelTool {
        fn name(&self) -> &str {
            "transfer_to_routing_agent"
        }
        fn description(&self) -> &str {
            "hand off to routing agent"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async {
                Ok(json!({
                    "__handoff": true,
                    "target": "routing-agent",
                    "reason": "needs routing"
                }))
            })
        }
    }

    /// HandoffTool sentinel must exit the loop with HandedOff + one Handoff effect.
    #[tokio::test]
    async fn handoff_sentinel_exits_loop() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call(
            "transfer_to_routing_agent",
            "hoff_1",
            json!({ "reason": "needs routing" }),
        );
        // If the loop does not exit, it would call the provider a second time.
        provider.respond_with_text("should never reach");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(HandoffSentinelTool));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("route me")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Transfer {
                transfer: TransferOutcome::HandedOff,
            }
        );
        assert_eq!(
            provider.call_count(),
            1,
            "loop must exit after handoff — provider must not be called again"
        );

        let handoff_intents: Vec<_> = output
            .intents
            .iter()
            .filter(|i| matches!(i.kind, IntentKind::Handoff { .. }))
            .collect();
        assert_eq!(
            handoff_intents.len(),
            1,
            "exactly one Handoff intent must be emitted"
        );
        match &handoff_intents[0].kind {
            IntentKind::Handoff { operator, context } => {
                assert_eq!(operator.as_str(), "routing-agent");
                assert_eq!(
                    context.task.as_text().unwrap_or(""),
                    "needs routing",
                    "context.task must carry the handoff reason"
                );
            }
            _ => unreachable!(),
        }
    }

    /// Normal tool results must not trigger a handoff — loop continues to completion.
    #[tokio::test]
    async fn non_handoff_tool_result_continues() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("search", "c1", json!({ "query": "hi" }));
        provider.respond_with_text("all done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "search" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("search for hi")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        assert_eq!(
            provider.call_count(),
            2,
            "loop must continue after a non-handoff tool result"
        );
        assert!(
            output
                .intents
                .iter()
                .all(|i| !matches!(i.kind, IntentKind::Handoff { .. })),
            "no Handoff intent must be emitted for a normal tool result"
        );
    }

    // ── Concurrency tests ─────────────────────────────────────────────────────

    /// A tool that sleeps for a fixed duration before returning. Used to probe
    /// whether tool calls run concurrently or sequentially.
    struct SlowTool {
        name: &'static str,
        delay_ms: u64,
        hint: ToolConcurrencyHint,
    }

    impl ToolDyn for SlowTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "slow tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            let delay_ms = self.delay_ms;
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                Ok(json!("done"))
            })
        }
        fn concurrency_hint(&self) -> ToolConcurrencyHint {
            self.hint
        }
    }

    /// Two Shared tools with a 50 ms delay each must finish in less than the
    /// time they would take running sequentially (< 90 ms vs ~100 ms).
    #[tokio::test]
    async fn shared_tools_run_concurrently() {
        let provider = TestProvider::new();
        provider.respond_with_tool_calls(vec![
            ("slow_a", "c1", json!({})),
            ("slow_b", "c2", json!({})),
        ]);
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(SlowTool {
            name: "slow_a",
            delay_ms: 50,
            hint: ToolConcurrencyHint::Shared,
        }));
        tools.register(Arc::new(SlowTool {
            name: "slow_b",
            delay_ms: 50,
            hint: ToolConcurrencyHint::Shared,
        }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let start = Instant::now();
        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        // Sequential execution would take ≥ 100 ms. Parallel completes in ~50 ms.
        // Allow generous headroom for slow CI: must be well under 2× single-tool time.
        assert!(
            elapsed < Duration::from_millis(90),
            "expected parallel execution (< 90 ms) but took {elapsed:?}"
        );
    }

    /// When ANY tool in a turn is Exclusive, all calls fall back to sequential.
    /// Verify that the loop still produces the correct output and both tool
    /// results are injected — correctness of the fallback path.
    #[tokio::test]
    async fn exclusive_tool_runs_sequentially() {
        let provider = TestProvider::new();
        provider.respond_with_tool_calls(vec![
            ("exclusive_tool", "c1", json!({})),
            ("shared_tool", "c2", json!({})),
        ]);
        provider.respond_with_text("done");

        let mut tools = ToolRegistry::new();
        // Exclusive is the default; this tool would prevent parallel dispatch.
        tools.register(Arc::new(SlowTool {
            name: "exclusive_tool",
            delay_ms: 10,
            hint: ToolConcurrencyHint::Exclusive,
        }));
        tools.register(Arc::new(SlowTool {
            name: "shared_tool",
            delay_ms: 10,
            hint: ToolConcurrencyHint::Shared,
        }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("go")))
            .await
            .unwrap();
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = react_loop(&mut ctx, &provider, &tools, &dispatch_ctx, &simple_config())
            .await
            .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed
            }
        );
        // Both tool results must appear in context so the second inference sees them.
        assert_eq!(
            ctx.metrics.tool_calls_total, 2,
            "both tools must run when fallback is sequential"
        );
    }
}
