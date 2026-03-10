//! Context compaction operations.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use layer0::context::Message;
use std::sync::Mutex;

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactResult {
    /// Number of messages before compaction.
    pub before: usize,
    /// Number of messages after compaction.
    pub after: usize,
}

type CompactStrategy = Mutex<Box<dyn FnMut(&[Message]) -> Vec<Message> + Send>>;

/// Run a compaction closure on the context's messages.
///
/// The closure receives the current messages and returns the compacted
/// set. Any compaction strategy (sliding window, tiered, salience-based)
/// can be expressed as this closure.
pub struct Compact {
    /// The compaction strategy, wrapped in Mutex for interior mutability.
    strategy: CompactStrategy,
}

impl Compact {
    /// Create a compaction op from a closure.
    pub fn new(strategy: impl FnMut(&[Message]) -> Vec<Message> + Send + 'static) -> Self {
        Self {
            strategy: Mutex::new(Box::new(strategy)),
        }
    }
}

#[async_trait]
impl ContextOp for Compact {
    type Output = CompactResult;

    async fn execute(&self, ctx: &mut Context) -> Result<CompactResult, EngineError> {
        let before = ctx.messages.len();

        let compacted = {
            let mut strategy = self.strategy.lock().map_err(|e| {
                EngineError::Custom(format!("compaction mutex poisoned: {e}").into())
            })?;
            strategy(&ctx.messages)
        };
        ctx.messages = compacted;

        Ok(CompactResult {
            before,
            after: ctx.messages.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::Role;

    #[tokio::test]
    async fn compact_reduces_messages() {
        let mut ctx = Context::new();
        for i in 0..10 {
            ctx.messages
                .push(Message::new(Role::User, Content::text(format!("msg {i}"))));
        }

        let result = ctx
            .run(Compact::new(|msgs| {
                // Keep only last 3
                msgs.iter().rev().take(3).rev().cloned().collect()
            }))
            .await
            .unwrap();

        assert_eq!(result.before, 10);
        assert_eq!(result.after, 3);
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(ctx.messages[0].text_content(), "msg 7");
    }
}
