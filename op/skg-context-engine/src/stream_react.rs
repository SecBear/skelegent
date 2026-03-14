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
use layer0::duration::DurationMs;
use layer0::operator::{ExitReason, OperatorMetadata, OperatorOutput};
use layer0::DispatchContext;
use skg_tool::ToolRegistry;
use skg_turn::infer::InferResponse;
use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest};
use std::any::TypeId;

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
///     &mut ctx, &provider, &tools, &dispatch_ctx, &config,
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
    dispatch_ctx: &DispatchContext,
    config: &ReactLoopConfig,
    on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
) -> Result<OperatorOutput, EngineError> {
    let on_event = std::sync::Arc::new(on_event);
    loop {
        // Phase 1: Compile context (re-filter tools each turn)
        let compile_config = config.compile_config(tools, ctx);
        let request = build_stream_request(ctx, &compile_config);

        // Fire Before<StreamInferBoundary> rules (e.g. budget guard)
        ctx.fire_before_rules(TypeId::of::<StreamInferBoundary>())
            .await?;

        // Phase 2: Stream inference
        let cb = std::sync::Arc::clone(&on_event);
        let response = provider
            .infer_stream(request, move |e| cb(e))
            .await
            .map_err(EngineError::Provider)?;

        // Fire After<StreamInferBoundary> rules
        ctx.fire_after_rules(TypeId::of::<StreamInferBoundary>())
            .await?;

        // Phase 3: Append response to context (rules fire)
        ctx.run(AppendResponse::new(response.clone())).await?;
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
                    dispatch_ctx.clone(),
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
    output.effects = ctx.effects().to_vec();
    output
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
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use layer0::id::OperatorId;
    use serde_json::json;
    use layer0::{DispatchContext, DispatchId};
    use skg_tool::{ToolDyn, ToolError, ToolRegistry};
    use skg_turn::stream::{StreamEvent, StreamProvider, StreamRequest, infer_stream_fallback};
    use skg_turn::test_utils::TestProvider;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

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
            _ctx: &DispatchContext,
        ) -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async { Ok(json!("mock result")) })
        }
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
}
