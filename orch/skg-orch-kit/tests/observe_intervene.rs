use async_trait::async_trait;
use layer0::content::Content;
use layer0::context::{Message, Role};
use skg_context_engine::{Context, ContextMutation, ContextOp};
use skg_orch_kit::{
    ContextIntervenor, ContextObserver, InterventionSendError, Observation, ObservationTry,
};
use tokio::sync::{broadcast, mpsc};

#[tokio::test]
async fn observer_drains_stream_events() {
    let (tx, _) = broadcast::channel(8);
    let mut observer = ContextObserver::subscribe(&tx);

    let mut ctx = Context::new();
    ctx.with_stream(tx);
    ctx.push_message(Message::new(Role::System, Content::text("one")));
    ctx.push_message(Message::new(Role::Assistant, Content::text("two")));

    let drained = observer.drain_available();
    assert_eq!(drained.events.len(), 2);
    assert!(matches!(drained.status, ObservationTry::Empty));
    assert!(matches!(
        &drained.events[0].mutation,
        ContextMutation::MessagePushed(message) if message.text_content() == "one"
    ));
    assert!(matches!(
        &drained.events[1].mutation,
        ContextMutation::MessagePushed(message) if message.text_content() == "two"
    ));
}

#[tokio::test]
async fn observer_reports_closed_channel() {
    let (tx, _) = broadcast::channel(8);
    let mut observer = ContextObserver::subscribe(&tx);
    drop(tx);

    assert!(matches!(observer.recv().await, Observation::Closed));
}

struct InjectSupervisorNote(&'static str);

#[async_trait]
impl ContextOp for InjectSupervisorNote {
    type Output = ();

    async fn execute(
        &self,
        ctx: &mut Context,
    ) -> Result<Self::Output, skg_context_engine::EngineError> {
        ctx.push_message(Message::new(Role::System, Content::text(self.0)));
        Ok(())
    }
}

struct PushUserMessage(&'static str);

#[async_trait]
impl ContextOp for PushUserMessage {
    type Output = ();

    async fn execute(
        &self,
        ctx: &mut Context,
    ) -> Result<Self::Output, skg_context_engine::EngineError> {
        ctx.push_message(Message::new(Role::User, Content::text(self.0)));
        Ok(())
    }
}

#[tokio::test]
async fn intervenor_sends_op_processed_at_next_boundary() {
    let (tx, rx) = mpsc::channel(8);
    let intervenor = ContextIntervenor::new(tx);

    let mut ctx = Context::new();
    ctx.with_intervention(rx);

    intervenor
        .send(InjectSupervisorNote("supervisor note"))
        .await
        .expect("send succeeds while worker is attached");

    ctx.run(PushUserMessage("worker turn"))
        .await
        .expect("run succeeds");

    let messages = ctx.messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text_content(), "supervisor note");
    assert_eq!(messages[1].text_content(), "worker turn");
}

#[tokio::test]
async fn intervenor_reports_closed_channel() {
    let (tx, rx) = mpsc::channel(1);
    let intervenor = ContextIntervenor::new(tx);
    drop(rx);

    let err = intervenor
        .send(InjectSupervisorNote("will fail"))
        .await
        .expect_err("closed worker channel must be reported");
    assert!(matches!(err, InterventionSendError::Closed));
}
