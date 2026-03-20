//! Streaming ReAct loop — [`stream_react_loop()`].
//!
//! Like [`react_loop()`](crate::react_loop) but streams inference output
//! via a callback. Tool dispatch, approval checking, and rule firing
//! work identically.

use crate::boundary::StreamInferBoundary;
use crate::compile::CompileConfig;
use crate::context::Context;
use crate::error::EngineError;
use crate::ops::response::AppendResponse;
use crate::ops::tool::{ExecuteTool, format_tool_result};
use crate::react::{ReactLoopConfig, check_approval, check_exit, format_tool_error, is_handoff_sentinel};
use futures_util::StreamExt;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::duration::DurationMs;
use layer0::effect::{Effect, EffectKind, HandoffContext};
use layer0::id::OperatorId;
use layer0::operator::{ExitReason, OperatorMetadata, OperatorOutput};
use skg_tool::ToolRegistry;
use skg_turn::infer::InferResponse;
use skg_turn::provider::Provider;
use skg_turn::stream::StreamEvent;

///
/// Same flow as [`react_loop()`](crate::react_loop) but streams inference
/// output through `on_event`. Tool dispatch, approval checking, budget guards,
/// and all rules work identically.
///
/// The `on_event` callback receives non-[`StreamEvent::Done`] events
/// (TextDelta, ThinkingDelta, ToolCallStart, ToolCallDelta, Usage) immediately
/// as they arrive from the provider stream. `Done` is deferred: it fires only
/// after the response has been committed to context via [`AppendResponse`], so
/// when the consumer receives `Done`, the turn is fully readable from context.
///
/// ```ignore
/// let output = stream_react_loop(
///     &mut ctx, &provider, &tools, &dispatch_ctx, &config,
///     |event| match event {
///         StreamEvent::TextDelta(text) => print!("{text}"),
///         _ => {}
///     },
/// ).await?;
/// ```
pub async fn stream_react_loop<P: Provider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    dispatch_ctx: &DispatchContext,
    config: &ReactLoopConfig,
    on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
) -> Result<OperatorOutput, EngineError> {
    loop {
        // Enter StreamInferBoundary: drains pending interventions + fires Before rules.
        // Compile AFTER so any context mutations appear in the request.
        if let Err(err) = ctx.enter_boundary::<StreamInferBoundary>().await {
            return structured_exit_output(err, ctx);
        }

        // Phase 1: Compile context (re-filter tools each turn, after before rules)
        let compile_config = config.compile_config(tools, ctx);
        let infer_request = build_infer_request(ctx, &compile_config);

        // Phase 2: Stream inference — emit deltas immediately, defer Done until committed.
        let mut infer_stream = provider
            .infer_stream(infer_request)
            .await
            .map_err(EngineError::Provider)?;

        let mut done_response: Option<skg_turn::InferResponse> = None;
        while let Some(event) = infer_stream.next().await {
            let event = event.map_err(EngineError::Provider)?;
            match event {
                StreamEvent::Done(resp) => {
                    // Stash; emit Done only after context commit succeeds.
                    done_response = Some(resp);
                }
                other => {
                    // TextDelta, ThinkingDelta, ToolCallStart, ToolCallDelta, Usage:
                    // no commit dependency — emit immediately.
                    on_event(other);
                }
            }
        }
        let response = done_response.ok_or_else(|| {
            EngineError::Provider(skg_turn::ProviderError::InvalidResponse(
                "streaming inference ended without Done event".into(),
            ))
        })?;

        // Exit StreamInferBoundary: fires After rules (before we commit events)
        if let Err(err) = ctx.exit_boundary::<StreamInferBoundary>().await {
            return structured_exit_output(err, ctx);
        }

        // Phase 3: Append response to context (rules fire)
        if let Err(err) = ctx.run(AppendResponse::new(response.clone())).await {
            return structured_exit_output(err, ctx);
        }
        ctx.metrics.turns_completed += 1;

        // Done is a commit signal: context is written, After rules fired.
        // Emit it only now so the consumer's invariant holds.
        on_event(StreamEvent::Done(response.clone()));

        // Phase 4: Check if model is done
        if !response.has_tool_calls() {
            let exit = check_exit(&response.stop_reason);
            return Ok(make_output(response, exit, ctx));
        }

        // Phase 5: Check tool approval
        let tool_calls = response.tool_calls.clone();
        let approval_effects = check_approval(&tool_calls, tools);

        if !approval_effects.is_empty() {
            ctx.extend_effects(approval_effects);
            return Ok(make_output(response, ExitReason::AwaitingApproval, ctx));
        }

        // Phase 6: Dispatch tool calls (non-streaming)
        for call in &tool_calls {
            let result_str = match ctx
                .run(ExecuteTool::new(
                    call.clone(),
                    tools.clone(),
                    dispatch_ctx.clone(),
                ))
                .await
            {
                Ok(value) => {
                    // Detect HandoffTool sentinel BEFORE formatting.
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
                        ctx.push_effect(Effect::new(EffectKind::Handoff {
                            operator: OperatorId::from(target.as_str()),
                            context: HandoffContext {
                                task: Content::text(reason),
                                history: None,
                                metadata: None,
                            },
                        }));
                        return Ok(make_context_output(
                            Content::text(""),
                            ExitReason::HandedOff,
                            ctx,
                        ));
                    }
                    match &config.tool_result_formatter {
                        Some(f) => f(&call.name, &value),
                        None => format_tool_result(&value),
                    }
                }
                Err(EngineError::Exit { reason, .. }) => {
                    return Ok(make_context_output(Content::text(""), reason, ctx));
                }
                Err(e) => match &config.tool_error_formatter {
                    Some(f) => f(&call.name, &e.to_string()),
                    None => format_tool_error(&e),
                },
            };

            let result_msg =
                InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
            if let Err(err) = ctx.inject_message(result_msg).await {
                return structured_exit_output(err, ctx);
            }
        }
    }
}

