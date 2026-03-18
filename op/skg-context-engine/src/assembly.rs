//! Fluent assembly API for context.
//!
//! Each method internally dispatches through [`Context::run()`], so rules fire
//! automatically. The user writes clean fluent code; the framework dispatches
//! through rules.

use crate::context::Context;
use crate::error::EngineError;
use crate::ops::compact::Compact;
use crate::ops::inject::{InjectMessage, InjectMessages, InjectSystem};
use crate::ops::store::{LoadConversation, SaveConversation};
use layer0::context::Message;
use layer0::effect::Scope;
use layer0::state::StateStore;
use std::sync::Arc;

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

    // ── Persistence ──────────────────────────────────────────

    /// Save the current conversation messages to a [`StateStore`].
    ///
    /// Serializes the message buffer as JSON under the given scope and key.
    /// Pair with [`load_conversation`](Self::load_conversation) to restore.
    pub async fn save_conversation(
        &mut self,
        store: Arc<dyn StateStore>,
        scope: Scope,
        key: impl Into<String>,
    ) -> Result<(), EngineError> {
        self.run(SaveConversation::new(store, scope, key)).await
    }

    /// Load conversation messages from a [`StateStore`].
    ///
    /// Reads a JSON array of messages from the store and replaces the
    /// context messages. Returns `None` if the key does not exist
    /// (context unchanged), or `Some(count)` if messages were loaded.
    pub async fn load_conversation(
        &mut self,
        store: Arc<dyn StateStore>,
        scope: Scope,
        key: impl Into<String>,
    ) -> Result<Option<usize>, EngineError> {
        self.run(LoadConversation::new(store, scope, key)).await
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
