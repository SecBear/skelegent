//! EchoOperator — returns the input message as the output.

use crate::dispatch::EffectEmitter;
use crate::dispatch_context::DispatchContext;
use crate::error::OperatorError;
use crate::operator::{ExitReason, OperatorInput, OperatorOutput};
use async_trait::async_trait;

/// An operator implementation that echoes the input message back as output.
/// Used for testing orchestration, environment, and hook integrations.
pub struct EchoOperator;

#[async_trait]
impl crate::operator::Operator for EchoOperator {
    async fn execute(
        &self,
        input: OperatorInput,
        _ctx: &DispatchContext,
        _emitter: &EffectEmitter,
    ) -> Result<OperatorOutput, OperatorError> {
        Ok(OperatorOutput::new(input.message, ExitReason::Complete))
    }
}