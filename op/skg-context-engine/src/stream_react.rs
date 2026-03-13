//! Streaming ReAct loop — [`stream_react_loop()`].
//!
//! Like [`react_loop()`](crate::react_loop) but streams inference output
//! via a callback. Tool dispatch, approval checking, and rule firing
//! work identically.
//!
//! For providers that don't implement [`StreamProvider`], use
//! [`infer_stream_fallback()`](skg_turn::stream::infer_stream_fallback)
//! to get a non-streaming fallback that still works with this function.

use crate::boundary::StreamInferBoundary;
use crate::compile::CompileConfig;
use crate::context::Context;
use crate::error::EngineError;
use crate::ops::response::AppendResponse;
use crate::ops::tool::ExecuteTool;
use crate::react::{ReactLoopConfig, check_approval, check_exit, format_tool_error};
use layer0::content::Content;
use layer0::duration::DurationMs;
use layer0::operator::{ExitReason, OperatorMetadata, OperatorOutput};
use skg_tool::{ToolCallContext, ToolRegistry};
use skg_turn::infer::InferResponse;
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};

async fn infer_stream_once<P, F>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    config: &ReactLoopConfig,
    on_event: std::sync::Arc<F>,
) -> Result<InferResponse, EngineError>
where
    P: StreamProvider,
    F: Fn(StreamEvent) + Send + Sync + 'static,
{
    ctx.enter_boundary::<StreamInferBoundary>().await?;

    let compile_config = config.compile_config(tools, ctx);
    let request = build_stream_request(ctx, &compile_config);
    let cb = std::sync::Arc::clone(&on_event);

    let response = provider
        .infer_stream(request, move |event| cb(event))
        .await
        .map_err(EngineError::Provider)?;

    ctx.exit_boundary::<StreamInferBoundary>().await?;
    Ok(response)
}

/// Run the streaming ReAct loop.
///
/// Same flow as [`react_loop()`](crate::react_loop) but streams inference
/// output through `on_event`. Tool dispatch, approval checking, budget guards,
/// and all rules work identically.
///
/// The `on_event` callback receives [`StreamEvent`]s during inference. After
/// streaming completes, the response is appended to context and tool dispatch
/// proceeds normally (non-streaming).
///
/// ```ignore
/// let output = stream_react_loop(
///     &mut ctx, &provider, &tools, &tool_ctx, &config,
///     |event| match event {
///         StreamEvent::TextDelta(text) => print!("{text}"),
///         _ => {}
///     },
/// ).await?;
/// ```
pub async fn stream_react_loop<P: StreamProvider>(
    ctx: &mut Context,
    provider: &P,
    tools: &ToolRegistry,
    tool_ctx: &ToolCallContext,
    config: &ReactLoopConfig,
    on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
) -> Result<OperatorOutput, EngineError> {
    let on_event = std::sync::Arc::new(on_event);
    loop {
        // Phase 1: Stream inference behind the governed boundary
        let response = match infer_stream_once(
            ctx,
            provider,
            tools,
            config,
            std::sync::Arc::clone(&on_event),
        )
        .await
        {
            Ok(response) => response,
            Err(err) => return structured_exit_output(err, ctx),
        };

        // Phase 3: Append response to context (rules fire)
        if let Err(err) = ctx.run(AppendResponse::new(response.clone())).await {
            return structured_exit_output(err, ctx);
        }
        ctx.metrics.turns_completed += 1;

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

            let result_msg =
                InferResponse::tool_result_message(&call.id, &call.name, result_str, false);
            if let Err(err) = ctx.inject_message(result_msg).await {
                return structured_exit_output(err, ctx);
            }
        }
    }
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

fn make_output(response: InferResponse, exit: ExitReason, ctx: &Context) -> OperatorOutput {
    make_context_output(response.content, exit, ctx)
}

fn build_stream_request(ctx: &Context, config: &CompileConfig) -> StreamRequest {
    // Build InferRequest then convert
    let compiled = ctx.compile(config);
    StreamRequest::from(compiled.request)
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
    use serde_json::json;
    use skg_tool::{ToolCallContext, ToolDyn, ToolError, ToolRegistry};
    use skg_turn::provider::ProviderError;
    use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest, infer_stream_fallback};
    use skg_turn::test_utils::TestProvider;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // A wrapper that makes TestProvider implement StreamProvider via fallback
    struct FallbackStreamProvider {
        inner: TestProvider,
    }

    impl FallbackStreamProvider {
        fn new() -> Self {
            Self {
                inner: TestProvider::new(),
            }
        }
    }

    impl skg_turn::provider::Provider for FallbackStreamProvider {
        fn infer(
            &self,
            request: skg_turn::InferRequest,
        ) -> impl std::future::Future<
            Output = Result<skg_turn::InferResponse, skg_turn::ProviderError>,
        > + Send {
            self.inner.infer(request)
        }
    }

    impl StreamProvider for FallbackStreamProvider {
        fn infer_stream(
            &self,
            request: StreamRequest,
            on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
        ) -> impl std::future::Future<
            Output = Result<skg_turn::InferResponse, skg_turn::ProviderError>,
        > + Send {
            infer_stream_fallback(&self.inner, request, on_event)
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

    impl StreamProvider for AlwaysErrorStreamProvider {
        async fn infer_stream(
            &self,
            _request: StreamRequest,
            _on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
        ) -> Result<skg_turn::InferResponse, ProviderError> {
            Err(ProviderError::TransientError {
                message: "stream failure".into(),
                status: None,
            })
        }
    }

    #[tokio::test]
    async fn stream_react_before_boundary_rule_mutates_request_before_provider_call() {
        let provider = FallbackStreamProvider::new();
        provider.inner.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::before::<StreamInferBoundary>(
            "mark before stream inference",
            100,
            PushMarker("before stream marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
            .inner
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
        let provider = FallbackStreamProvider::new();
        provider.inner.respond_with_text("done");

        let mut ctx = Context::with_rules(vec![Rule::after::<StreamInferBoundary>(
            "mark after stream inference",
            100,
            PushMarker("after stream marker"),
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
            .inner
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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
        let provider = FallbackStreamProvider::new();
        provider.inner.respond_with_text("should never be used");

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
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
        assert_eq!(provider.inner.call_count(), 0);
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
        let provider = FallbackStreamProvider::new();
        provider.inner.respond_with_text("should never be used");

        let mut ctx = Context::with_rules(vec![Rule::before::<StreamInferBoundary>(
            "halt before stream inference",
            100,
            HaltBeforeStreamInference,
        )]);
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
        assert_eq!(provider.inner.call_count(), 0);
    }

    #[tokio::test]
    async fn stream_react_loop_simple_text() {
        let provider = FallbackStreamProvider::new();
        provider.inner.respond_with_text("hello streaming!");

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hi")))
            .await
            .unwrap();

        let tools = ToolRegistry::new();
        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let output = stream_react_loop(
            &mut ctx,
            &provider,
            &tools,
            &tool_ctx,
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
        assert!(captured.iter().any(|e| e.starts_with("text:")));
        assert!(captured.iter().any(|e| e == "done"));
    }

    #[tokio::test]
    async fn stream_react_loop_with_tool_call() {
        let provider = FallbackStreamProvider::new();
        provider
            .inner
            .respond_with_tool_call("echo", "c1", json!({"msg": "hi"}));
        provider.inner.respond_with_text("echoed!");

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(MockTool { name: "echo" }));

        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("echo something")))
            .await
            .unwrap();

        let tool_ctx = ToolCallContext::new(OperatorId::from("test"));

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
        assert_eq!(output.metadata.turns_used, 2);
    }
}
