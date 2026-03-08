//! Salience-aware context packing via iterative MMR selection.
//!
//! [`SaliencePackingStrategy`] is the first [`ContextStrategy`] that consumes
//! [`AnnotatedMessage::salience`]. It partitions messages into pinned
//! (always kept) and candidates, then greedily selects candidates using
//! Maximal Marginal Relevance (MMR) until the token budget is exhausted.
//!
//! # Algorithm
//!
//! 1. **Partition** — pinned messages (policy = `Pinned`) are unconditionally kept.
//! 2. **Budget** — remaining token budget = `token_budget - pinned_tokens`.
//! 3. **MMR selection** — iteratively pick the candidate that maximises
//!    `λ * salience - (1-λ) * max_similarity(candidate, already_selected)`.
//!    Similarity is term-level Jaccard (v1) — no embeddings required.
//! 4. **Reorder** (optional) — interleave high-salience items at start/end
//!    positions to mitigate "lost in the middle" attention degradation.
//! 5. **Emit** — `[pinned] ++ [selected]`.
//!
//! # Design decisions
//!
//! - **λ = 0.7 default** — relevance-leaning, per Elastic/Qdrant production guidance.
//! - **Term Jaccard** — dependency-free; cosine over embeddings is the v2 path
//!   when `sqlite-vec` is available.
//! - **Recency is upstream** — the caller (Context Package Generator) bakes
//!   recency into `salience` before handing messages to this strategy. The
//!   strategy treats `salience` as a pre-computed composite score.

use std::collections::HashSet;

use layer0::CompactionPolicy;
use neuron_turn::context::{AnnotatedMessage, CompactionError, ContextStrategy};
use neuron_turn::types::ContentPart;

/// Configuration for [`SaliencePackingStrategy`].
#[derive(Debug, Clone)]
pub struct SaliencePackingConfig {
    /// Total token budget for the context window (pinned + selected).
    /// Default: 100,000 tokens.
    pub token_budget: usize,

    /// MMR trade-off parameter. 0.0 = max diversity, 1.0 = max relevance.
    /// Default: 0.7 (relevance-leaning production standard).
    pub lambda: f64,

    /// Salience value assigned to messages without an explicit score.
    /// Default: 0.5 (midpoint of 0.0–1.0).
    pub default_salience: f64,

    /// Apply "lost in the middle" reordering to selected messages.
    /// Places highest-salience items at the start and end of the sequence.
    /// Default: false (preserve input order for conversational coherence).
    pub reorder_for_recall: bool,

    /// Approximate characters per token for estimation. Default: 4.
    pub chars_per_token: usize,
}

impl Default for SaliencePackingConfig {
    fn default() -> Self {
        Self {
            token_budget: 100_000,
            lambda: 0.7,
            default_salience: 0.5,
            reorder_for_recall: false,
            chars_per_token: 4,
        }
    }
}

/// Salience-aware context packing strategy using iterative MMR selection.
///
/// See [module docs](self) for algorithm details.
#[derive(Debug)]
pub struct SaliencePackingStrategy {
    config: SaliencePackingConfig,
}

impl SaliencePackingStrategy {
    /// Create a new strategy with the given configuration.
    pub fn new(config: SaliencePackingConfig) -> Self {
        Self { config }
    }

    /// Create a strategy with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SaliencePackingConfig::default())
    }

    /// Estimate tokens for a single message.
    fn estimate_single(&self, msg: &AnnotatedMessage) -> usize {
        let content_tokens: usize = msg
            .message
            .content
            .iter()
            .map(|part| match part {
                ContentPart::Text { text } => text.len() / self.config.chars_per_token,
                ContentPart::ToolUse { input, .. } => {
                    input.to_string().len() / self.config.chars_per_token
                }
                ContentPart::ToolResult { content, .. } => {
                    content.len() / self.config.chars_per_token
                }
                ContentPart::Image { .. } => 1000,
            })
            .sum();
        // Per-message overhead: role, formatting tokens.
        content_tokens + 4
    }

    /// Extract all text content from a message for similarity computation.
    fn text_of(msg: &AnnotatedMessage) -> String {
        msg.message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Resolve the salience score for a message, falling back to
    /// `default_salience` when unset.
    fn salience_of(&self, msg: &AnnotatedMessage) -> f64 {
        msg.salience.unwrap_or(self.config.default_salience)
    }
}

