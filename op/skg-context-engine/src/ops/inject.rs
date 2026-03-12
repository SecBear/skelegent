//! Context injection operations.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::content::Content;
use layer0::context::{Message, Role};

/// Inject a system message at the start of the context.
///
/// If a system message already exists at position 0, it is replaced.
/// Otherwise a new system message is inserted at position 0.
pub struct InjectSystem {
    /// The system prompt text.
    pub prompt: String,
}

#[async_trait]
impl ContextOp for InjectSystem {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        let system_msg = Message::new(Role::System, Content::text(&self.prompt));

        if ctx
            .messages()
            .first()
            .is_some_and(|m| m.role == Role::System)
        {
            ctx.replace_message(0, system_msg);
        } else {
            ctx.insert_message(0, system_msg);
        }
        Ok(())
    }
}

/// Inject a single message at the end of the context.
pub struct InjectMessage {
    /// The message to append.
    pub message: Message,
}

#[async_trait]
impl ContextOp for InjectMessage {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        ctx.push_message(self.message.clone());
        Ok(())
    }
}

/// Inject multiple messages at the end of the context.
pub struct InjectMessages {
    /// The messages to append.
    pub messages: Vec<Message>,
}

#[async_trait]
impl ContextOp for InjectMessages {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        ctx.extend_messages(self.messages.iter().cloned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inject_system_inserts_at_start() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        ctx.run(InjectSystem {
            prompt: "You are helpful.".into(),
        })
        .await
        .unwrap();

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].role, Role::System);
        assert_eq!(ctx.messages()[0].text_content(), "You are helpful.");
    }

    #[tokio::test]
    async fn inject_system_replaces_existing() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("old")));
        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        ctx.run(InjectSystem {
            prompt: "new".into(),
        })
        .await
        .unwrap();

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].text_content(), "new");
    }

    #[tokio::test]
    async fn inject_message_appends() {
        let mut ctx = Context::new();
        ctx.run(InjectMessage {
            message: Message::new(Role::User, Content::text("hello")),
        })
        .await
        .unwrap();
        ctx.run(InjectMessage {
            message: Message::new(Role::Assistant, Content::text("hi")),
        })
        .await
        .unwrap();

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].role, Role::User);
        assert_eq!(ctx.messages()[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn inject_messages_appends_all() {
        let mut ctx = Context::new();
        ctx.run(InjectMessages {
            messages: vec![
                Message::new(Role::User, Content::text("a")),
                Message::new(Role::Assistant, Content::text("b")),
                Message::new(Role::User, Content::text("c")),
            ],
        })
        .await
        .unwrap();

        assert_eq!(ctx.messages().len(), 3);
    }
}
