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
use layer0::context::Role;
use layer0::lifecycle::CompactionPolicy;
use neuron_turn::infer::{InferRequest, InferResponse};
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
    /// Custom trigger predicate, if any.
    custom_trigger: Option<Trigger>,
    /// Rule priority (default: 50).
    priority: i32,
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
            custom_trigger: None,
            priority: 50,
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
            custom_trigger: None,
            priority: 50,
        }
    }

    /// Convert this into a [`Rule`] that fires automatically.
    ///
    /// Uses [`Trigger::When`] — evaluates at the start of every [`Context::run()`]
    /// call. Priority is 50: below budget guards (default 100) and above
    /// telemetry (0).
    /// Override the trigger predicate.
    ///
    /// By default, the rule fires when `messages.len() > max_messages`.
    /// Use this to supply a custom predicate.
    pub fn with_trigger(mut self, trigger: Trigger) -> Self {
        self.custom_trigger = Some(trigger);
        self
    }

    /// Override the rule priority (default: 50).
    ///
    /// Higher priority rules fire first. Budget guards default to 100,
    /// compaction to 50, telemetry to 0.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Convert this into a [`Rule`] that fires automatically.
    ///
    /// Uses [`Trigger::When`] — evaluates at the start of every [`Context::run()`]
    /// call. Priority is 50: below budget guards (default 100) and above
    /// telemetry (0).
    pub fn into_rule(mut self) -> Rule {
        let max = self.max_messages;
        let trigger = self
            .custom_trigger
            .take()
            .unwrap_or_else(|| Trigger::When(Box::new(move |ctx| ctx.messages.len() > max)));
        let priority = self.priority;
        Rule::new("compaction", trigger, priority, self)
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

/// Default system prompt for [`summarize`] and [`summarize_with`].
pub const DEFAULT_SUMMARY_PROMPT: &str = "Summarize the conversation so far. Preserve: (1) key decisions made, (2) unresolved questions, (3) facts established, (4) current task state. Be concise but lose no critical information.";

/// Configuration for LLM-driven summarization.
#[derive(Debug, Clone)]
pub struct SummarizeConfig {
    /// System prompt instructing the model how to summarize.
    pub prompt: String,
    /// Maximum output tokens for the summarization call.
    pub max_tokens: u32,
    /// Compaction policy applied to the returned summary message.
    pub output_policy: CompactionPolicy,
    /// Model to use for the summarization call. `None` uses the provider's default.
    pub model: Option<String>,
}

impl Default for SummarizeConfig {
    fn default() -> Self {
        Self {
            prompt: DEFAULT_SUMMARY_PROMPT.into(),
            max_tokens: 2048,
            output_policy: CompactionPolicy::Pinned,
            model: None,
        }
    }
}

impl SummarizeConfig {
    /// Build an [`InferRequest`] for summarization.
    ///
    /// This is the request that [`summarize_with()`] sends to the provider.
    /// Use this to inspect or modify the request before calling the provider yourself.
    pub fn build_request(&self, messages: &[Message]) -> InferRequest {
        let mut request = InferRequest::new(messages.to_vec())
            .with_system(&self.prompt)
            .with_max_tokens(self.max_tokens);
        if let Some(ref model) = self.model {
            request = request.with_model(model);
        }
        request
    }

    /// Parse a provider response into a summarization message.
    ///
    /// Validates the response is non-empty and wraps it as a [`Message`] with
    /// the configured [`CompactionPolicy`]. This is the parsing step that
    /// [`summarize_with()`] applies after inference.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Halted`] if the response text is empty.
    pub fn parse_response(&self, response: InferResponse) -> Result<Message, EngineError> {
        let is_empty = response.text().is_none_or(str::is_empty);
        if is_empty {
            return Err(EngineError::Halted {
                reason: "summarization produced empty response".into(),
            });
        }
        let mut msg = Message::new(Role::Assistant, response.content);
        msg.meta.policy = self.output_policy;
        Ok(msg)
    }
}

/// Summarize messages using the default prompt and policy.
/// See [`summarize_with`] for full configuration.
pub async fn summarize<P: Provider>(
    messages: &[Message],
    provider: &P,
) -> Result<Message, EngineError> {
    summarize_with(messages, provider, &SummarizeConfig::default()).await
}

/// Summarize messages with custom configuration.
///
/// Use [`SummarizeConfig`] to control the prompt, max tokens, and output
/// compaction policy. The returned message has the configured
/// [`SummarizeConfig::output_policy`].
///
/// # Errors
///
/// Returns [`EngineError::Halted`] if the provider returns an empty response.
/// Returns [`EngineError::Provider`] if the provider call fails.
pub async fn summarize_with<P: Provider>(
    messages: &[Message],
    provider: &P,
    config: &SummarizeConfig,
) -> Result<Message, EngineError> {
    let request = config.build_request(messages);
    let response = provider.infer(request).await?;
    config.parse_response(response)
}

/// Default system prompt template for [`extract_cognitive_state`].
///
/// The placeholder `{schema}` is replaced with the pretty-printed JSON schema.
pub const DEFAULT_EXTRACT_PROMPT_TEMPLATE: &str = "Extract the current cognitive state from this conversation according to the following JSON schema. Return ONLY valid JSON, no explanation.\n\nSchema:\n{schema}";

/// Configuration for cognitive state extraction.
#[derive(Debug, Clone)]
pub struct ExtractConfig {
    /// System prompt template. Must contain `{schema}` which is replaced with
    /// the pretty-printed schema.
    pub prompt_template: String,
    /// Maximum output tokens.
    pub max_tokens: u32,
    /// Model to use for the extraction call. `None` uses the provider's default.
    pub model: Option<String>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            prompt_template: DEFAULT_EXTRACT_PROMPT_TEMPLATE.into(),
            max_tokens: 4096,
            model: None,
        }
    }
}

