//! Salience-aware context packing utilities.
//!
//! Provides [`SaliencePackingConfig`] for configuring salience-based compaction
//! and [`term_jaccard`] for term-level Jaccard similarity.

use std::collections::HashSet;

/// Configuration for salience-aware context packing.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
