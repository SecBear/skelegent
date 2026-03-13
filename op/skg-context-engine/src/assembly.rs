//! Fluent assembly API for context.
//!
//! Each method internally dispatches through [`Context::run()`], so rules fire
//! automatically. The user writes clean fluent code; the framework dispatches
//! through rules.

use crate::context::Context;
use crate::error::EngineError;
use crate::ops::compact::Compact;
use crate::ops::inject::{InjectMessage, InjectMessages, InjectSystem};
use layer0::context::Message;

/// Fluent context assembly methods.
///
/// Every method dispatches through [`Context::run()`], making it automatically
/// hookable by rules. A budget guard, overwatch agent, or telemetry recorder
/// will see these operations without any explicit wiring.
impl Context {
    /// Inject a system prompt. Replaces existing system message if present.
    pub async fn inject_system(&mut self, prompt: &str) -> Result<(), EngineError> {
        self.run(InjectSystem {
            prompt: prompt.to_string(),
        })
        .await
    }

    /// Append a single message to the context.
    pub async fn inject_message(&mut self, msg: Message) -> Result<(), EngineError> {
        self.run(InjectMessage { message: msg }).await
    }

    /// Append multiple messages to the context.
    pub async fn inject_messages(&mut self, msgs: Vec<Message>) -> Result<(), EngineError> {
        self.run(InjectMessages { messages: msgs }).await
    }

    /// Run compaction on the context's messages.
    pub async fn compact(
        &mut self,
        strategy: impl FnMut(&[Message]) -> Vec<Message> + Send + 'static,
    ) -> Result<(), EngineError> {
        self.run(Compact::new(strategy)).await?;
        Ok(())
    }

    /// Run compaction only if the predicate is true.
    pub async fn compact_if(
        &mut self,
        predicate: impl FnOnce(&Context) -> bool,
        strategy: impl FnMut(&[Message]) -> Vec<Message> + Send + 'static,
    ) -> Result<(), EngineError> {
        if predicate(self) {
            self.compact(strategy).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::Role;

    #[tokio::test]
    async fn fluent_assembly_chain() {
        let mut ctx = Context::new();
        ctx.inject_system("You are helpful.").await.unwrap();
        ctx.inject_message(Message::new(Role::User, Content::text("hello")))
            .await
            .unwrap();

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].role, Role::System);
        assert_eq!(ctx.messages()[1].role, Role::User);
    }

    #[tokio::test]
    async fn compact_if_skips_when_false() {
        let mut ctx = Context::new();
        ctx.inject_message(Message::new(Role::User, Content::text("hello")))
            .await
            .unwrap();

        ctx.compact_if(
            |ctx| ctx.messages().len() > 10,
            |msgs| msgs.iter().rev().take(1).cloned().collect(),
        )
        .await
        .unwrap();

        // Only 1 message, predicate false, no compaction
        assert_eq!(ctx.messages().len(), 1);
    }
}
