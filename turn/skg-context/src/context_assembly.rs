//! Context assembly for sweep agents.
//!
//! Assembles a [`Vec<Message>`] from state store data for a given
//! decision ID. The output is intended for downstream compaction by
//! [`salience_packing_compactor`](crate::salience_packing_compactor).
//!
//! # Assembly pipeline
//!
//! 1. **System instructions** (Pinned) — optional caller-supplied prompt.
//! 2. **Decision card** (Pinned, salience = 1.0) — the rolling summary.
//! 3. **Recent deltas** (Normal, recency-scored) — latest changes/findings.
//! 4. **FTS search hits** (Normal, BM25-scored) — related evidence.
//! 5. **Combine** — all messages, ready for
//!    [`salience_packing_compactor`](crate::salience_packing_compactor).
//!
//! # Key schema
//!
//! The assembler expects entries in the state store keyed as:
//! - `card:{decision_id}` — JSON value for the rolling decision summary.
//! - `delta:{decision_id}:{timestamp_micros}` — one JSON value per finding.
//!
//! FTS search uses the `decision_id` as the query term.

use std::time::{SystemTime, UNIX_EPOCH};

use layer0::CompactionPolicy;
use layer0::Content;
use layer0::context::{Message, MessageMeta, Role};
use layer0::error::StateError;
use layer0::intent::Scope;
use layer0::state::StateReader;

/// Configuration for [`ContextAssembler`].
#[derive(Debug, Clone)]
pub struct ContextAssemblyConfig {
    /// Maximum number of recent deltas to include. Default: 5.
    pub max_deltas: usize,
    /// Number of FTS results to fetch. Default: 50.
    pub fetch_k: usize,
    /// Half-life in days for exponential recency decay. Default: 7.0.
    pub recency_half_life_days: f64,
}

impl Default for ContextAssemblyConfig {
    fn default() -> Self {
        Self {
            max_deltas: 5,
            fetch_k: 50,
            recency_half_life_days: 7.0,
        }
    }
}

/// Assembles context packages for sweep agents from state store data.
///
/// Reads decision cards, recent deltas, and FTS search hits from a
/// [`StateReader`], wrapping each as a [`Message`] with
/// appropriate salience scores and compaction policies.
///
/// # Example
///
/// ```no_run
/// use skg_context::context_assembly::{ContextAssembler, ContextAssemblyConfig};
///
/// let assembler = ContextAssembler::new(ContextAssemblyConfig::default());
/// ```
#[derive(Debug)]
pub struct ContextAssembler {
    config: ContextAssemblyConfig,
}

impl ContextAssembler {
    /// Create a new assembler with the given configuration.
    pub fn new(config: ContextAssemblyConfig) -> Self {
        Self { config }
    }

