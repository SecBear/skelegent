//! Model-specific pricing for Anthropic's API.
//!
//! Prices are per-token in USD. Anthropic publishes per-MTok prices; we store them
//! divided by 1_000_000.
//!
//! Cache pricing follows Anthropic's standard rates:
//! - Cache read: 10% of the model's base input price
//! - Cache write: 125% of the model's base input price

use rust_decimal::Decimal;
use skg_turn::types::TokenUsage;

/// Per-token cost breakdown for an Anthropic model family.
pub(crate) struct ModelPricing {
    input_per_token: Decimal,
    output_per_token: Decimal,
    cache_read_per_token: Option<Decimal>,
    cache_write_per_token: Option<Decimal>,
}

/// Resolve pricing for the given model name.
///
/// Matching is prefix-based so versioned names like `claude-sonnet-4-20250514`
/// map correctly. Unknown models default to Sonnet-class pricing (middle ground).
pub(crate) fn lookup(model: &str) -> ModelPricing {
    // Check prefixes from most-specific to least-specific so that a future
    // "claude-opus-4-haiku" style name would not mis-match.
    if model.starts_with("claude-opus-4") || model.starts_with("claude-3-opus") {
        opus()
    } else if model.starts_with("claude-haiku-4") || model.starts_with("claude-3-5-haiku") {
        haiku()
    } else if model.starts_with("claude-sonnet-4") || model.starts_with("claude-3-5-sonnet") {
        sonnet()
    } else {
        // Default to Sonnet-class: most commonly deployed, middle-ground price.
        // Avoids under-reporting cost for expensive models at the risk of slightly
        // over-reporting for cheaper ones.
        sonnet()
    }
}

/// Compute total cost for a completed inference given per-token pricing and
/// observed token usage.
///
/// All four token types are factored in: input, output, cache-read, and
/// cache-write (prompt caching creation). Token counts without a corresponding
/// price entry (or `None` counts) contribute zero.
pub(crate) fn compute_cost(pricing: &ModelPricing, usage: &TokenUsage) -> Decimal {
    let input_cost = Decimal::from(usage.input_tokens) * pricing.input_per_token;
    let output_cost = Decimal::from(usage.output_tokens) * pricing.output_per_token;

    let cache_read_cost = usage
        .cache_read_tokens
        .zip(pricing.cache_read_per_token)
        .map(|(tokens, price)| Decimal::from(tokens) * price)
        .unwrap_or(Decimal::ZERO);

    let cache_write_cost = usage
        .cache_creation_tokens
        .zip(pricing.cache_write_per_token)
        .map(|(tokens, price)| Decimal::from(tokens) * price)
        .unwrap_or(Decimal::ZERO);

    input_cost + output_cost + cache_read_cost + cache_write_cost
}

// ── Model families ────────────────────────────────────────────────────────────

/// Sonnet-class: $3.00 input / $15.00 output per MTok
fn sonnet() -> ModelPricing {
    ModelPricing {
        input_per_token: Decimal::new(3, 6),    // 3.00e-6  = $3.00/MTok
        output_per_token: Decimal::new(15, 6),  // 15.0e-6  = $15.00/MTok
        cache_read_per_token: Some(Decimal::new(3, 7)),   // 0.30e-6  = $0.30/MTok (10%)
        cache_write_per_token: Some(Decimal::new(375, 8)), // 3.75e-6  = $3.75/MTok (125%)
    }
}

/// Haiku-class: $0.25 input / $1.25 output per MTok
fn haiku() -> ModelPricing {
    ModelPricing {
        input_per_token: Decimal::new(25, 8),    // 25e-8   = $0.25/MTok
        output_per_token: Decimal::new(125, 8),  // 125e-8  = $1.25/MTok
        cache_read_per_token: Some(Decimal::new(25, 9)),    // 25e-9   = $0.025/MTok (10%)
        cache_write_per_token: Some(Decimal::new(3125, 10)), // 3125e-10 = $0.3125/MTok (125%)
    }
}

/// Opus-class: $15.00 input / $75.00 output per MTok
fn opus() -> ModelPricing {
    ModelPricing {
        input_per_token: Decimal::new(15, 6),    // 15e-6   = $15.00/MTok
        output_per_token: Decimal::new(75, 6),   // 75e-6   = $75.00/MTok
        cache_read_per_token: Some(Decimal::new(15, 7)),    // 15e-7   = $1.50/MTok  (10%)
        cache_write_per_token: Some(Decimal::new(1875, 8)), // 1875e-8 = $18.75/MTok (125%)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: compute cost for N input tokens, M output, no cache.
    fn cost_simple(model: &str, input: u64, output: u64) -> Decimal {
        let usage = TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
        };
        compute_cost(&lookup(model), &usage)
    }

    #[test]
    fn haiku_1m_tokens_matches_published_price() {
        // 1M input + 1M output at $0.25/$1.25 per MTok → $1.50 total
        let cost = cost_simple("claude-haiku-4-5-20251001", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(150, 2), "haiku 1M+1M = $1.50");
    }

    #[test]
    fn sonnet_1m_tokens_matches_published_price() {
        // 1M input + 1M output at $3.00/$15.00 per MTok → $18.00 total
        let cost = cost_simple("claude-sonnet-4-20250514", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(1800, 2), "sonnet 1M+1M = $18.00");
    }

    #[test]
    fn opus_1m_tokens_matches_published_price() {
        // 1M input + 1M output at $15.00/$75.00 per MTok → $90.00 total
        let cost = cost_simple("claude-opus-4-20250514", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(9000, 2), "opus 1M+1M = $90.00");
    }

    #[test]
    fn legacy_sonnet_prefix_resolves() {
        // claude-3-5-sonnet should also hit Sonnet pricing
        let cost = cost_simple("claude-3-5-sonnet-20241022", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(1800, 2));
    }

    #[test]
    fn legacy_haiku_prefix_resolves() {
        let cost = cost_simple("claude-3-5-haiku-20241022", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(150, 2));
    }

    #[test]
    fn legacy_opus_prefix_resolves() {
        let cost = cost_simple("claude-3-opus-20240229", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(9000, 2));
    }

    #[test]
    fn unknown_model_defaults_to_sonnet_pricing() {
        // Unknown model → Sonnet-class default
        let cost = cost_simple("claude-unknown-future-model", 1_000_000, 1_000_000);
        assert_eq!(cost, Decimal::new(1800, 2), "unknown model should use Sonnet pricing");
    }

    #[test]
    fn cache_tokens_factored_into_cost() {
        // Sonnet: 1M cache-read + 1M cache-write on top of 0 normal tokens
        // cache_read = $0.30/MTok → $0.30; cache_write = $3.75/MTok → $3.75
        let usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: Some(1_000_000),
            cache_creation_tokens: Some(1_000_000),
            reasoning_tokens: None,
        };
        let cost = compute_cost(&lookup("claude-sonnet-4-20250514"), &usage);
        // $0.30 + $3.75 = $4.05
        assert_eq!(cost, Decimal::new(405, 2), "cache costs = $4.05");
    }

    #[test]
    fn no_cache_tokens_contributes_zero() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 100,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
        };
        let with_cache = TokenUsage {
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            ..usage.clone()
        };
        let p = lookup("claude-sonnet-4");
        assert_eq!(compute_cost(&p, &usage), compute_cost(&lookup("claude-sonnet-4"), &with_cache));
    }
}
