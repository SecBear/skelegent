//! Streaming ReAct loop — [`stream_react_loop()`].
//!
//! Like [`react_loop()`](crate::react_loop) but streams inference output
//! via a callback. Tool dispatch, approval checking, and pipeline middleware
//! work identically.

use crate::compile::CompileConfig;
use crate::context::Context;
use crate::error::EngineError;
use crate::pipeline::Pipeline;
use crate::react::{
    ReactLoopConfig, check_approval, check_exit, execute_tool, format_tool_error,
    format_tool_result, is_handoff_sentinel,
};
use futures_util::StreamExt;
use layer0::DispatchContext;
use layer0::content::Content;
use layer0::duration::DurationMs;
use layer0::id::OperatorId;
use layer0::intent::{HandoffContext, Intent, IntentKind};
#[cfg(test)]
use layer0::operator::{LimitReason, TerminalOutcome};
use layer0::operator::{OperatorMetadata, OperatorOutput, Outcome, TransferOutcome};
use layer0::wait::WaitReason;
use skg_tool::ToolRegistry;
use skg_turn::infer::InferResponse;
use skg_turn::provider::Provider;
use skg_turn::stream::StreamEvent;

///
/// Same flow as [`react_loop()`](crate::react_loop) but streams inference
/// output through `on_event`. Tool dispatch, approval checking, budget guards,
/// and pipeline middleware work identically.
///
/// The `on_event` callback receives non-[`StreamEvent::Done`] events
/// (TextDelta, ThinkingDelta, ToolCallStart, ToolCallDelta, Usage) immediately
/// as they arrive from the provider stream. `Done` is deferred: it fires only
/// after the response has been committed to context via
/// [`Context::append_response`], so when the consumer receives `Done`, the
/// turn is fully readable from context.
///
/// ```ignore
/// let output = stream_react_loop(
///     &mut ctx, &provider, &tools, &dispatch_ctx, &config, &pipeline,
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
    pipeline: &Pipeline,
    on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
) -> Result<OperatorOutput, EngineError> {
    loop {
        // Before phase: run pipeline before_send middleware (context assembly).
        // Compile AFTER so any mutations appear in the request.
        if let Err(err) = pipeline.run_before(ctx).await {
            return structured_exit_output(err, ctx);
        }

        // Phase 1: Compile context (re-filter tools each turn, after before middleware)
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

        // After phase: run pipeline after_send middleware before committing response.
        // If it errors, Done is suppressed — consumer invariant preserved.
        if let Err(err) = pipeline.run_after(ctx).await {
            return structured_exit_output(err, ctx);
        }

        // Phase 3: Append response to context (sync, infallible).
        ctx.append_response(&response);
        ctx.metrics.turns_completed += 1;

        // Done is the commit signal: context is written, after middleware ran.
        // Emit only now so the consumer's invariant holds.
        on_event(StreamEvent::Done(response.clone()));

        // Phase 4: Check if model is done
        if !response.has_tool_calls() {
            let exit = check_exit(&response.stop_reason);
            return Ok(make_output(response, exit, ctx));
        }

        // Phase 5: Check tool approval
        let tool_calls = response.tool_calls.clone();
        let approval_intents = check_approval(&tool_calls, tools);

        if !approval_intents.is_empty() {
            ctx.extend_intents(approval_intents);
            return Ok(make_output(
                response,
                Outcome::Suspended {
                    reason: WaitReason::Approval,
                },
                ctx,
            ));
        }

        // Phase 6: Dispatch tool calls (non-streaming, sequential)
        for call in &tool_calls {
            let result_str = match execute_tool(call, tools, dispatch_ctx, ctx).await {
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
                    match &config.tool_result_formatter {
                        Some(f) => f(&call.name, &value),
                        None => format_tool_result(&value),
                    }
                }
                Err(EngineError::Exit { outcome, .. }) => {
                    return Ok(make_context_output(Content::text(""), outcome, ctx));
                }
                Err(e) => match &config.tool_error_formatter {
                    Some(f) => f(&call.name, &e.to_string()),
                    None => format_tool_error(&e),
                },
            };

            let result_msg =
                InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
            ctx.inject_message(result_msg);
        }
    }
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

fn make_output(response: InferResponse, outcome: Outcome, ctx: &mut Context) -> OperatorOutput {
    make_context_output(response.content, outcome, ctx)
}