    /// Assemble the context package for a decision.
    ///
    /// Returns a `Vec<Message>` containing:
    /// - System instructions (Pinned, if `system_prompt` is `Some`)
    /// - Decision card (Pinned, salience = 1.0)
    /// - Recent deltas (Normal, recency-scored salience)
    /// - FTS search hits (Normal, BM25-normalized salience)
    ///
    /// The returned messages are NOT yet budget-enforced. Pass them through
    /// [`salience_packing_compactor`](crate::salience_packing_compactor)
    /// to enforce token limits.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] if store reads fail.
    pub async fn assemble(
        &self,
        store: &dyn StateReader,
        scope: &Scope,
        decision_id: &str,
        system_prompt: Option<&str>,
    ) -> Result<Vec<Message>, StateError> {
        let now_us = now_micros();
        let mut messages = Vec::new();

        // Step 1: System instructions (Pinned)
        if let Some(prompt) = system_prompt {
            messages.push(Message::pinned(Role::User, Content::text(prompt)));
        }

        // Step 2: Decision card (Pinned, salience = 1.0)
        let card_key = format!("card:{decision_id}");
        if let Some(card_value) = store.read(scope, &card_key).await? {
            let mut card_msg =
                Message::pinned(Role::User, Content::text(value_to_text(&card_value)));
            card_msg.meta.salience = Some(1.0);
            messages.push(card_msg);
        }

        // Step 3: Recent deltas (Normal, recency-scored)
        let delta_prefix = format!("delta:{decision_id}:");
        let delta_keys = store.list(scope, &delta_prefix).await?;

        let mut delta_entries: Vec<(i64, String)> = delta_keys
            .into_iter()
            .filter_map(|key| {
                let ts = parse_delta_timestamp(&key, decision_id)?;
                Some((ts, key))
            })
            .collect();
        // Most recent first.
        delta_entries.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        delta_entries.truncate(self.config.max_deltas);

        for (ts, key) in &delta_entries {
            if let Some(value) = store.read(scope, key).await? {
                let salience = recency_score(*ts, now_us, self.config.recency_half_life_days);
                let mut msg = Message::new(Role::User, Content::text(value_to_text(&value)));
                msg.meta = MessageMeta::with_policy(CompactionPolicy::Normal)
                    .set_source("sweep:delta")
                    .set_salience(salience);
                messages.push(msg);
            }
        }

        // Step 4: FTS search hits (Normal, BM25-normalized salience)
        let results = store
            .search(scope, decision_id, self.config.fetch_k)
            .await?;

        if !results.is_empty() {
            let scores: Vec<f64> = results.iter().map(|r| r.score).collect();
            let normalized = normalize_bm25_scores(&scores);

            for (i, result) in results.iter().enumerate() {
                // Skip entries we already included as card or delta.
                if result.key == card_key || result.key.starts_with(&delta_prefix) {
                    continue;
                }

                let text = match &result.snippet {
                    Some(snippet) if !snippet.is_empty() => snippet.clone(),
                    _ => {
                        // No snippet — read the full value.
                        match store.read(scope, &result.key).await? {
                            Some(val) => value_to_text(&val),
                            None => continue, // deleted between search and read
                        }
                    }
                };

                let mut msg = Message::new(Role::User, Content::text(text));
                msg.meta = MessageMeta::with_policy(CompactionPolicy::Normal)
                    .set_source("sweep:fts")
                    .set_salience(normalized[i]);
                messages.push(msg);
            }
        }

        Ok(messages)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a JSON value to display text.
///
/// Strings are returned directly. Everything else is pretty-printed JSON.
fn value_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// Parse a timestamp from a delta key `delta:{decision_id}:{timestamp_micros}`.
fn parse_delta_timestamp(key: &str, decision_id: &str) -> Option<i64> {
    let prefix = format!("delta:{decision_id}:");
    let rest = key.strip_prefix(&prefix)?;
    rest.parse::<i64>().ok()
}

/// Exponential recency decay.
///
/// Returns 0.0–1.0 where 1.0 = just created.
/// Uses half-life decay: `score = exp(-ln(2) / half_life * age)`.
pub(crate) fn recency_score(timestamp_micros: i64, now_micros: i64, half_life_days: f64) -> f64 {
    if half_life_days <= 0.0 {
        return 0.5;
    }
    let age_micros = (now_micros - timestamp_micros).max(0) as f64;
    let age_days = age_micros / (86_400.0 * 1_000_000.0);
    let lambda = 2.0_f64.ln() / half_life_days;
    (-lambda * age_days).exp()
}

/// Min-max normalize BM25 scores to 0.0–1.0.
///
/// If all scores are equal, returns 0.5 for all entries.
pub(crate) fn normalize_bm25_scores(scores: &[f64]) -> Vec<f64> {
    if scores.is_empty() {
        return vec![];
    }
    let min = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON {
        return vec![0.5; scores.len()];
    }
    scores.iter().map(|&s| (s - min) / (max - min)).collect()
}

/// Current Unix epoch in microseconds.
fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Unit tests for pure helpers (no async, no store)
    // -----------------------------------------------------------------------

    #[test]
    fn recency_score_at_zero_age() {
        let now = 1_000_000_000_000i64; // arbitrary
        let score = recency_score(now, now, 7.0);
        assert!((score - 1.0).abs() < 1e-10, "age=0 should give score=1.0");
    }

    #[test]
    fn recency_score_at_one_half_life() {
        let half_life = 7.0;
        let one_hl_micros = (half_life * 86_400.0 * 1_000_000.0) as i64;
        let now = 2_000_000_000_000i64;
        let ts = now - one_hl_micros;
        let score = recency_score(ts, now, half_life);
        assert!(
            (score - 0.5).abs() < 1e-6,
            "age=half_life should give score~0.5, got {score}"
        );
    }

    #[test]
    fn recency_score_at_two_half_lives() {
        let half_life = 7.0;
        let two_hl_micros = (2.0 * half_life * 86_400.0 * 1_000_000.0) as i64;
        let now = 2_000_000_000_000i64;
        let ts = now - two_hl_micros;
        let score = recency_score(ts, now, half_life);
        assert!(
            (score - 0.25).abs() < 1e-6,
            "age=2*half_life should give score~0.25, got {score}"
        );
    }

    #[test]
    fn recency_score_future_timestamp_clamps() {
        let now = 1_000_000_000_000i64;
        let future = now + 1_000_000;
        let score = recency_score(future, now, 7.0);
        assert!(
            (score - 1.0).abs() < 1e-10,
            "future timestamps should clamp to 1.0"
        );
    }

    #[test]
    fn recency_score_zero_half_life() {
        let score = recency_score(0, 1_000_000, 0.0);
        assert!(
            (score - 0.5).abs() < 1e-10,
            "zero half_life should return 0.5"
        );
    }

    #[test]
    fn normalize_bm25_basic() {
        let scores = vec![1.0, 5.0, 3.0];
        let norm = normalize_bm25_scores(&scores);
        assert!((norm[0] - 0.0).abs() < 1e-10); // min -> 0.0
        assert!((norm[1] - 1.0).abs() < 1e-10); // max -> 1.0
        assert!((norm[2] - 0.5).abs() < 1e-10); // mid -> 0.5
    }

    #[test]
    fn normalize_bm25_single() {
        let norm = normalize_bm25_scores(&[42.0]);
        assert_eq!(norm.len(), 1);
        assert!((norm[0] - 0.5).abs() < 1e-10, "single score -> 0.5");
    }

    #[test]
    fn normalize_bm25_all_equal() {
        let norm = normalize_bm25_scores(&[3.0, 3.0, 3.0]);
        assert!(norm.iter().all(|&s| (s - 0.5).abs() < 1e-10));
    }

    #[test]
    fn normalize_bm25_empty() {
        assert!(normalize_bm25_scores(&[]).is_empty());
    }

    #[test]
    fn value_to_text_string() {
        let v = serde_json::Value::String("hello world".into());
        assert_eq!(value_to_text(&v), "hello world");
    }

    #[test]
    fn value_to_text_object() {
        let v = serde_json::json!({"key": "val"});
        let text = value_to_text(&v);
        assert!(text.contains("key"));
        assert!(text.contains("val"));
    }

    #[test]
    fn value_to_text_number() {
        let v = serde_json::json!(42);
        assert_eq!(value_to_text(&v), "42");
    }

    #[test]
    fn parse_delta_timestamp_valid() {
        let ts = parse_delta_timestamp("delta:topic-3b:1709500000000000", "topic-3b");
        assert_eq!(ts, Some(1_709_500_000_000_000));
    }

    #[test]
    fn parse_delta_timestamp_wrong_prefix() {
        assert!(parse_delta_timestamp("card:topic-3b", "topic-3b").is_none());
    }

    #[test]
    fn parse_delta_timestamp_wrong_decision() {
        assert!(parse_delta_timestamp("delta:topic-2a:123", "topic-3b").is_none());
    }

    #[test]
    fn parse_delta_timestamp_non_numeric() {
        assert!(parse_delta_timestamp("delta:topic-3b:abc", "topic-3b").is_none());
    }
}
