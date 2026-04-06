use layer0::Content;
use layer0::context::{Message, Role};
use skg_context_engine::{CompileConfig, Context, Middleware, Pipeline};

struct PushMarker(&'static str);
impl Middleware for PushMarker {
    async fn process(&self, ctx: &mut Context) -> Result<(), skg_context_engine::EngineError> {
        ctx.push_message(Message::new(Role::System, Content::text(self.0)));
        Ok(())
    }
    fn name(&self) -> &str {
        self.0
    }
}

// ── Pipeline replaces typed boundary markers ─────────────────────────────────

#[tokio::test]
async fn pipeline_before_send_runs_before_inference() {
    let mut pipeline = Pipeline::new();
    pipeline.push_before(Box::new(PushMarker("before")));

    let mut ctx = Context::new();
    pipeline.run_before(&mut ctx).await.unwrap();
    assert_eq!(ctx.messages().len(), 1);
    assert_eq!(ctx.messages()[0].text_content(), "before");
}

#[tokio::test]
async fn pipeline_after_send_runs_after_inference() {
    let mut pipeline = Pipeline::new();
    pipeline.push_after(Box::new(PushMarker("after")));

    let mut ctx = Context::new();
    pipeline.run_after(&mut ctx).await.unwrap();
    assert_eq!(ctx.messages().len(), 1);
    assert_eq!(ctx.messages()[0].text_content(), "after");
}

#[test]
fn context_compile_does_not_mutate_context() {
    let mut ctx = Context::new();
    ctx.push_message(Message::new(Role::User, Content::text("hello")));

    let config = CompileConfig::default();
    let _compiled = ctx.compile(&config);

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

    assert_eq!(compiled.request.messages.len(), 2);
}