/// Compile the context into an [`InferRequest`](skg_turn::InferRequest) for streaming.
fn build_infer_request(ctx: &Context, config: &CompileConfig) -> skg_turn::InferRequest {
    ctx.compile(config).request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::middleware::Middleware;
    use crate::pipeline::Pipeline;
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

    struct PushMarker(&'static str);
    impl crate::middleware::Middleware for PushMarker {
        async fn process(&self, ctx: &mut Context) -> Result<(), crate::error::EngineError> {
            ctx.push_message(Message::new(Role::System, Content::text(self.0)));
            Ok(())
        }
        fn name(&self) -> &str {
            self.0
        }
    }

    struct HaltMiddleware {
        error: crate::error::EngineError,
    }
    impl HaltMiddleware {
        fn halted(reason: &str) -> Self {
            Self {
                error: crate::error::EngineError::Halted {
                    reason: reason.into(),
                },
            }
        }
        fn exit(outcome: Outcome) -> Self {
            Self {
                error: crate::error::EngineError::Exit {
                    outcome,
                    detail: "test exit".into(),
                },
            }
        }
    }
    impl crate::middleware::Middleware for HaltMiddleware {
        async fn process(&self, _ctx: &mut Context) -> Result<(), crate::error::EngineError> {
            // Clone the error for each invocation (EngineError is not Clone, so reconstruct)
            match &self.error {
                crate::error::EngineError::Halted { reason } => {
                    Err(crate::error::EngineError::Halted {
                        reason: reason.clone(),
                    })
                }
                crate::error::EngineError::Exit { outcome, detail } => {
                    Err(crate::error::EngineError::Exit {
                        outcome: outcome.clone(),
                        detail: detail.clone(),
                    })
                }
                _ => unreachable!(),
            }
        }
        fn name(&self) -> &str {
            "halt"
        }
    }

    struct TrackingMarker {
        ran: Arc<Mutex<bool>>,
        label: &'static str,
    }
    impl crate::middleware::Middleware for TrackingMarker {
        async fn process(&self, ctx: &mut Context) -> Result<(), crate::error::EngineError> {
            ctx.push_message(Message::new(Role::System, Content::text(self.label)));
            *self.ran.lock().unwrap() = true;
            Ok(())
        }
        fn name(&self) -> &str {
            self.label
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

    /// Inline budget middleware for tests: mirrors the old BudgetGuard semantics.
    struct BudgetMiddleware {
        max_cost: Option<rust_decimal::Decimal>,
        max_turns: Option<u32>,
        max_duration: Option<Duration>,
    }

    impl Middleware for BudgetMiddleware {
        async fn process(&self, ctx: &mut Context) -> Result<(), EngineError> {
            if let Some(max_cost) = self.max_cost {
                if ctx.metrics.cost >= max_cost {
                    return Err(EngineError::Exit {
                        outcome: Outcome::Limited {
                            limit: LimitReason::BudgetExhausted,
                        },
                        detail: "cost limit exceeded".into(),
                    });
                }
            }
            if let Some(max_turns) = self.max_turns {
                if ctx.metrics.turns_completed >= max_turns {
                    return Err(EngineError::Exit {
                        outcome: Outcome::Limited {
                            limit: LimitReason::MaxTurns,
                        },
                        detail: "turn limit exceeded".into(),
                    });
                }
            }
            if let Some(max_duration) = self.max_duration {
                if ctx.metrics.elapsed_ms() >= max_duration.as_millis() as u64 {
                    return Err(EngineError::Exit {
                        outcome: Outcome::Limited {
                            limit: LimitReason::Timeout,
                        },
                        detail: "duration limit exceeded".into(),
                    });
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn stream_react_before_middleware_mutates_request_before_provider_call() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(PushMarker("before stream marker")));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &pipeline,
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
        let request = provider
            .last_request()
            .expect("provider should record request");
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.text_content() == "before stream marker"),
            "before middleware must mutate context before stream request compilation and provider send"
        );
    }

    #[tokio::test]
    async fn stream_react_after_middleware_runs_after_success_only() {
        let provider = TestProvider::new();
        provider.respond_with_text("done");

        let mut pipeline = Pipeline::new();
        pipeline.push_after(Box::new(PushMarker("after stream marker")));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &pipeline,
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
        let request = provider
            .last_request()
            .expect("provider should record request");
        assert!(
            !request
                .messages
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after middleware must not mutate the request that was already sent to the provider"
        );
        assert!(
            ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after middleware must run after a successful provider call"
        );
    }

    #[tokio::test]
    async fn stream_react_loop_does_not_emit_done_before_after_middleware_commit() {
        // If after_send middleware exits, Done must not be emitted — the turn
        // was not committed to context from the consumer's perspective.
        let provider = TestProvider::new();
        provider.respond_with_text("hello before after-exit");

        let mut pipeline = Pipeline::new();
        pipeline.push_after(Box::new(HaltMiddleware::exit(Outcome::Limited {
            limit: LimitReason::Timeout,
        })));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

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
            &pipeline,
            move |event| events_clone.lock().unwrap().push(event),
        )
        .await
        .expect("after-middleware exit should return structured operator output");

        assert_eq!(
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::Timeout,
            }
        );
        let captured = events.lock().unwrap();
        assert!(
            !captured.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "Done must not be emitted when after_send middleware exits before context commit"
        );
    }

    #[tokio::test]
    async fn stream_react_loop_does_not_emit_done_before_after_boundary_exit() {
        let provider = TestProvider::new();
        provider.respond_with_text("hello before exit");

        let mut pipeline = Pipeline::new();
        pipeline.push_after(Box::new(HaltMiddleware::exit(Outcome::Limited {
            limit: LimitReason::Timeout,
        })));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

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
            &pipeline,
            move |event| events_clone.lock().unwrap().push(event),
        )
        .await
        .expect("stream exit should return structured operator output");

        assert_eq!(
            output.outcome,
            Outcome::Limited {
                limit: LimitReason::Timeout,
            }
        );
        let captured = events.lock().unwrap();
        assert!(
            !captured.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "Done must not be emitted before the after-phase exit is accepted"
        );
    }

    #[tokio::test]
    async fn stream_react_after_middleware_does_not_run_on_provider_error() {
        let provider = AlwaysErrorStreamProvider;

        // Track whether after middleware ran.
        let ran = Arc::new(Mutex::new(false));
        let ran_clone = Arc::clone(&ran);

        let mut pipeline = Pipeline::new();
        pipeline.push_after(Box::new(TrackingMarker {
            ran: Arc::clone(&ran_clone),
            label: "after stream marker",
        }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &pipeline,
            |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            EngineError::Provider(ProviderError::TransientError { .. })
        ));
        assert!(
            !*ran.lock().unwrap(),
            "after middleware must not run when streaming provider inference fails"
        );
        assert!(
            !ctx.messages()
                .iter()
                .any(|message| message.text_content() == "after stream marker"),
            "after middleware must not inject messages when streaming provider inference fails"
        );
    }

    async fn assert_stream_budget_exit_before_provider_call(
        mutate_ctx: impl FnOnce(&mut Context),
        budget: BudgetMiddleware,
        expected_outcome: Outcome,
    ) {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(budget));

        let mut ctx = Context::new();
        mutate_ctx(&mut ctx);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &pipeline,
            |_| {},
        )
        .await
        .expect("budget exits should return structured operator output");

        assert_eq!(output.outcome, expected_outcome);
        assert_eq!(output.message.as_text(), Some(""));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn stream_react_loop_returns_structured_budget_exhausted_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
            |ctx| ctx.metrics.cost = rust_decimal::Decimal::new(250, 2),
            BudgetMiddleware {
                max_cost: Some(rust_decimal::Decimal::new(100, 2)),
                max_turns: None,
                max_duration: None,
            },
            Outcome::Limited {
                limit: LimitReason::BudgetExhausted,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn stream_react_loop_returns_structured_max_turns_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
            |ctx| ctx.metrics.turns_completed = 1,
            BudgetMiddleware {
                max_cost: None,
                max_turns: Some(1),
                max_duration: None,
            },
            Outcome::Limited {
                limit: LimitReason::MaxTurns,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn stream_react_loop_returns_structured_timeout_exit_before_provider_call() {
        assert_stream_budget_exit_before_provider_call(
            |ctx| ctx.metrics.start = Instant::now() - Duration::from_secs(5),
            BudgetMiddleware {
                max_cost: None,
                max_turns: None,
                max_duration: Some(Duration::from_secs(1)),
            },
            Outcome::Limited {
                limit: LimitReason::Timeout,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn stream_react_loop_halts_before_provider_call_on_before_middleware() {
        let provider = TestProvider::new();
        provider.respond_with_text("should never be used");

        let mut pipeline = Pipeline::new();
        pipeline.push_before(Box::new(HaltMiddleware::halted(
            "blocked before stream inference",
        )));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let tool_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let err = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
            &simple_config(),
            &pipeline,
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
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let pipeline = Pipeline::new();
        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &pipeline,
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

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );

        let captured = events.lock().unwrap();
        // TestProvider uses Provider's default infer_stream impl, which wraps the
        // final response in a single Done event rather than native text deltas.
        assert!(captured.iter().any(|e| e == "done"));
    }

    #[tokio::test]
    async fn stream_react_loop_with_tool_call() {
        let provider = TestProvider::new();
        provider.respond_with_tool_call("echo", "c1", json!({"msg": "hi"}));
        provider.respond_with_text("echoed!");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "echo" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("echo something")));

        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));

        let pipeline = Pipeline::new();
        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &pipeline,
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
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
            ) -> Result<skg_turn::stream::InferStream, skg_turn::provider::ProviderError>
            {
                let response = skg_turn::test_utils::make_text_response("hello");
                let events: Vec<Result<StreamEvent, skg_turn::provider::ProviderError>> = vec![
                    Ok(StreamEvent::TextDelta("hello".into())),
                    Ok(StreamEvent::Done(response)),
                ];
                Ok(skg_turn::stream::InferStream::new(
                    futures_util::stream::iter(events),
                ))
            }
        }

        let provider = DeltaStreamProvider;
        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")));

        let tools = ToolRegistry::new();
        let dispatch_ctx = DispatchContext::new(DispatchId::from("test"), OperatorId::from("test"));
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let pipeline = Pipeline::new();
        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &dispatch_ctx,
            &simple_config(),
            &pipeline,
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

        assert_eq!(
            output.outcome,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            }
        );
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
