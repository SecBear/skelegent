//! Opt-in telemetry rule — zero-config observability via `tracing`.
//!
//! Add `TelemetryRule::default()` to your context for full tracing:
//! ```ignore
//! ctx.add_rule(TelemetryRule::default().into_rule());
//! ```
//!
//! Fires `AfterAny` to observe all context operations. Emits tracing spans
//! with metrics, tool names, message counts, and cost tracking.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use crate::rule::{Rule, Trigger};
use async_trait::async_trait;

/// What level of detail to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryVerbosity {
    /// Metrics only: tokens, cost, turns, tool calls.
    Metrics,
    /// Metrics + operation names.
    #[default]
    Operations,
    /// Everything: metrics, operations, message counts, effect counts.
    Full,
}

/// Configuration for the telemetry rule.
#[derive(Debug, Clone, Default)]
pub struct TelemetryConfig {
    /// What level of detail to emit.
    pub verbosity: TelemetryVerbosity,
    /// Tracing level for normal operations.
    pub level: TelemetryLevel,
}

/// Maps to tracing levels without exposing tracing types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryLevel {
    /// `tracing::Level::TRACE`
    Trace,
    /// `tracing::Level::DEBUG`
    Debug,
    /// `tracing::Level::INFO`
    #[default]
    Info,
}

/// Opt-in telemetry rule. Fires after every context operation to emit
/// tracing events with accumulated metrics.
///
/// # Usage
///
/// ```ignore
/// use neuron_context_engine::{Context, rules::TelemetryRule};
///
/// let mut ctx = Context::new();
/// ctx.add_rule(TelemetryRule::default().into_rule());
/// // All context operations now emit tracing events.
/// ```
///
/// Opt-out: simply don't add the rule, or build a context without it.
pub struct TelemetryRule {
    /// Configuration.
    pub config: TelemetryConfig,
    /// Snapshot of metrics at last emission, for delta calculation.
    last_tokens_in: u64,
    last_tokens_out: u64,
    last_tool_calls: u32,
}

impl TelemetryRule {
    /// Create with default config (Operations verbosity, Info level).
    pub fn new() -> Self {
        Self {
            config: TelemetryConfig::default(),
            last_tokens_in: 0,
            last_tokens_out: 0,
            last_tool_calls: 0,
        }
    }

    /// Create with custom config.
    pub fn with_config(config: TelemetryConfig) -> Self {
        Self {
            config,
            last_tokens_in: 0,
            last_tokens_out: 0,
            last_tool_calls: 0,
        }
    }

    /// Convert this into a [`Rule`] that fires after every operation.
    ///
    /// Default priority is 0 (lowest). Telemetry should observe, not
    /// interfere with higher-priority rules like budget guards.
    pub fn into_rule(self) -> Rule {
        Rule::new("telemetry", Trigger::AfterAny, 0, self)
    }

    /// Convert into a rule with custom priority.
    pub fn into_rule_with_priority(self, priority: i32) -> Rule {
        Rule::new("telemetry", Trigger::AfterAny, priority, self)
    }
}

impl Default for TelemetryRule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextOp for TelemetryRule {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        let m = &ctx.metrics;

        // Calculate deltas since last emission
        let delta_in = m.tokens_in.saturating_sub(self.last_tokens_in);
        let delta_out = m.tokens_out.saturating_sub(self.last_tokens_out);
        let delta_tools = m.tool_calls_total.saturating_sub(self.last_tool_calls);

        match self.config.level {
            TelemetryLevel::Trace => match self.config.verbosity {
                TelemetryVerbosity::Metrics => {
                    tracing::trace!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.metrics"
                    );
                }
                TelemetryVerbosity::Operations => {
                    tracing::trace!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.op"
                    );
                }
                TelemetryVerbosity::Full => {
                    tracing::trace!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        message_count = ctx.messages.len(),
                        effect_count = ctx.effects.len(),
                        rule_count = ctx.rule_count(),
                        "neuron.full"
                    );
                }
            },
            TelemetryLevel::Debug => match self.config.verbosity {
                TelemetryVerbosity::Metrics => {
                    tracing::debug!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.metrics"
                    );
                }
                TelemetryVerbosity::Operations => {
                    tracing::debug!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.op"
                    );
                }
                TelemetryVerbosity::Full => {
                    tracing::debug!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        message_count = ctx.messages.len(),
                        effect_count = ctx.effects.len(),
                        rule_count = ctx.rule_count(),
                        "neuron.full"
                    );
                }
            },
            TelemetryLevel::Info => match self.config.verbosity {
                TelemetryVerbosity::Metrics => {
                    tracing::info!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.metrics"
                    );
                }
                TelemetryVerbosity::Operations => {
                    tracing::info!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        "neuron.op"
                    );
                }
                TelemetryVerbosity::Full => {
                    tracing::info!(
                        tokens_in = m.tokens_in,
                        tokens_out = m.tokens_out,
                        delta_tokens_in = delta_in,
                        delta_tokens_out = delta_out,
                        cost = %m.cost,
                        turns = m.turns_completed,
                        tool_calls = m.tool_calls_total,
                        delta_tool_calls = delta_tools,
                        elapsed_ms = m.elapsed_ms(),
                        message_count = ctx.messages.len(),
                        effect_count = ctx.effects.len(),
                        rule_count = ctx.rule_count(),
                        "neuron.full"
                    );
                }
            },
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_rule_default_config() {
        let rule = TelemetryRule::default();
        assert_eq!(rule.config.verbosity, TelemetryVerbosity::Operations);
        assert_eq!(rule.config.level, TelemetryLevel::Info);
    }

    #[test]
    fn telemetry_rule_custom_config() {
        let rule = TelemetryRule::with_config(TelemetryConfig {
            verbosity: TelemetryVerbosity::Full,
            level: TelemetryLevel::Debug,
        });
        assert_eq!(rule.config.verbosity, TelemetryVerbosity::Full);
        assert_eq!(rule.config.level, TelemetryLevel::Debug);
    }

    #[test]
    fn into_rule_produces_after_any() {
        let rule = TelemetryRule::default().into_rule();
        assert_eq!(rule.name, "telemetry");
        assert_eq!(rule.priority, 0);
    }

    #[test]
    fn into_rule_with_priority() {
        let rule = TelemetryRule::default().into_rule_with_priority(-10);
        assert_eq!(rule.priority, -10);
    }

    #[tokio::test]
    async fn telemetry_fires_without_error() {
        let mut ctx = Context::new();
        ctx.metrics.tokens_in = 100;
        ctx.metrics.tokens_out = 50;
        ctx.metrics.turns_completed = 2;
        ctx.metrics.tool_calls_total = 3;

        let telemetry = TelemetryRule::default();
        let result = telemetry.execute(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn telemetry_full_verbosity() {
        let mut ctx = Context::new();
        ctx.metrics.tokens_in = 200;

        let telemetry = TelemetryRule::with_config(TelemetryConfig {
            verbosity: TelemetryVerbosity::Full,
            level: TelemetryLevel::Trace,
        });
        let result = telemetry.execute(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn telemetry_metrics_only() {
        let mut ctx = Context::new();
        let telemetry = TelemetryRule::with_config(TelemetryConfig {
            verbosity: TelemetryVerbosity::Metrics,
            level: TelemetryLevel::Debug,
        });
        let result = telemetry.execute(&mut ctx).await;
        assert!(result.is_ok());
    }
}
