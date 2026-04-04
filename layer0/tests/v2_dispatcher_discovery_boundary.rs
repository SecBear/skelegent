use async_trait::async_trait;
use layer0::operator::TriggerType;
use layer0::{
    CapabilityDescriptor, CapabilityFilter, CapabilityId, CapabilitySource, Content,
    DispatchContext, DispatchEvent, DispatchId, Dispatcher, InvocationHandle, Operator, OperatorId,
    OperatorInput, OperatorOutput, Outcome, ProtocolError, TerminalOutcome,
};

struct NoopOperator;

#[async_trait]
impl Operator for NoopOperator {
    async fn execute(
        &self,
        _input: OperatorInput,
        _ctx: &DispatchContext,
    ) -> Result<OperatorOutput, ProtocolError> {
        Ok(OperatorOutput::new(
            Content::text("ok"),
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            },
        ))
    }
}

struct InvokeOnly;

#[async_trait]
impl Dispatcher for InvokeOnly {
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<InvocationHandle, ProtocolError> {
        let operator = NoopOperator;
        let ctx = ctx.clone();
        let (handle, sender) = InvocationHandle::channel(ctx.dispatch_id.clone());
        tokio::spawn(async move {
            let result = operator.execute(input, &ctx).await;
            match result {
                Ok(output) => {
                    let _ = sender.send(DispatchEvent::Completed { output }).await;
                }
                Err(error) => {
                    let _ = sender.send(DispatchEvent::Failed { error }).await;
                }
            }
        });
        Ok(handle)
    }
}

struct DiscoverOnly;

#[async_trait]
impl CapabilitySource for DiscoverOnly {
    async fn list(
        &self,
        _filter: CapabilityFilter,
    ) -> Result<Vec<CapabilityDescriptor>, ProtocolError> {
        Ok(Vec::new())
    }

    async fn get(&self, _id: &CapabilityId) -> Result<Option<CapabilityDescriptor>, ProtocolError> {
        Ok(None)
    }
}

#[test]
fn dispatcher_and_capability_source_are_distinct_traits() {
    fn accepts_dispatcher(_: &dyn Dispatcher) {}
    fn accepts_source(_: &dyn CapabilitySource) {}

    let dispatcher = InvokeOnly;
    let source = DiscoverOnly;

    accepts_dispatcher(&dispatcher);
    accepts_source(&source);
}

#[tokio::test]
async fn dispatcher_remains_invocation_only() {
    let dispatcher = InvokeOnly;
    let ctx = DispatchContext::new(DispatchId::new("dispatch-1"), OperatorId::new("noop"));
    let input = OperatorInput::new(Content::text("{}"), TriggerType::Task);
    let output = dispatcher
        .dispatch(&ctx, input)
        .await
        .expect("dispatch handle")
        .collect()
        .await
        .expect("completed");
    assert_eq!(
        output.outcome,
        Outcome::Terminal {
            terminal: TerminalOutcome::Completed
        }
    );
}
