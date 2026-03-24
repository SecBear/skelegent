//! Token counting for context budget management.
//!
//! Providers implement [`TokenCounter`] to give accurate token counts for
//! their specific tokenizer. [`HeuristicTokenCounter`] is provided as a
//! reasonable default when no provider-specific tokenizer is available.

use layer0::context::Message;

/// Token counting for context budget management.
///
/// Providers implement this to give accurate token counts for their
/// specific tokenizer. A default heuristic implementation is provided
/// for providers that don't have native token counting.
pub trait TokenCounter: Send + Sync {
    /// Count tokens in a sequence of messages.
    fn count_messages(&self, messages: &[Message]) -> usize;

    /// Count tokens in a text string.
    fn count_text(&self, text: &str) -> usize;

    /// The model's maximum context window size in tokens.
    fn context_limit(&self) -> usize;
}

/// Heuristic token counter using a characters-per-token ratio.
///
/// A reasonable default when no provider-specific tokenizer is available.
/// Defaults to 4 characters per token (English text average). Delegates
/// per-message counting to [`Message::estimated_tokens`], which handles
/// all content block types (text, tool use/result, images, etc.) with
/// the same ratio and a 4-token per-message overhead.
pub struct HeuristicTokenCounter {
    chars_per_token: f32,
    context_limit: usize,
}

impl HeuristicTokenCounter {
    /// Create with default 4 chars/token ratio.
    pub fn new(context_limit: usize) -> Self {
        Self {
            chars_per_token: 4.0,
            context_limit,
        }
    }

    /// Create with a custom chars/token ratio.
    ///
    /// Values below 1.0 are technically valid but will produce inflated counts;
    /// typical English text is 3.5–5 chars/token for most tokenizers.
    pub fn with_ratio(chars_per_token: f32, context_limit: usize) -> Self {
        Self {
            chars_per_token,
            context_limit,
        }
    }
}

impl TokenCounter for HeuristicTokenCounter {
    /// Count tokens across a message slice.
    ///
    /// Delegates to [`Message::estimated_tokens`] which applies the same
    /// chars/4 ratio across all content block types plus a 4-token overhead
    /// per message. When `chars_per_token` differs from 4.0, the count is
    /// adjusted by the ratio between the requested and default ratios.
    fn count_messages(&self, messages: &[Message]) -> usize {
        // Message::estimated_tokens uses 4 chars/token + 4 overhead.
        // Scale by the ratio relative to the default (4.0) so custom ratios work.
        let scale = 4.0 / self.chars_per_token;
        messages
            .iter()
            .map(|m| {
                let base = m.estimated_tokens() as f32;
                base.mul_add(scale, 0.0).ceil() as usize
            })
            .sum()
    }

    /// Count tokens in a plain text string.
    fn count_text(&self, text: &str) -> usize {
        (text.len() as f32 / self.chars_per_token).ceil() as usize
    }

    /// The model's maximum context window size in tokens.
    fn context_limit(&self) -> usize {
        self.context_limit
    }
}

/// Well-known context window sizes for common models.
pub mod limits {
    /// Claude Sonnet/Opus: 200K tokens.
    pub const ANTHROPIC_200K: usize = 200_000;
    /// Claude Haiku: 200K tokens.
    pub const ANTHROPIC_HAIKU: usize = 200_000;
    /// GPT-4o: 128K tokens.
    pub const OPENAI_GPT4O: usize = 128_000;
    /// GPT-4o mini: 128K tokens.
    pub const OPENAI_GPT4O_MINI: usize = 128_000;
    /// Ollama default (varies by model; 8K is a safe conservative baseline).
    pub const OLLAMA_DEFAULT: usize = 8_192;
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::{Message, Role};

    fn text_message(text: &str) -> Message {
        Message::new(Role::User, Content::Text(text.to_string()))
    }

    #[test]
    fn heuristic_count_text() {
        let counter = HeuristicTokenCounter::new(limits::ANTHROPIC_200K);
        // 8 chars / 4 = 2 tokens
        assert_eq!(counter.count_text("hellowor"), 2);
        // 9 chars / 4 = 2.25 → ceil → 3 tokens
        assert_eq!(counter.count_text("helloworl"), 3);
        // empty string → 0
        assert_eq!(counter.count_text(""), 0);
    }

    #[test]
    fn heuristic_count_messages() {
        let counter = HeuristicTokenCounter::new(limits::ANTHROPIC_200K);
        // "hello" = 5 chars → 5/4 = 1 (integer) + 4 overhead = 5 tokens per Message::estimated_tokens
        let msgs = vec![text_message("hello"), text_message("hello")];
        let count = counter.count_messages(&msgs);
        // Each message: 5/4=1 + 4 = 5; scale=1.0; ceil(5.0)=5; total=10
        assert_eq!(count, 10);
    }

    #[test]
    fn heuristic_context_limit() {
        let counter = HeuristicTokenCounter::new(limits::OPENAI_GPT4O);
        assert_eq!(counter.context_limit(), 128_000);
    }

    #[test]
    fn heuristic_empty_messages() {
        let counter = HeuristicTokenCounter::new(limits::ANTHROPIC_200K);
        assert_eq!(counter.count_messages(&[]), 0);
    }

    #[test]
    fn custom_ratio() {
        // With 8 chars/token, "hellohello" (10 chars) → ceil(10/8) = 2
        let counter = HeuristicTokenCounter::with_ratio(8.0, limits::ANTHROPIC_200K);
        assert_eq!(counter.count_text("hellohello"), 2);

        // count_messages with doubled ratio should halve the token count
        // relative to the default (scale = 4/8 = 0.5)
        let default_counter = HeuristicTokenCounter::new(limits::ANTHROPIC_200K);
        let msgs = vec![text_message("hello world test message content")];
        let default_count = default_counter.count_messages(&msgs);
        let custom_count = counter.count_messages(&msgs);
        assert!(
            custom_count <= default_count,
            "larger ratio should produce fewer or equal tokens"
        );
    }
}
