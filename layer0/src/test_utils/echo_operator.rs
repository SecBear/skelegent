//! EchoOperator — returns the input message as the output.

use crate::dispatch_context::DispatchContext;
use crate::error::ProtocolError;
use crate::operator::{OperatorInput, OperatorOutput, Outcome, TerminalOutcome};
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
    ) -> Result<OperatorOutput, ProtocolError> {
        Ok(OperatorOutput::new(
            input.message,
            Outcome::Terminal {
                terminal: TerminalOutcome::Completed,
            },
        ))
    }
}