fn structured_exit_output(
    err: EngineError,
    ctx: &mut Context,
) -> Result<OperatorOutput, EngineError> {
    match err {
        EngineError::Exit { reason, .. } => Ok(make_context_output(Content::text(""), reason, ctx)),
        other => Err(other),
    }
}

fn make_context_output(message: Content, exit: ExitReason, ctx: &mut Context) -> OperatorOutput {
    let mut output = OperatorOutput::new(message, exit);
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = ctx.metrics.tokens_in;
    meta.tokens_out = ctx.metrics.tokens_out;
    meta.cost = ctx.metrics.cost;
    meta.turns_used = ctx.metrics.turns_completed;
    meta.duration = DurationMs::from_millis(ctx.metrics.elapsed_ms());
    output.metadata = meta;
    output.effects = ctx.drain_effects();
    output
}

fn make_output(response: InferResponse, exit: ExitReason, ctx: &mut Context) -> OperatorOutput {
    make_context_output(response.content, exit, ctx)
}

/// Compile the context into an [`InferRequest`](skg_turn::InferRequest) for streaming.
fn build_infer_request(ctx: &Context, config: &CompileConfig) -> skg_turn::InferRequest {
    ctx.compile(config).request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::op::ContextOp;
    use crate::rules::{BudgetGuard, BudgetGuardConfig};
    use crate::{Rule, StreamInferBoundary};
    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use layer0::id::OperatorId;
    use layer0::{DispatchContext, DispatchId};
    use serde_json::json;
    use skg_tool::{ToolDyn, ToolError, ToolRegistry};
    use skg_turn::provider::ProviderError;
    use skg_turn::stream::StreamEvent;
    use skg_turn::test_utils::TestProvider;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

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

    struct HaltBeforeStreamInference;

    #[async_trait]
    impl ContextOp for HaltBeforeStreamInference {
        type Output = ();

        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Err(EngineError::Halted {
                reason: "blocked before stream inference".into(),
            })
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

    /// A provider that always fails on both `infer` and `infer_stream`.
    struct AlwaysErrorStreamProvider;

    impl skg_turn::provider::Provider for AlwaysErrorStreamProvider {
        async fn infer(
            &self,
            _request: skg_turn::InferRequest,
        ) -> Result<skg_turn::InferResponse, ProviderError> {
            Err(ProviderError::TransientError {
                message: "stream failure".into(),
                status: None,
            })
        }
    }

    #[tokio::test]
    async fn stream_react_before_boundary_rule_mutates_request_before_provider_call() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::before::<StreamInferBoundary>(
            "mark before stream inference",
            100,
            PushMarker("before stream marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let request = provider
            .last_request()
            .expect("provider should record request");
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.text_content() == "before stream marker"),
            "before rule must mutate context before stream request compilation and provider send"
        );
    }

    #[tokio::test]
    async fn stream_react_after_boundary_rule_runs_after_success_only() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::after::<StreamInferBoundary>(
            "mark after stream inference",
            100,
            PushMarker("after stream marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let request = provider
            .last_request()
            .expect("provider should record request");
        assert!(
            !request
                .messages
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after rule must not mutate the request that was already sent to the provider"
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after rule must run after a successful provider call"
        );
    }

    struct ExitBeforeStreamAppend;

    #[async_trait]
    impl ContextOp for ExitBeforeStreamAppend {
        type Output = ();

        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Err(EngineError::Exit {
                reason: ExitReason::Timeout,
                detail: "append boundary timeout".into(),
            })
        }
    }

    #[tokio::test]
    async fn stream_react_loop_does_not_emit_done_before_append_commit() {
        let provider = TestProvider::new();
        provider.respond_with_text("hello before append exit");

        let mut ctx =
            Context::with_rules(vec![Rule::before::<crate::ops::response::AppendResponse>(
                "exit before append commit",
                100,
                ExitBeforeStreamAppend,
            )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let events = Arc::new(Mutex::new(Vec::<StreamEvent>::new()));
        let events_clone = Arc::clone(&events);

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            move |event| events_clone.lock().unwrap().push(event),
        )
        .await
        .expect("append exit should return structured operator output");

        assert_eq!(output.exit_reason, ExitReason::Timeout);
        let captured = events.lock().unwrap();
        assert!(
            !captured.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "Done must not be emitted before the streamed turn is committed to context"
        );
    }

    struct ExitAfterStreamInference;

    #[async_trait]
    impl ContextOp for ExitAfterStreamInference {
        type Output = ();

        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Err(EngineError::Exit {
                reason: ExitReason::Timeout,
                detail: "stream boundary timeout".into(),
            })
        }
    }

    #[tokio::test]
    async fn stream_react_loop_does_not_emit_done_before_after_boundary_exit() {
        let provider = TestProvider::new();
        provider.respond_with_text("hello before exit");

        let mut ctx = Context::with_rules(vec![Rule::after::<StreamInferBoundary>(
            "exit after stream inference",
            100,
            ExitAfterStreamInference,
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let events = Arc::new(Mutex::new(Vec::<StreamEvent>::new()));
        let events_clone = Arc::clone(&events);

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            move |event| events_clone.lock().unwrap().push(event),
        )
        .await
        .expect("stream exit should return structured operator output");

        assert_eq!(output.exit_reason, ExitReason::Timeout);
        let captured = events.lock().unwrap();
        assert!(
            !captured.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "Done must not be emitted before the after-boundary exit is accepted"
        );
    }

    #[tokio::test]
    async fn stream_react_after_boundary_rule_does_not_run_on_provider_error() {
        let provider = AlwaysErrorStreamProvider;

        let mut ctx = Context::with_rules(vec![Rule::after::<StreamInferBoundary>(
            "mark after stream inference",
            100,
            PushMarker("after stream marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            EngineError::Provider(ProviderError::TransientError { .. })
        ));
        assert!(
            !ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after rule must not run when streaming provider inference fails"
        );
    }

    async fn assert_stream_budget_exit_before_provider_call(
        mutate_ctx: impl FnOnce(&mut Context),
        config: BudgetGuardConfig,
        expected_exit: ExitReason,
    ) {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<StreamInferBoundary>(
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

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .expect("budget exits should return structured operator output");

        assert_eq!(output.exit_reason, expected_exit);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn stream_react_loop_returns_structured_budget_exhausted_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
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
    async fn stream_react_loop_returns_structured_max_turns_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
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
    async fn stream_react_loop_returns_structured_timeout_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
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
    async fn stream_react_loop_halts_before_provider_call_on_stream_boundary_rule() {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<StreamInferBoundary>(
            "halt before stream inference",
            100,
            HaltBeforeStreamInference,
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(err, EngineError::Halted { .. }));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn stream_react_loop_simple_text() {
        let provider = TestProvider::new();
        provider.respond_with_text("hello streaming!");

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            move |event| {
                let label = match &event {
                    StreamEvent::TextDelta(t) => format!("text:{t}"),
                    StreamEvent::Done(_) => "done".into(),
                    _ => "other".into(),
                };
                events_clone.lock().unwrap().push(label);
            },
        )
        .await
        .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);

        let captured = events.lock().unwrap();
        // TestProvider uses Provider's default infer_stream impl, which wraps the
        // final response in a single Done event rather than native text deltas.
        assert!(captured.iter().any(|e| e == "done"));
    }

    #[tokio::test]
    async fn stream_react_loop_with_tool_call() {
        let provider = TestProvider::new();
        provider
            .respond_with_tool_call("echo", "c1", json!({"msg": "hi"}));
        provider.respond_with_text("echoed!");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "echo" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("echo something")))
            .await
            .unwrap();

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.metadata.turns_used, 2);
    }

    #[tokio::test]
    async fn stream_emits_text_delta_before_done() {
        // Provider that emits TextDelta then Done, verifying real-time ordering.
        struct DeltaStreamProvider;

        impl skg_turn::provider::Provider for DeltaStreamProvider {
            async fn infer(
                &self,
                _request: skg_turn::InferRequest,
            ) -> Result<skg_turn::InferResponse, skg_turn::provider::ProviderError> {
                Ok(skg_turn::test_utils::make_text_response("hello"))
            }

            async fn infer_stream(
                &self,
                _request: skg_turn::InferRequest,
            ) -> Result<skg_turn::stream::InferStream, skg_turn::provider::ProviderError> {
                let response = skg_turn::test_utils::make_text_response("hello");
                let events: Vec<Result<StreamEvent, skg_turn::provider::ProviderError>> = vec![
                    Ok(StreamEvent::TextDelta("hello".into())),
                    Ok(StreamEvent::Done(response)),
                ];
                Ok(skg_turn::stream::InferStream::new(futures_util::stream::iter(events)))
            }
        }

        let provider = DeltaStreamProvider;
        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let dispatch_ctx =
            DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            move |event| {
                let label = match &event {
                    StreamEvent::TextDelta(t) => format!("text:{t}"),
                    StreamEvent::Done(_) => "done".into(),
                    _ => "other".into(),
                };
                events_clone.lock().unwrap().push(label);
            },
        )
        .await
        .unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        let captured = events.lock().unwrap();
        let text_pos = captured
            .iter()
            .position(|e| e == "text:hello")
            .expect("TextDelta must be emitted");
        let done_pos = captured
            .iter()
            .position(|e| e == "done")
            .expect("Done must be emitted");
        assert!(text_pos < done_pos, "TextDelta must arrive before Done");
    }
}