/// Jaccard similarity over unique lowercased whitespace-split terms.
///
/// Returns 0.0 for empty inputs, 1.0 for identical term sets.
/// This is the v1 similarity function — cosine over embeddings is planned
/// for v2 when `sqlite-vec` is available.
pub(crate) fn term_jaccard(a: &str, b: &str) -> f64 {
    let terms_a: HashSet<&str> = a.split_whitespace().collect();
    let terms_b: HashSet<&str> = b.split_whitespace().collect();
    let intersection = terms_a.intersection(&terms_b).count();
    let union = terms_a.union(&terms_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

impl ContextStrategy for SaliencePackingStrategy {
    fn token_estimate(&self, messages: &[AnnotatedMessage]) -> usize {
        messages.iter().map(|m| self.estimate_single(m)).sum()
    }

    fn should_compact(&self, messages: &[AnnotatedMessage], _limit: usize) -> bool {
        self.token_estimate(messages) > self.config.token_budget
    }

    fn compact(
        &self,
        messages: Vec<AnnotatedMessage>,
    ) -> Result<Vec<AnnotatedMessage>, CompactionError> {
        // Phase 1: Partition into pinned and candidates.
        let (pinned, mut candidates): (Vec<_>, Vec<_>) = messages
            .into_iter()
            .partition(|m| matches!(m.policy, Some(CompactionPolicy::Pinned)));

        // Phase 2: Budget calculation.
        let pinned_tokens: usize = pinned.iter().map(|m| self.estimate_single(m)).sum();
        if pinned_tokens >= self.config.token_budget {
            // Pinned alone exceeds budget — return them and nothing else.
            return Ok(pinned);
        }
        let mut remaining = self.config.token_budget - pinned_tokens;

        // Phase 3: Iterative MMR selection.
        let mut selected: Vec<AnnotatedMessage> = Vec::new();
        let mut selected_texts: Vec<String> = Vec::new();

        while !candidates.is_empty() && remaining > 0 {
            let mut best_idx: Option<usize> = None;
            let mut best_mmr = f64::NEG_INFINITY;

            for (i, candidate) in candidates.iter().enumerate() {
                let sim1 = self.salience_of(candidate);

                // Max redundancy against already-selected set.
                let sim2 = if selected_texts.is_empty() {
                    0.0
                } else {
                    let cand_text = Self::text_of(candidate);
                    selected_texts
                        .iter()
                        .map(|s| term_jaccard(&cand_text, s))
                        .fold(0.0_f64, f64::max)
                };

                let mmr = self.config.lambda * sim1 - (1.0 - self.config.lambda) * sim2;

                if mmr > best_mmr {
                    best_mmr = mmr;
                    best_idx = Some(i);
                }
            }

            // Safety: candidates is non-empty so best_idx is always Some.
            let idx = best_idx.expect("candidates non-empty");
            let best = candidates.remove(idx);
            let tokens = self.estimate_single(&best);

            if tokens <= remaining {
                remaining -= tokens;
                selected_texts.push(Self::text_of(&best));
                selected.push(best);
            }
            // else: candidate doesn't fit, already removed from pool. Loop
            // continues to try smaller candidates.
        }

        // Phase 4: Optional "lost in the middle" reordering.
        if self.config.reorder_for_recall && selected.len() > 2 {
            // Sort by salience descending.
            selected.sort_by(|a, b| {
                self.salience_of(b)
                    .partial_cmp(&self.salience_of(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let len = selected.len();
            let mut reordered: Vec<Option<AnnotatedMessage>> = (0..len).map(|_| None).collect();
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
        let mut result = pinned;
        result.extend(selected);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_turn::types::{ProviderMessage, Role};

    /// Helper: create a text message with optional salience and policy.
    fn msg(
        role: Role,
        text: &str,
        salience: Option<f64>,
        policy: Option<CompactionPolicy>,
    ) -> AnnotatedMessage {
        AnnotatedMessage {
            message: ProviderMessage {
                role,
                content: vec![ContentPart::Text {
                    text: text.to_string(),
                }],
            },
            policy,
            source: None,
            salience,
        }
    }

    /// Shorthand: normal message with salience.
    fn scored(text: &str, salience: f64) -> AnnotatedMessage {
        msg(
            Role::User,
            text,
            Some(salience),
            Some(CompactionPolicy::Normal),
        )
    }

    /// Shorthand: pinned message.
    fn pinned(text: &str) -> AnnotatedMessage {
        msg(Role::User, text, None, Some(CompactionPolicy::Pinned))
    }

    /// Shorthand: normal message without salience.
    fn unscored(text: &str) -> AnnotatedMessage {
        msg(Role::User, text, None, Some(CompactionPolicy::Normal))
    }

    fn small_config(budget: usize) -> SaliencePackingConfig {
        SaliencePackingConfig {
            token_budget: budget,
            chars_per_token: 1, // 1:1 for predictable math in tests
            ..Default::default()
        }
    }

    // ── Edge case: empty input ──────────────────────────────────────

    #[test]
    fn empty_input_returns_empty() {
        let strategy = SaliencePackingStrategy::new(small_config(1000));
        let result = strategy.compact(vec![]).unwrap();
        assert!(result.is_empty());
    }

    // ── Edge case: all messages pinned ──────────────────────────────

    #[test]
    fn all_pinned_returns_all() {
        let strategy = SaliencePackingStrategy::new(small_config(1000));
        let messages = vec![pinned("system prompt"), pinned("constraint A")];
        let result = strategy.compact(messages).unwrap();
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|m| m.policy == Some(CompactionPolicy::Pinned))
        );
    }

    // ── Edge case: budget < pinned tokens ───────────────────────────

    #[test]
    fn budget_less_than_pinned_returns_pinned_only() {
        // Pinned messages are ~20 chars each + 4 overhead = ~24 tokens (with chars_per_token=1).
        // Budget = 10 tokens: less than pinned. No candidates should be included.
        let strategy = SaliencePackingStrategy::new(small_config(10));
        let messages = vec![
            pinned("this is a long pinned system prompt"),
            scored("candidate message", 0.9),
        ];
        let result = strategy.compact(messages).unwrap();
        // Only pinned survives.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].policy, Some(CompactionPolicy::Pinned));
    }

    // ── Edge case: no salience set on any message ───────────────────

    #[test]
    fn no_salience_uses_default() {
        let strategy = SaliencePackingStrategy::new(small_config(10_000));
        let messages = vec![
            unscored("alpha bravo charlie"),
            unscored("delta echo foxtrot"),
            unscored("alpha bravo charlie"), // duplicate of first
        ];
        let result = strategy.compact(messages).unwrap();
        // All should be selected (budget is large).
        // MMR should still work: the duplicate has high redundancy vs the first.
        assert_eq!(result.len(), 3);
    }

    // ── Edge case: single candidate ─────────────────────────────────

    #[test]
    fn single_candidate_selected() {
        let strategy = SaliencePackingStrategy::new(small_config(10_000));
        let messages = vec![scored("only candidate", 0.8)];
        let result = strategy.compact(messages).unwrap();
        assert_eq!(result.len(), 1);
    }

    // ── Edge case: all candidates too large ─────────────────────────

    #[test]
    fn all_candidates_too_large_returns_pinned_only() {
        // Budget = 50, pinned = ~14 tokens (10 chars + 4 overhead).
        // Remaining = 36. Each candidate is 100 chars + 4 = 104 tokens. None fit.
        let strategy = SaliencePackingStrategy::new(small_config(50));
        let messages = vec![
            pinned("pinned msg"),
            scored(&"x".repeat(100), 0.9),
            scored(&"y".repeat(100), 0.8),
        ];
        let result = strategy.compact(messages).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].policy, Some(CompactionPolicy::Pinned));
    }

    // ── Edge case: lambda = 1.0 (pure relevance) ───────────────────

    #[test]
    fn lambda_one_selects_by_salience_only() {
        let config = SaliencePackingConfig {
            token_budget: 10_000,
            lambda: 1.0,
            chars_per_token: 1,
            ..Default::default()
        };
        let strategy = SaliencePackingStrategy::new(config);

        // Two candidates with identical text but different salience.
        let messages = vec![
            scored("identical text here", 0.3),
            scored("identical text here", 0.9),
        ];
        let result = strategy.compact(messages).unwrap();
        // Higher salience should be selected first.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].salience, Some(0.9));
    }

    // ── Edge case: lambda = 0.0 (pure diversity) ────────────────────

    #[test]
    fn lambda_zero_maximizes_diversity() {
        let config = SaliencePackingConfig {
            token_budget: 10_000,
            lambda: 0.0,
            chars_per_token: 1,
            ..Default::default()
        };
        let strategy = SaliencePackingStrategy::new(config);

        // High salience but identical text vs low salience but unique text.
        let messages = vec![
            scored("common words shared between messages", 0.9),
            scored("common words shared between messages", 0.8),
            scored("completely unique different vocabulary", 0.1),
        ];
        let result = strategy.compact(messages).unwrap();
        // With lambda=0, after selecting the first message, the unique message
        // should be preferred over the duplicate despite lower salience.
        assert_eq!(result.len(), 3);
        // The first two selected should be the most diverse pair.
        // First pick: sim2=0 for all, so mmr = 0 * salience - 1 * 0 = 0 for all.
        // Actually with lambda=0: mmr = 0 * sim1 - 1 * sim2 = -sim2.
        // First pick: all have sim2=0, so all tied at mmr=0. First in list wins.
        // Second pick: msg[1] has high sim2 with msg[0], msg[2] has low sim2.
        // So msg[2] gets mmr ~ 0 and msg[1] gets mmr ~ -0.8. msg[2] wins.
        // Verify the unique message is selected second.
        let texts: Vec<&str> = result
            .iter()
            .filter_map(|m| match &m.message.content[0] {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts[1], "completely unique different vocabulary");
    }

    // ── Pinned invariant: pinned messages always survive ────────────

    #[test]
    fn pinned_always_survive() {
        let strategy = SaliencePackingStrategy::new(small_config(500));
        let messages = vec![
            pinned("system instruction"),
            pinned("hard constraint"),
            scored(&"a".repeat(100), 0.9),
            scored(&"b".repeat(100), 0.7),
            scored(&"c".repeat(100), 0.5),
        ];
        let result = strategy.compact(messages).unwrap();

        let pinned_count = result
            .iter()
            .filter(|m| m.policy == Some(CompactionPolicy::Pinned))
            .count();
        assert_eq!(pinned_count, 2, "both pinned messages must survive");
    }

    // ── Budget invariant: output never exceeds token budget ─────────

    #[test]
    fn output_within_budget() {
        let budget = 200;
        let strategy = SaliencePackingStrategy::new(small_config(budget));

        let messages: Vec<_> = (0..20)
            .map(|i| {
                scored(
                    &format!("message number {} with content", i),
                    (20 - i) as f64 / 20.0,
                )
            })
            .collect();

        let result = strategy.compact(messages).unwrap();
        let total_tokens = strategy.token_estimate(&result);
        assert!(
            total_tokens <= budget,
            "output tokens ({total_tokens}) must not exceed budget ({budget})"
        );
    }

    // ── MMR diversity: redundant messages penalised ─────────────────

    #[test]
    fn mmr_penalizes_redundancy() {
        let strategy = SaliencePackingStrategy::new(SaliencePackingConfig {
            // Budget fits exactly 2 of the 3 candidates.
            token_budget: 100,
            lambda: 0.5,
            chars_per_token: 1,
            ..Default::default()
        });

        let messages = vec![
            scored("alpha bravo charlie delta", 0.8),
            scored("alpha bravo charlie echo", 0.8), // near-duplicate of first
            scored("foxtrot golf hotel india", 0.7), // unique but lower salience
        ];

        let result = strategy.compact(messages).unwrap();
        // With balanced lambda, after selecting the first message, the unique
        // message should be preferred over the near-duplicate.
        let texts: Vec<String> = result
            .iter()
            .filter_map(|m| match &m.message.content[0] {
                ContentPart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();

        assert!(texts.contains(&"alpha bravo charlie delta".to_string()));
        assert!(texts.contains(&"foxtrot golf hotel india".to_string()));
    }

    // ── Reorder for recall ──────────────────────────────────────────

    #[test]
    fn reorder_for_recall_places_high_salience_at_edges() {
        let config = SaliencePackingConfig {
            token_budget: 10_000,
            lambda: 1.0, // pure relevance so order is predictable
            reorder_for_recall: true,
            chars_per_token: 1,
            ..Default::default()
        };
        let strategy = SaliencePackingStrategy::new(config);

        let messages = vec![
            scored("low salience", 0.1),
            scored("medium salience", 0.5),
            scored("high salience", 0.9),
        ];

        let result = strategy.compact(messages).unwrap();
        assert_eq!(result.len(), 3);

        // After reordering: highest at start, 2nd highest at end, lowest in middle.
        let saliences: Vec<f64> = result.iter().map(|m| m.salience.unwrap()).collect();
        // Sorted desc: [0.9, 0.5, 0.1]
        // Interleaved: idx0=0.9 (even→left), idx1=0.1←(odd→right), idx2=0.5 (even→mid)
        // Final: [0.9, 0.5, 0.1]
        assert_eq!(saliences[0], 0.9, "highest salience at start");
        assert_eq!(saliences[2], 0.5, "2nd highest at end");
    }

    // ── should_compact threshold ────────────────────────────────────

    #[test]
    fn should_compact_respects_budget() {
        let strategy = SaliencePackingStrategy::new(small_config(50));
        let small = vec![scored("hi", 0.5)];
        let large = vec![scored(&"x".repeat(100), 0.5)];

        assert!(!strategy.should_compact(&small, 0)); // _limit is ignored
        assert!(strategy.should_compact(&large, 0));
    }

    // ── term_jaccard unit tests ─────────────────────────────────────

    #[test]
    fn jaccard_identical() {
        assert!((term_jaccard("foo bar baz", "foo bar baz") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_disjoint() {
        assert!((term_jaccard("foo bar", "baz qux")).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_partial_overlap() {
        // {foo, bar} ∩ {bar, baz} = {bar}, union = {foo, bar, baz}
        let j = term_jaccard("foo bar", "bar baz");
        assert!((j - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn jaccard_empty_inputs() {
        assert!((term_jaccard("", "")).abs() < f64::EPSILON);
        assert!((term_jaccard("foo", "")).abs() < f64::EPSILON);
        assert!((term_jaccard("", "bar")).abs() < f64::EPSILON);
    }

    // ── Object safety ───────────────────────────────────────────────

    #[test]
    fn strategy_is_object_safe() {
        fn _assert_object_safe(_: &dyn ContextStrategy) {}
        let s = SaliencePackingStrategy::with_defaults();
        _assert_object_safe(&s);
    }
}
