//! Context strategy for managing the conversation window.
//!
//! The [`ContextStrategy`] trait handles client-side context compaction.
//! Provider-native truncation (e.g., OpenAI `truncation: auto`) is
//! invisible to the strategy — handled by the Provider impl internally.

use crate::types::ProviderMessage;

/// Strategy for managing context window size.
///
/// Implementations: `NoCompaction` (passthrough), `SlidingWindow`
/// (drop oldest messages), `Summarization` (future).
pub trait ContextStrategy: Send + Sync {
    /// Estimate token count for a message list.
    fn token_estimate(&self, messages: &[ProviderMessage]) -> usize;

    /// Whether compaction should run given the current messages and limit.
    fn should_compact(&self, messages: &[ProviderMessage], limit: usize) -> bool;

    /// Compact the message list. Returns a shorter list.
    fn compact(&self, messages: Vec<ProviderMessage>) -> Vec<ProviderMessage>;
}

/// A no-op context strategy that never compacts.
///
/// Useful for short conversations or when the provider handles
/// truncation natively.
pub struct NoCompaction;

impl ContextStrategy for NoCompaction {
    fn token_estimate(&self, messages: &[ProviderMessage]) -> usize {
        // Rough estimate: 4 chars per token
        messages
            .iter()
            .flat_map(|m| &m.content)
            .map(|part| {
                use crate::types::ContentPart;
                match part {
                    ContentPart::Text { text } => text.len() / 4,
                    ContentPart::ToolUse { input, .. } => input.to_string().len() / 4,
                    ContentPart::ToolResult { content, .. } => content.len() / 4,
                    ContentPart::Image { .. } => 1000, // rough image token estimate
                }
            })
            .sum()
    }

    fn should_compact(&self, _messages: &[ProviderMessage], _limit: usize) -> bool {
        false
    }

    fn compact(&self, messages: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentPart, Role};

    #[test]
    fn no_compaction_never_compacts() {
        let strategy = NoCompaction;
        let messages = vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "hello".into(),
            }],
        }];

        assert!(!strategy.should_compact(&messages, 100));
        let compacted = strategy.compact(messages.clone());
        assert_eq!(compacted.len(), messages.len());
    }

    #[test]
    fn no_compaction_estimates_tokens() {
        let strategy = NoCompaction;
        let messages = vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "a".repeat(400),
            }],
        }];

        let estimate = strategy.token_estimate(&messages);
        assert_eq!(estimate, 100); // 400 chars / 4
    }

    #[test]
    fn no_compaction_preserves_all_messages() {
        let strategy = NoCompaction;
        let messages = vec![
            ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "msg1".into(),
                }],
            },
            ProviderMessage {
                role: Role::Assistant,
                content: vec![ContentPart::Text {
                    text: "msg2".into(),
                }],
            },
            ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "msg3".into(),
                }],
            },
        ];

        let compacted = strategy.compact(messages.clone());
        assert_eq!(compacted.len(), 3);
        assert_eq!(compacted[0].content, messages[0].content);
        assert_eq!(compacted[1].content, messages[1].content);
        assert_eq!(compacted[2].content, messages[2].content);
    }

    #[test]
    fn no_compaction_estimates_tool_use_tokens() {
        let strategy = NoCompaction;
        let messages = vec![ProviderMessage {
            role: Role::Assistant,
            content: vec![ContentPart::ToolUse {
                id: "tu_1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            }],
        }];

        let estimate = strategy.token_estimate(&messages);
        // The JSON representation of the input will be tokenized
        assert!(estimate > 0);
    }

    #[test]
    fn no_compaction_estimates_tool_result_tokens() {
        let strategy = NoCompaction;
        let messages = vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "a".repeat(200),
                is_error: false,
            }],
        }];

        let estimate = strategy.token_estimate(&messages);
        assert_eq!(estimate, 50); // 200 chars / 4
    }

    #[test]
    fn no_compaction_estimates_image_tokens() {
        let strategy = NoCompaction;
        let messages = vec![ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Image {
                source: crate::types::ImageSource::Url {
                    url: "https://example.com/img.png".into(),
                },
                media_type: "image/png".into(),
            }],
        }];

        let estimate = strategy.token_estimate(&messages);
        assert_eq!(estimate, 1000); // rough image estimate
    }

    #[test]
    fn context_strategy_is_object_safe() {
        fn _assert_object_safe(_: &dyn ContextStrategy) {}
        let nc = NoCompaction;
        _assert_object_safe(&nc);
    }
}
