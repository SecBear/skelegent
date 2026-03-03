#![deny(missing_docs)]
//! Context strategy implementations for neuron-turn.
//!
//! Provides [`SlidingWindow`] for dropping oldest messages when context
//! exceeds a limit. `NoCompaction` is in neuron-turn itself.

use neuron_turn::context::ContextStrategy;
use neuron_turn::types::{ContentPart, ProviderMessage};

/// Sliding window context strategy.
///
/// When context exceeds the limit, drops the oldest messages
/// (keeping the first message, which is typically the initial user message).
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
    fn token_estimate(&self, messages: &[ProviderMessage]) -> usize {
        messages
            .iter()
            .map(|m| self.estimate_message_tokens(m))
            .sum()
    }

    fn should_compact(&self, messages: &[ProviderMessage], limit: usize) -> bool {
        self.token_estimate(messages) > limit
    }

    fn compact(&self, messages: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
        if messages.len() <= 2 {
            return messages;
        }

        // Keep first message + most recent messages that fit
        let first = messages[0].clone();
        let rest = &messages[1..];

        // Work backwards, accumulating messages until we hit roughly half the
        // original size (heuristic: keep recent context, drop old)
        let total_tokens: usize = messages
            .iter()
            .map(|m| self.estimate_message_tokens(m))
            .sum();
        let target = total_tokens / 2;

        let mut kept = Vec::new();
        let mut current_tokens = self.estimate_message_tokens(&first);

        for msg in rest.iter().rev() {
            let msg_tokens = self.estimate_message_tokens(msg);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_turn::types::Role;

    fn text_message(role: Role, text: &str) -> ProviderMessage {
        ProviderMessage {
            role,
            content: vec![ContentPart::Text {
                text: text.to_string(),
            }],
        }
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

        let compacted = sw.compact(messages.clone());

        // Should keep first message
        assert_eq!(compacted[0].role, Role::User);
        assert!(compacted[0].content[0] == messages[0].content[0]);

        // Should keep some recent messages
        assert!(compacted.len() < messages.len());
        assert!(compacted.len() >= 2);

        // Last message should be the latest
        assert_eq!(
            compacted.last().unwrap().content[0],
            messages.last().unwrap().content[0]
        );
    }

    #[test]
    fn sliding_window_short_messages_unchanged() {
        let sw = SlidingWindow::new();
        let messages = vec![
            text_message(Role::User, "hi"),
            text_message(Role::Assistant, "hello"),
        ];

        let compacted = sw.compact(messages.clone());
        assert_eq!(compacted.len(), messages.len());
    }

    #[test]
    fn sliding_window_single_message_unchanged() {
        let sw = SlidingWindow::new();
        let messages = vec![text_message(Role::User, "hi")];
        let compacted = sw.compact(messages.clone());
        assert_eq!(compacted.len(), 1);
    }
}