impl ExtractConfig {
    /// Build an [`InferRequest`] for cognitive state extraction.
    ///
    /// Substitutes `{schema}` in the prompt template with the pretty-printed schema.
    /// This is the request that [`extract_cognitive_state_with()`] sends to the provider.
    pub fn build_request(&self, messages: &[Message], schema: &serde_json::Value) -> InferRequest {
        let schema_pretty =
            serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        let system = self.prompt_template.replace("{schema}", &schema_pretty);
        let mut request = InferRequest::new(messages.to_vec())
            .with_system(system)
            .with_max_tokens(self.max_tokens);
        if let Some(ref model) = self.model {
            request = request.with_model(model);
        }
        request
    }

    /// Parse a provider response into a cognitive state JSON value.
    ///
    /// Strips markdown JSON fences via [`strip_json_fences()`] and parses
    /// the result as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Halted`] if the response cannot be parsed as JSON.
    pub fn parse_response(
        &self,
        response: &InferResponse,
    ) -> Result<serde_json::Value, EngineError> {
        let text = response.text().unwrap_or("");
        let trimmed = strip_json_fences(text);
        serde_json::from_str(trimmed).map_err(|err| EngineError::Halted {
            reason: format!("cognitive state extraction failed to parse: {err}"),
        })
    }
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
    extract_cognitive_state_with(messages, provider, schema, &ExtractConfig::default()).await
}

/// Extract cognitive state with custom configuration.
///
/// The `config.prompt_template` must contain `{schema}` — it is replaced with
/// the pretty-printed schema.
///
/// # Errors
///
/// Returns [`EngineError::Halted`] if the response cannot be parsed as JSON.
/// Returns [`EngineError::Provider`] if the provider call fails.
pub async fn extract_cognitive_state_with<P: Provider>(
    messages: &[Message],
    provider: &P,
    schema: &serde_json::Value,
    config: &ExtractConfig,
) -> Result<serde_json::Value, EngineError> {
    let request = config.build_request(messages, schema);
    let response = provider.infer(request).await?;
    config.parse_response(&response)
}

