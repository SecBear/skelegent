//! Pre-built compaction strategies and a reactive [`CompactionRule`].
//!
//! Provides two ready-to-use compaction strategies as pure functions and as a
//! reactive rule that fires automatically when message count exceeds a threshold.
//!
//! Use the pure functions directly with the [`crate::ops::compact::Compact`] op,
//! or register a [`CompactionRule`] to compact on a trigger.
//!
//! # Example
//!
//! ```ignore
//! use neuron_context_engine::rules::CompactionRule;
//!
//! let mut ctx = Context::new();
//! ctx.add_rule(CompactionRule::sliding_window(50).into_rule());
//! // Context compacts automatically whenever messages exceed 50.
//! ```

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use crate::rule::{Rule, Trigger};
use async_trait::async_trait;
use layer0::context::Message;
use layer0::lifecycle::CompactionPolicy;
use layer0::context::Role;
use neuron_turn::infer::InferRequest;
use neuron_turn::provider::Provider;

/// A summary produced by a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionReport {
    /// Number of messages before compaction.
    pub messages_before: usize,
    /// Number of messages after compaction.
    pub messages_after: usize,
}

/// Pre-built compaction strategies.
///
/// Each variant configures a different removal heuristic. Use [`sliding_window`]
/// and [`policy_trim`] as standalone functions, or wrap a strategy in a
/// [`CompactionRule`] for automatic reactive firing.
#[derive(Debug, Clone)]
pub enum CompactionStrategy {
    /// Keep the most recent `keep` non-pinned messages, always preserving
    /// [`CompactionPolicy::Pinned`] messages.
    SlidingWindow {
        /// Number of non-pinned messages to retain.
        keep: usize,
    },
    /// Remove messages by policy priority until count reaches `target`.
    ///
    /// Removal order: [`CompactionPolicy::DiscardWhenDone`] first (oldest first),
    /// then [`CompactionPolicy::CompressFirst`] (oldest first), then
    /// [`CompactionPolicy::Normal`] (oldest first). Pinned messages are never
    /// removed.
    PolicyTrim {
        /// Target message count to trim down to.
        target: usize,
    },
}

/// Keep the most recent `keep` non-pinned messages, preserving all pinned messages.
///
/// [`CompactionPolicy::Pinned`] messages are always retained regardless of `keep`.
/// From the non-pinned messages, only the `keep` most recent are kept. If `keep`
/// is 0, only pinned messages survive. Original message order is preserved in the
/// output.
///
/// # Examples
///
/// ```ignore
/// use neuron_context_engine::{ops::compact::Compact, rules::compaction::sliding_window};
///
/// ctx.run(Compact::new(|msgs| sliding_window(msgs, 50))).await?;
/// ```
pub fn sliding_window(messages: &[Message], keep: usize) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Collect indices of non-pinned messages in order (oldest first).
    let non_pinned: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.meta.policy != CompactionPolicy::Pinned)
        .map(|(i, _)| i)
        .collect();

    // Determine which non-pinned messages to keep (the most recent `keep`).
    let keep_start = non_pinned.len().saturating_sub(keep);
    let keep_set: std::collections::HashSet<usize> =
        non_pinned[keep_start..].iter().copied().collect();

    // Return pinned messages and kept non-pinned messages, in original order.
    messages
        .iter()
        .enumerate()
        .filter(|(i, m)| m.meta.policy == CompactionPolicy::Pinned || keep_set.contains(i))
        .map(|(_, m)| m.clone())
        .collect()
}

