use layer0::context::{Message, Role};
use layer0::Content;
use skg_context_engine::{CompileConfig, Context, InferBoundary, StreamInferBoundary};

// ── Provider deltas projected only at locked semantic boundaries ─────────────
//
// The InferBoundary and StreamInferBoundary markers exist as rule targets.
// Provider interactions are governed through these explicit boundary types.
// No provider delta leaks into Context without passing through one of these
// semantic governance points.

#[test]
fn infer_boundary_marker_exists() {
    // InferBoundary is the typed governance marker for non-streaming inference.
    let _: InferBoundary = InferBoundary;
}

#[test]
fn stream_infer_boundary_marker_exists() {
    // StreamInferBoundary is the typed governance marker for streaming inference.
    let _: StreamInferBoundary = StreamInferBoundary;
}

#[test]
fn context_compile_does_not_mutate_context() {
    // compile() snapshots context for inference without mutating it.
    // This is the assembly → boundary contract: context is readable,
    // not consumed by the projection step.
    let mut ctx = Context::new();
    ctx.push_message(Message::new(Role::User, Content::text("hello")));

    let config = CompileConfig::default();
    let _compiled = ctx.compile(&config);

    // Context is unchanged after compile.
    assert_eq!(ctx.messages().len(), 1);
    assert_eq!(ctx.messages()[0].text_content(), "hello");
}

#[test]
fn context_compile_produces_request_with_all_messages() {
    let mut ctx = Context::new();
    ctx.push_message(Message::new(Role::User, Content::text("first")));
    ctx.push_message(Message::new(Role::Assistant, Content::text("response")));

    let config = CompileConfig::default();
    let compiled = ctx.compile(&config);

    // The compiled request captures all context messages.
    assert_eq!(compiled.request.messages.len(), 2);
}