/// Strip markdown code fences from a JSON response.
///
/// Models often wrap JSON in ` ```json\n...\n``` ` even when instructed not to.
/// This function removes those fences, returning the inner content trimmed.
///
/// Used internally by [`extract_cognitive_state_with()`]. Exposed as a primitive
/// so developers who write their own extraction pipeline can reuse it.
pub fn strip_json_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim()
            .strip_suffix("```")
            .unwrap_or(rest.trim())
            .trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim()
            .strip_suffix("```")
            .unwrap_or(rest.trim())
            .trim()
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
        let provider = TestProvider::with_responses(vec![make_text_response(json_response)]);
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
        let provider = TestProvider::with_responses(vec![make_text_response("not json at all")]);
        let messages: Vec<Message> = (0..3).map(|i| msg(&format!("msg {i}"))).collect();
        let schema = serde_json::json!({});
        let result = extract_cognitive_state(&messages, &provider, &schema).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn summarize_with_custom_prompt() {
        let provider = TestProvider::with_responses(vec![make_text_response("Custom summary")]);
        let messages: Vec<Message> = (0..3).map(|i| msg(&format!("msg {i}"))).collect();
        let config = SummarizeConfig {
            prompt: "Just say what happened.".into(),
            ..SummarizeConfig::default()
        };
        let result = summarize_with(&messages, &provider, &config).await.unwrap();
        assert_eq!(result.text_content(), "Custom summary");
        assert_eq!(result.meta.policy, CompactionPolicy::Pinned);
    }

    #[tokio::test]
    async fn summarize_with_custom_policy() {
        let provider = TestProvider::with_responses(vec![make_text_response("Summary")]);
        let messages = vec![msg("hello")];
        let config = SummarizeConfig {
            output_policy: CompactionPolicy::Normal,
            ..SummarizeConfig::default()
        };
        let result = summarize_with(&messages, &provider, &config).await.unwrap();
        assert_eq!(result.meta.policy, CompactionPolicy::Normal);
    }

    #[tokio::test]
    async fn extract_cognitive_state_with_custom_prompt() {
        let provider =
            TestProvider::with_responses(vec![make_text_response(r#"{"key": "value"}"#)]);
        let messages = vec![msg("hello")];
        let config = ExtractConfig {
            prompt_template: "Custom extraction: {schema}".into(),
            ..ExtractConfig::default()
        };
        let result =
            extract_cognitive_state_with(&messages, &provider, &serde_json::json!({}), &config)
                .await
                .unwrap();
        assert_eq!(result["key"], "value");
    }

    #[tokio::test]
    async fn summarize_with_model_selection() {
        let provider = TestProvider::with_responses(vec![make_text_response("Model summary")]);
        let messages = vec![msg("hello")];
        let config = SummarizeConfig {
            model: Some("claude-haiku-4-5-20251001".into()),
            ..SummarizeConfig::default()
        };
        let result = summarize_with(&messages, &provider, &config).await.unwrap();
        assert_eq!(result.text_content(), "Model summary");
    }

    #[test]
    fn test_strip_json_fences_with_json_tag() {
        let input = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_json_fences(input), "{\"a\": 1}");
    }

    #[test]
    fn test_strip_json_fences_without_tag() {
        let input = "```\n{\"a\": 1}\n```";
        assert_eq!(strip_json_fences(input), "{\"a\": 1}");
    }

    #[test]
    fn test_strip_json_fences_no_fences() {
        let input = "{\"a\": 1}";
        assert_eq!(strip_json_fences(input), "{\"a\": 1}");
    }

    #[test]
    fn test_build_summarize_request_default() {
        let messages = vec![msg("hello")];
        let config = SummarizeConfig::default();
        let request = config.build_request(&messages);
        assert_eq!(request.max_tokens, Some(2048));
    }

    #[test]
    fn test_build_summarize_request_custom_model() {
        let messages = vec![msg("hello")];
        let config = SummarizeConfig {
            model: Some("custom-model".into()),
            ..SummarizeConfig::default()
        };
        let request = config.build_request(&messages);
        assert_eq!(request.model.as_deref(), Some("custom-model"));
    }

    #[test]
    fn test_parse_summarize_response_empty() {
        let response = make_text_response("");
        let config = SummarizeConfig::default();
        let result = config.parse_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_summarize_response_success() {
        let response = make_text_response("Summary text");
        let config = SummarizeConfig {
            output_policy: CompactionPolicy::Normal,
            ..SummarizeConfig::default()
        };
        let result = config.parse_response(response).unwrap();
        assert_eq!(result.text_content(), "Summary text");
        assert_eq!(result.meta.policy, CompactionPolicy::Normal);
        assert_eq!(result.role, Role::Assistant);
    }

    #[test]
    fn test_build_extract_request_schema_substitution() {
        let messages = vec![msg("hello")];
        let schema = serde_json::json!({"type": "object"});
        let config = ExtractConfig::default();
        let request = config.build_request(&messages, &schema);
        // The system prompt should contain the schema
        let system = request.system.unwrap();
        assert!(
            system.contains("\"type\": \"object\""),
            "system prompt should contain schema: {system}"
        );
    }

    #[test]
    fn test_parse_extract_response_valid_json() {
        let response = make_text_response("{\"key\": \"value\"}");
        let result = ExtractConfig::default().parse_response(&response).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn test_parse_extract_response_with_fences() {
        let response = make_text_response("```json\n{\"key\": \"value\"}\n```");
        let result = ExtractConfig::default().parse_response(&response).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn test_parse_extract_response_invalid() {
        let response = make_text_response("not json");
        let result = ExtractConfig::default().parse_response(&response);
        assert!(result.is_err());
    }

    #[test]
    fn test_compaction_rule_custom_priority() {
        let rule = CompactionRule::sliding_window(10)
            .with_priority(99)
            .into_rule();
        assert_eq!(rule.priority, 99);
    }
}