/// Remove messages by policy priority until `messages.len() <= target`.
///
/// Removal priority (oldest first within each tier):
/// 1. [`CompactionPolicy::DiscardWhenDone`]
/// 2. [`CompactionPolicy::CompressFirst`]
/// 3. [`CompactionPolicy::Normal`]
///
/// [`CompactionPolicy::Pinned`] messages are never removed. If only pinned
/// messages remain before reaching `target`, those are returned as-is. If
/// `messages.len() <= target`, the input is returned unchanged.
pub fn policy_trim(messages: &[Message], target: usize) -> Vec<Message> {
    if messages.len() <= target {
        return messages.to_vec();
    }

    // Collect removable indices per tier, in original order (oldest first).
    let discard: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.meta.policy == CompactionPolicy::DiscardWhenDone)
        .map(|(i, _)| i)
        .collect();

    let compress: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.meta.policy == CompactionPolicy::CompressFirst)
        .map(|(i, _)| i)
        .collect();

    let normal: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.meta.policy == CompactionPolicy::Normal)
        .map(|(i, _)| i)
        .collect();

    let mut remove_set = std::collections::HashSet::new();
    let mut remaining = messages.len();

    for idx in discard {
        if remaining <= target {
            break;
        }
        remove_set.insert(idx);
        remaining -= 1;
    }

    for idx in compress {
        if remaining <= target {
            break;
        }
        remove_set.insert(idx);
        remaining -= 1;
    }

    for idx in normal {
        if remaining <= target {
            break;
        }
        remove_set.insert(idx);
        remaining -= 1;
    }

    messages
        .iter()
        .enumerate()
        .filter(|(i, _)| !remove_set.contains(i))
        .map(|(_, m)| m.clone())
        .collect()
}

/// A reactive rule that compacts messages when count exceeds a threshold.
///
/// Wraps a [`CompactionStrategy`] and fires as a [`Rule`] via
/// [`CompactionRule::into_rule`]. The rule triggers at the start of every
/// [`Context::run()`] call when `messages.len() > max_messages`.
///
/// # Example
///
/// ```ignore
/// use neuron_context_engine::rules::CompactionRule;
///
/// let mut ctx = Context::new();
/// ctx.add_rule(CompactionRule::policy_trim(100).into_rule());
/// ```
pub struct CompactionRule {
    /// The strategy applied when the rule fires.
    strategy: CompactionStrategy,
    /// Fire when message count exceeds this value.
    max_messages: usize,
}

impl CompactionRule {
    /// Create a rule using [`CompactionStrategy::SlidingWindow`].
    ///
    /// Fires when `messages.len() > keep`, then retains only the most recent
    /// `keep` non-pinned messages (plus all pinned messages).
    pub fn sliding_window(keep: usize) -> Self {
        Self {
            strategy: CompactionStrategy::SlidingWindow { keep },
            max_messages: keep,
        }
    }

    /// Create a rule using [`CompactionStrategy::PolicyTrim`].
    ///
    /// Fires when `messages.len() > target`, then removes messages by policy
    /// priority until count reaches `target`.
    pub fn policy_trim(target: usize) -> Self {
        Self {
            strategy: CompactionStrategy::PolicyTrim { target },
            max_messages: target,
        }
    }

    /// Convert this into a [`Rule`] that fires automatically.
    ///
    /// Uses [`Trigger::When`] — evaluates at the start of every [`Context::run()`]
    /// call. Priority is 50: below budget guards (default 100) and above
    /// telemetry (0).
    pub fn into_rule(self) -> Rule {
        let max = self.max_messages;
        Rule::new(
            "compaction",
            Trigger::When(Box::new(move |ctx| ctx.messages.len() > max)),
            50,
            self,
        )
    }
}

#[async_trait]
impl ContextOp for CompactionRule {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        let before = ctx.messages.len();

        ctx.messages = match &self.strategy {
            CompactionStrategy::SlidingWindow { keep } => sliding_window(&ctx.messages, *keep),
            CompactionStrategy::PolicyTrim { target } => policy_trim(&ctx.messages, *target),
        };

        let after = ctx.messages.len();
        tracing::info!(
            messages_before = before,
            messages_after = after,
            "neuron.compaction"
        );

        Ok(())
    }
}

