#![deny(missing_docs)]
//! Context strategy implementations for neuron-turn.
//!
//! Provides [`SlidingWindow`] for dropping oldest messages when context
//! exceeds a limit, [`SaliencePackingStrategy`] for salience-aware
//! packing via iterative MMR selection, and [`ContextAssembler`] for
//! assembling sweep context packages from state store data.
//! `NoCompaction` is in neuron-turn itself.

pub mod context_assembly;
mod salience_packing;

pub use context_assembly::{ContextAssembler, ContextAssemblyConfig};
pub use salience_packing::{SaliencePackingConfig, SaliencePackingStrategy};

use layer0::CompactionPolicy;
use neuron_turn::context::{AnnotatedMessage, CompactionError, ContextStrategy};
use neuron_turn::types::{ContentPart, ProviderMessage};

/// Sliding window context strategy.
///
/// When context exceeds the limit, drops the oldest messages
/// (keeping the first message, which is typically the initial user message).
/// Pinned messages (policy = `Pinned`) are always preserved.
pub struct SlidingWindow {
    /// Approximate chars-per-token ratio for estimation.
    chars_per_token: usize,
}

impl SlidingWindow {
    /// Create a new sliding window strategy.
    ///
    /// `chars_per_token` controls the token estimation granularity
    /// (default: 4 chars per token).
    pub fn new() -> Self {
        Self { chars_per_token: 4 }
    }

    /// Create with a custom chars-per-token ratio.
    pub fn with_ratio(chars_per_token: usize) -> Self {
        Self {
            chars_per_token: chars_per_token.max(1),
        }
    }

    fn estimate_message_tokens(&self, msg: &ProviderMessage) -> usize {
        msg.content
            .iter()
            .map(|part| match part {
                ContentPart::Text { text } => text.len() / self.chars_per_token,
                ContentPart::ToolUse { input, .. } => {
                    input.to_string().len() / self.chars_per_token
                }
                ContentPart::ToolResult { content, .. } => content.len() / self.chars_per_token,
                ContentPart::Image { .. } => 1000,
            })
            .sum::<usize>()
            + 4 // overhead per message (role, formatting)
    }
}

impl Default for SlidingWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextStrategy for SlidingWindow {
    fn token_estimate(&self, messages: &[AnnotatedMessage]) -> usize {
        messages
            .iter()
            .map(|m| self.estimate_message_tokens(&m.message))
            .sum()
    }

    fn should_compact(&self, messages: &[AnnotatedMessage], limit: usize) -> bool {
        self.token_estimate(messages) > limit
    }

