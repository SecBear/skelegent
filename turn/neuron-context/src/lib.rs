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

/// Configuration for [`tiered_compactor`].
///
/// Mirrors the zone model from `TieredStrategy` in `neuron-turn`, adapted for the
/// closure-based compactor API where a message's token count comes from
/// [`Message::estimated_tokens`] rather than a `chars_per_token` ratio.
#[derive(Debug, Clone)]
pub struct TieredConfig {
    /// How many of the most-recent unpinned, non-noise messages to keep in the
    /// active zone. Messages beyond this count (older normal messages) become
    /// summary candidates and are discarded (this closure variant has no LLM
    /// summariser; see `TieredStrategy` for that). Default: 10.
    pub active_zone_size: usize,
}

impl Default for TieredConfig {
    fn default() -> Self {
        Self { active_zone_size: 10 }
    }
}

/// Zone-partitioned compactor: partitions messages into four zones and retains
/// only the pinned and active zones.
///
/// ## Zone model
///
/// | Zone | Policy | Action |
/// |------|--------|--------|
/// | Pinned | `Pinned` | Always kept |
/// | Active | Most-recent `active_zone_size` normal messages | Always kept |
/// | Summary candidates | Older normal messages | Discarded |
/// | Noise | `DiscardWhenDone` or `CompressFirst` | Discarded |
///
/// Pinned messages survive unconditionally. `DiscardWhenDone` and `CompressFirst`
/// messages are dropped immediately. Older normal messages that fall outside the
/// active window are also dropped — this closure variant has no LLM summariser.
/// Wire in `TieredStrategy::with_summariser` if first-generation summarisation is
/// required.
///
/// The returned closure is passed to `Context::compact_with()`.
pub fn tiered_compactor(config: TieredConfig) -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        let mut pinned: Vec<Message> = Vec::new();
        let mut normal: Vec<Message> = Vec::new();

        for msg in msgs {
            match msg.meta.policy {
                CompactionPolicy::Pinned => pinned.push(msg.clone()),
                // Noise zone: DiscardWhenDone and CompressFirst are dropped.
                CompactionPolicy::DiscardWhenDone | CompactionPolicy::CompressFirst => {}
                // Normal zone: eligible for active/summary split.
                CompactionPolicy::Normal => normal.push(msg.clone()),
            }
        }

        // Partition normal messages into summary candidates (older) and active
        // (most-recent `active_zone_size`). Summary candidates are discarded here
        // since this closure variant has no summariser.
        let active_size = config.active_zone_size.min(normal.len());
        let split_point = normal.len().saturating_sub(active_size);
        let active = normal.split_off(split_point); // normal[split_point..]
        drop(normal); // summary candidates — discarded

        // Result order: [pinned] then [active zone].
        let mut result = pinned;
        result.extend(active);
        result
    }
}

/// Salience-aware MMR compactor: iteratively selects messages that maximise
/// Maximal Marginal Relevance (λ·salience − (1−λ)·max_redundancy).
///
/// The returned closure is passed to `Context::compact_with()`.
/// Pinned messages always survive. Remaining token budget is filled by
/// greedily selecting from candidates using term-Jaccard similarity.
///
/// See [`SaliencePackingStrategy`] for the `ContextStrategy` equivalent.
pub fn salience_packing_compactor(
    config: SaliencePackingConfig,
) -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        // Phase 1: Partition into pinned and candidates.
        let (pinned, mut candidates): (Vec<_>, Vec<_>) = msgs
            .iter()
            .partition(|m| m.meta.policy == CompactionPolicy::Pinned);

        // Phase 2: Budget calculation.
        let pinned_tokens: usize = pinned.iter().map(|m| m.estimated_tokens()).sum();
        if pinned_tokens >= config.token_budget {
            // Pinned alone exceeds budget — return them and nothing else.
            return pinned.into_iter().cloned().collect();
        }
        let mut remaining = config.token_budget - pinned_tokens;

        // Phase 3: Iterative MMR selection.
        let mut selected: Vec<&Message> = Vec::new();
        let mut selected_texts: Vec<String> = Vec::new();

        while !candidates.is_empty() && remaining > 0 {
            let mut best_idx: Option<usize> = None;
            let mut best_mmr = f64::NEG_INFINITY;

            for (i, candidate) in candidates.iter().enumerate() {
                let sim1 = candidate.meta.salience.unwrap_or(config.default_salience);

                // Max redundancy against already-selected set.
                let sim2 = if selected_texts.is_empty() {
                    0.0
                } else {
                    let cand_text = candidate.text_content();
                    selected_texts
                        .iter()
                        .map(|s| salience_packing::term_jaccard(&cand_text, s))
                        .fold(0.0_f64, f64::max)
                };

                let mmr = config.lambda * sim1 - (1.0 - config.lambda) * sim2;

                if mmr > best_mmr {
                    best_mmr = mmr;
                    best_idx = Some(i);
                }
            }

            // Safety: candidates is non-empty so best_idx is always Some.
            let idx = best_idx.expect("candidates non-empty");
            let best = candidates.remove(idx);
            let tokens = best.estimated_tokens();

            if tokens <= remaining {
                remaining -= tokens;
                selected_texts.push(best.text_content());
                selected.push(best);
            }
            // else: candidate doesn't fit, already removed from pool. Loop
            // continues to try smaller candidates.
        }

        // Phase 4: Optional "lost in the middle" reordering.
        if config.reorder_for_recall && selected.len() > 2 {
            // Sort by salience descending.
            selected.sort_by(|a, b| {
                b.meta
                    .salience
                    .unwrap_or(config.default_salience)
                    .partial_cmp(&a.meta.salience.unwrap_or(config.default_salience))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let len = selected.len();
            let mut reordered: Vec<Option<&Message>> = (0..len).map(|_| None).collect();
            let mut left = 0;
            let mut right = len - 1;

            for (i, item) in selected.into_iter().enumerate() {
                if i % 2 == 0 {
                    reordered[left] = Some(item);
                    left += 1;
                } else {
                    reordered[right] = Some(item);
                    if right == 0 {
                        break;
                    }
                    right -= 1;
                }
            }

            selected = reordered.into_iter().flatten().collect();
        }

        // Phase 5: Emit [pinned] ++ [selected].
        let mut result: Vec<Message> = pinned.into_iter().cloned().collect();
        result.extend(selected.into_iter().cloned());
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