/// Summarize a slice of messages into a single pinned assistant message.
///
/// Calls the provider with a system prompt that instructs the model to produce
/// a concise summary preserving key decisions, unresolved questions, facts
/// established, and current task state. The returned message has
/// [`CompactionPolicy::Pinned`] so it survives further compaction.
///
/// The caller decides where to inject the result; this function does not
/// mutate any [`crate::context::Context`].
///
/// # Errors
///
/// Returns [`EngineError::Halted`] if the provider returns an empty response.
/// Returns [`EngineError::Provider`] if the provider call fails.
pub async fn summarize<P: Provider>(
    messages: &[Message],
    provider: &P,
) -> Result<Message, EngineError> {
    let request = InferRequest::new(messages.to_vec())
        .with_system(
            "Summarize the conversation so far. Preserve: (1) key decisions made, (2) unresolved questions, (3) facts established, (4) current task state. Be concise but lose no critical information.",
        )
        .with_max_tokens(2048);
    let response = provider.infer(request).await?;
    let is_empty = response.text().is_none_or(str::is_empty);
    if is_empty {
        return Err(EngineError::Halted {
            reason: "summarization produced empty response".into(),
        });
    }
    let mut msg = Message::new(Role::Assistant, response.content);
    msg.meta.policy = CompactionPolicy::Pinned;
    Ok(msg)
}

/// Extract the current cognitive state from a slice of messages as JSON.
///
/// Calls the provider with a system prompt instructing it to return JSON matching
/// the provided schema. The response text is parsed as [`serde_json::Value`] and
/// returned directly.
///
/// The caller decides what to do with the result; this function does not
/// mutate any [`crate::context::Context`].
///
/// # Errors
///
/// Returns [`EngineError::Halted`] if the response cannot be parsed as JSON.
/// Returns [`EngineError::Provider`] if the provider call fails.
pub async fn extract_cognitive_state<P: Provider>(
    messages: &[Message],
    provider: &P,
    schema: &serde_json::Value,
) -> Result<serde_json::Value, EngineError> {
    let schema_pretty = serde_json::to_string_pretty(schema)
        .unwrap_or_else(|_| schema.to_string());
    let system = format!(
        "Extract the current cognitive state from this conversation according to the following JSON schema. Return ONLY valid JSON, no explanation.\n\nSchema:\n{schema_pretty}"
    );
    let request = InferRequest::new(messages.to_vec())
        .with_system(system)
        .with_max_tokens(4096);
    let response = provider.infer(request).await?;
    let text = response.text().unwrap_or("");
    let trimmed = strip_json_fences(text);
    serde_json::from_str(trimmed).map_err(|err| EngineError::Halted {
        reason: format!("cognitive state extraction failed to parse: {err}"),
    })
}