    fn compact(
        &self,
        messages: Vec<AnnotatedMessage>,
    ) -> Result<Vec<AnnotatedMessage>, CompactionError> {
        // Partition: pinned messages survive all compaction.
        let (pinned, normal): (Vec<AnnotatedMessage>, Vec<AnnotatedMessage>) = messages
            .into_iter()
            .partition(|m| matches!(m.policy, Some(CompactionPolicy::Pinned)));

        // Apply sliding-window to non-pinned messages (existing algorithm).
        let compacted_normal = if normal.len() <= 2 {
            normal
        } else {
            let first = normal[0].clone();
            let rest = &normal[1..];

            let total_tokens: usize = {
                let first_tokens = self.estimate_message_tokens(&first.message);
                let rest_tokens: usize = rest
                    .iter()
                    .map(|m| self.estimate_message_tokens(&m.message))
                    .sum();
                first_tokens + rest_tokens
            };
            let target = total_tokens / 2;

            let mut kept = Vec::new();
            let mut current_tokens = self.estimate_message_tokens(&first.message);

            for msg in rest.iter().rev() {
                let msg_tokens = self.estimate_message_tokens(&msg.message);
                if current_tokens + msg_tokens > target && !kept.is_empty() {
                    break;
                }
                kept.push(msg.clone());
                current_tokens += msg_tokens;
            }

            kept.reverse();
            let mut result = vec![first];
            result.extend(kept);
            result
        };

        // Pinned messages go first (invariants), then compacted normal messages.
        let mut result = pinned;
        result.extend(compacted_normal);
        Ok(result)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// New middleware-era compactor (Phase R)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use layer0::context::Message;

/// Sliding window compactor: drops oldest messages, keeps first + recent by token budget.
///
/// The returned closure is passed to `Context::compact_with()`.
/// Pinned messages always survive. The first non-pinned message is preserved
/// (typically the initial user message). Remaining budget is filled from the end.
pub fn sliding_window_compactor() -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        let (pinned, normal): (Vec<_>, Vec<_>) = msgs
            .iter()
            .partition(|m| matches!(m.meta.policy, CompactionPolicy::Pinned));

        let compacted_normal = if normal.len() <= 2 {
            normal.into_iter().cloned().collect()
        } else {
            let first = normal[0].clone();
            let rest = &normal[1..];

            let total_tokens: usize = std::iter::once(&first)
                .chain(rest.iter().copied())
                .map(|m| m.estimated_tokens())
                .sum();
            let target = total_tokens / 2;

            let mut kept = Vec::new();
            let mut current_tokens = first.estimated_tokens();

            for msg in rest.iter().rev() {
                let msg_tokens = msg.estimated_tokens();
                if current_tokens + msg_tokens > target && !kept.is_empty() {
                    break;
                }
                kept.push((*msg).clone());
                current_tokens += msg_tokens;
            }

            kept.reverse();
            let mut result = vec![first];
            result.extend(kept);
            result
        };

        let mut result: Vec<Message> = pinned.into_iter().cloned().collect();
        result.extend(compacted_normal);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_turn::types::Role;

    fn text_message(role: Role, text: &str) -> AnnotatedMessage {
        AnnotatedMessage::from(ProviderMessage {
            role,
            content: vec![ContentPart::Text {
                text: text.to_string(),
            }],
        })
    }

    #[test]
    fn sliding_window_estimates_tokens() {
        let sw = SlidingWindow::new();
        let messages = vec![text_message(Role::User, &"a".repeat(400))];
        // 400 chars / 4 = 100, + 4 overhead = 104
        assert_eq!(sw.token_estimate(&messages), 104);
    }

    #[test]
    fn sliding_window_should_compact() {
        let sw = SlidingWindow::new();
        let messages = vec![text_message(Role::User, &"a".repeat(400))];
        assert!(sw.should_compact(&messages, 50));
        assert!(!sw.should_compact(&messages, 200));
    }

    #[test]
    fn sliding_window_compact_preserves_first_and_recent() {
        let sw = SlidingWindow::new();
        let messages = vec![
            text_message(Role::User, &"first ".repeat(100)),
            text_message(Role::Assistant, &"old ".repeat(100)),
            text_message(Role::User, &"middle ".repeat(100)),
            text_message(Role::Assistant, &"recent ".repeat(100)),
            text_message(Role::User, &"latest ".repeat(100)),
        ];

        let compacted = sw.compact(messages.clone()).unwrap();

        // Should keep first message
        assert_eq!(compacted[0].message.role, Role::User);
        assert!(compacted[0].message.content[0] == messages[0].message.content[0]);

        // Should keep some recent messages
        assert!(compacted.len() < messages.len());
        assert!(compacted.len() >= 2);

        // Last message should be the latest
        assert_eq!(
            compacted.last().unwrap().message.content[0],
            messages.last().unwrap().message.content[0]
        );
    }

    #[test]
    fn sliding_window_short_messages_unchanged() {
        let sw = SlidingWindow::new();
        let messages = vec![
            text_message(Role::User, "hi"),
            text_message(Role::Assistant, "hello"),
        ];

        let compacted = sw.compact(messages.clone()).unwrap();
        assert_eq!(compacted.len(), messages.len());
    }

    #[test]
    fn sliding_window_single_message_unchanged() {
        let sw = SlidingWindow::new();
        let messages = vec![text_message(Role::User, "hi")];
        let compacted = sw.compact(messages.clone()).unwrap();
        assert_eq!(compacted.len(), 1);
    }

    #[test]
    fn sliding_window_pinned_messages_survive_compaction() {
        let sw = SlidingWindow::new();
        // Build a list where a pinned message would otherwise be dropped
        let pinned = AnnotatedMessage::pinned(ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "pinned constraint".to_string(),
            }],
        });
        let mut messages = vec![pinned.clone()];
        // Add enough normal messages to trigger compaction
        for i in 0..10 {
            messages.push(text_message(Role::User, &"x".repeat(400 + i * 10)));
        }

        let compacted = sw.compact(messages).unwrap();

        // The pinned message must survive
        assert!(
            compacted
                .iter()
                .any(|m| m.message.content == pinned.message.content),
            "pinned message must survive compaction"
        );
    }

    // --- New compactor tests (Phase R) ---

    use layer0::content::Content;
    use layer0::context::{Message, Role as L0Role};

    fn l0_text_msg(role: L0Role, text: &str) -> Message {
        Message::new(role, Content::text(text))
    }

    #[test]
    fn sliding_compactor_preserves_first_and_recent() {
        let mut compactor = sliding_window_compactor();
        let messages = vec![
            l0_text_msg(L0Role::User, &"first ".repeat(100)),
            l0_text_msg(L0Role::Assistant, &"old ".repeat(100)),
            l0_text_msg(L0Role::User, &"middle ".repeat(100)),
            l0_text_msg(L0Role::Assistant, &"recent ".repeat(100)),
            l0_text_msg(L0Role::User, &"latest ".repeat(100)),
        ];

        let compacted = compactor(&messages);

        // Should keep first message
        assert!(matches!(compacted[0].role, L0Role::User));
        // Should compact
        assert!(compacted.len() < messages.len());
        assert!(compacted.len() >= 2);
        // Last should be latest
        assert_eq!(
            compacted.last().unwrap().text_content(),
            messages.last().unwrap().text_content()
        );
    }

    #[test]
    fn sliding_compactor_short_messages_unchanged() {
        let mut compactor = sliding_window_compactor();
        let messages = vec![
            l0_text_msg(L0Role::User, "hi"),
            l0_text_msg(L0Role::Assistant, "hello"),
        ];
        let compacted = compactor(&messages);
        assert_eq!(compacted.len(), 2);
    }

    #[test]
    fn sliding_compactor_single_message_unchanged() {
        let mut compactor = sliding_window_compactor();
        let messages = vec![l0_text_msg(L0Role::User, "hi")];
        let compacted = compactor(&messages);
        assert_eq!(compacted.len(), 1);
    }

    #[test]
    fn sliding_compactor_pinned_survive() {
        let mut compactor = sliding_window_compactor();
        let pinned = Message::pinned(L0Role::User, Content::text("pinned constraint"));
        let mut messages = vec![pinned.clone()];
        for i in 0..10 {
            messages.push(l0_text_msg(L0Role::User, &"x".repeat(400 + i * 10)));
        }

        let compacted = compactor(&messages);

        assert!(
            compacted
                .iter()
                .any(|m| m.text_content() == "pinned constraint"),
            "pinned message must survive compaction"
        );
    }
}