/// Strip markdown code fences from a JSON response.
///
/// Models often wrap JSON in ````json\n...\n```` even when instructed not to.
fn strip_json_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim().strip_suffix("```").unwrap_or(rest.trim()).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim().strip_suffix("```").unwrap_or(rest.trim()).trim()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::context::{Message, Role};
    use layer0::lifecycle::CompactionPolicy;
    use neuron_turn::test_utils::{TestProvider, make_text_response};

    /// Build a normal (non-pinned) message.
    fn msg(text: &str) -> Message {
        Message::new(Role::User, Content::text(text))
    }

    /// Build a message with an explicit policy.
    fn msg_with_policy(text: &str, policy: CompactionPolicy) -> Message {
        let mut m = Message::new(Role::User, Content::text(text));
        m.meta.policy = policy;
        m
    }

    /// A no-op context operation used to trigger rule evaluation in tests.
    struct Noop;

    #[async_trait]
    impl ContextOp for Noop {
        type Output = ();
        async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
            Ok(())
        }
    }

    #[test]
    fn sliding_window_keeps_recent() {
        let messages: Vec<Message> = (0..10).map(|i| msg(&format!("msg {i}"))).collect();
        let result = sliding_window(&messages, 3);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].text_content(), "msg 7");
        assert_eq!(result[1].text_content(), "msg 8");
        assert_eq!(result[2].text_content(), "msg 9");
    }

    #[test]
    fn sliding_window_preserves_pinned() {
        // 10 messages: positions 0 and 5 are pinned, rest are normal.
        let mut messages: Vec<Message> = (0..10).map(|i| msg(&format!("msg {i}"))).collect();
        messages[0].meta.policy = CompactionPolicy::Pinned;
        messages[5].meta.policy = CompactionPolicy::Pinned;

        // Non-pinned: indices 1,2,3,4,6,7,8,9 (8 total). keep=3 → indices 7,8,9.
        let result = sliding_window(&messages, 3);

        assert_eq!(result.len(), 5, "pinned(2) + kept non-pinned(3) = 5");

        let texts: Vec<String> = result.iter().map(|m| m.text_content()).collect();
        assert!(
            texts.contains(&"msg 0".to_string()),
            "pinned at 0 must survive"
        );
        assert!(
            texts.contains(&"msg 5".to_string()),
            "pinned at 5 must survive"
        );
        assert!(texts.contains(&"msg 7".to_string()));
        assert!(texts.contains(&"msg 8".to_string()));
        assert!(texts.contains(&"msg 9".to_string()));
    }

    #[test]
    fn policy_trim_removes_discard_first() {
        // 6 messages: 2 DiscardWhenDone, 2 Normal, 1 CompressFirst, 1 Pinned.
        // target=3 — need to remove 3. Should remove both DiscardWhenDone and
        // 1 CompressFirst (which is older than the remaining Normal messages).
        let messages = vec![
            msg_with_policy("discard-a", CompactionPolicy::DiscardWhenDone),
            msg_with_policy("discard-b", CompactionPolicy::DiscardWhenDone),
            msg_with_policy("compress-a", CompactionPolicy::CompressFirst),
            msg_with_policy("normal-a", CompactionPolicy::Normal),
            msg_with_policy("normal-b", CompactionPolicy::Normal),
            msg_with_policy("pinned", CompactionPolicy::Pinned),
        ];

        let result = policy_trim(&messages, 3);

        assert_eq!(result.len(), 3);
        let texts: Vec<String> = result.iter().map(|m| m.text_content()).collect();
        // Both DiscardWhenDone messages must be gone.
        assert!(!texts.contains(&"discard-a".to_string()));
        assert!(!texts.contains(&"discard-b".to_string()));
        // Pinned must survive.
        assert!(texts.contains(&"pinned".to_string()));
    }

    #[test]
    fn policy_trim_then_compress_first() {
        // 5 messages: 1 DiscardWhenDone, 2 CompressFirst, 2 Normal (no Pinned).
        // target=2 — need to remove 3: 1 DiscardWhenDone + 2 CompressFirst.
        let messages = vec![
            msg_with_policy("discard-a", CompactionPolicy::DiscardWhenDone),
            msg_with_policy("compress-a", CompactionPolicy::CompressFirst),
            msg_with_policy("compress-b", CompactionPolicy::CompressFirst),
            msg_with_policy("normal-a", CompactionPolicy::Normal),
            msg_with_policy("normal-b", CompactionPolicy::Normal),
        ];

        let result = policy_trim(&messages, 2);

        assert_eq!(result.len(), 2);
        let texts: Vec<String> = result.iter().map(|m| m.text_content()).collect();
        // Remaining should be the two Normal messages.
        assert!(texts.contains(&"normal-a".to_string()));
        assert!(texts.contains(&"normal-b".to_string()));
        // DiscardWhenDone and both CompressFirst messages must be gone.
        assert!(!texts.contains(&"discard-a".to_string()));
        assert!(!texts.contains(&"compress-a".to_string()));
        assert!(!texts.contains(&"compress-b".to_string()));
    }

    #[test]
    fn policy_trim_preserves_pinned() {
        // 5 messages: 2 Pinned, 3 Normal. target=0 — only pinned should survive.
        let messages = vec![
            msg_with_policy("pinned-a", CompactionPolicy::Pinned),
            msg_with_policy("normal-a", CompactionPolicy::Normal),
            msg_with_policy("pinned-b", CompactionPolicy::Pinned),
            msg_with_policy("normal-b", CompactionPolicy::Normal),
            msg_with_policy("normal-c", CompactionPolicy::Normal),
        ];

        let result = policy_trim(&messages, 0);

        assert_eq!(result.len(), 2, "only the 2 pinned messages survive");
        let texts: Vec<String> = result.iter().map(|m| m.text_content()).collect();
        assert!(texts.contains(&"pinned-a".to_string()));
        assert!(texts.contains(&"pinned-b".to_string()));
    }

    #[tokio::test]
    async fn compaction_rule_fires_when_over_limit() {
        let mut ctx = Context::new();

        // Add 20 normal messages.
        for i in 0..20 {
            ctx.messages
                .push(Message::new(Role::User, Content::text(format!("msg {i}"))));
        }

        // Rule: keep at most 5 non-pinned messages. Fires when len > 5.
        ctx.add_rule(CompactionRule::sliding_window(5).into_rule());

        // Trigger rule evaluation by running a no-op.
        ctx.run(Noop).await.unwrap();

        assert!(
            ctx.messages.len() <= 5,
            "expected <=5 messages after compaction, got {}",
            ctx.messages.len()
        );
    }

    #[tokio::test]
    async fn compaction_rule_noop_under_limit() {
        let mut ctx = Context::new();

        // Add 10 messages — well under the max_messages=50 threshold.
        for i in 0..10 {
            ctx.messages
                .push(Message::new(Role::User, Content::text(format!("msg {i}"))));
        }

        // Rule fires when len > 50. With only 10 messages, it must not fire.
        ctx.add_rule(CompactionRule::sliding_window(50).into_rule());

        ctx.run(Noop).await.unwrap();

        assert_eq!(
            ctx.messages.len(),
            10,
            "no compaction expected under the threshold"
        );
    }
    #[tokio::test]
    async fn summarize_produces_pinned_message() {
        let provider =
            TestProvider::with_responses(vec![make_text_response("Summary: key facts here")]);
        let messages: Vec<Message> = (0..5).map(|i| msg(&format!("msg {i}"))).collect();
        let result = summarize(&messages, &provider).await.unwrap();
        assert_eq!(result.role, Role::Assistant);
        assert_eq!(result.meta.policy, CompactionPolicy::Pinned);
        assert_eq!(result.text_content(), "Summary: key facts here");
    }

    #[tokio::test]
    async fn summarize_empty_response_errors() {
        let provider = TestProvider::with_responses(vec![make_text_response("")]);
        let messages: Vec<Message> = (0..5).map(|i| msg(&format!("msg {i}"))).collect();
        let result = summarize(&messages, &provider).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn extract_cognitive_state_parses_json() {
        let json_response = r#"{"goal": "test", "progress": 50}"#;
        let provider =
            TestProvider::with_responses(vec![make_text_response(json_response)]);
        let messages: Vec<Message> = (0..3).map(|i| msg(&format!("msg {i}"))).collect();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string"},
                "progress": {"type": "number"}
            }
        });
        let result = extract_cognitive_state(&messages, &provider, &schema)
            .await
            .unwrap();
        assert_eq!(result["goal"], "test");
        assert_eq!(result["progress"], 50);
    }

    #[tokio::test]
    async fn extract_cognitive_state_invalid_json_errors() {
        let provider =
            TestProvider::with_responses(vec![make_text_response("not json at all")]);
        let messages: Vec<Message> = (0..3).map(|i| msg(&format!("msg {i}"))).collect();
        let schema = serde_json::json!({});
        let result = extract_cognitive_state(&messages, &provider, &schema).await;
        assert!(result.is_err());
    }

}
