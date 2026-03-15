//! Budget guard rule — halts execution when limits are exceeded.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::ContextOp;
use async_trait::async_trait;
use rust_decimal::Decimal;
use std::time::Duration;

/// Configuration for the budget guard.
#[derive(Debug, Clone)]
pub struct BudgetGuardConfig {
    /// Maximum cost in USD. `None` = no limit.
    pub max_cost: Option<Decimal>,
    /// Maximum inference turns. `None` = no limit.
    pub max_turns: Option<u32>,
    /// Maximum wall-clock duration. `None` = no limit.
    pub max_duration: Option<Duration>,
    /// Maximum total tool calls. `None` = no limit.
    pub max_tool_calls: Option<u32>,
}

impl Default for BudgetGuardConfig {
    fn default() -> Self {
        Self {
            max_cost: None,
            max_turns: Some(10),
            max_duration: None,
            max_tool_calls: None,
        }
    }
}

/// Budget guard — halts execution when any configured limit is exceeded.
///
/// Use with `Rule::before::<InferBoundary>` (or `StreamInferBoundary`) to
/// fire before each inference call. The `react_loop` fires boundary rules
/// around provider calls, so `Before<InferBoundary>` is the canonical trigger.
///
/// When the guard fires and a limit is exceeded, it returns
/// `EngineError::Halted` which stops the pipeline.
///
// TODO: `skelegent/skelegent/src/agent.rs` still uses `Trigger::BeforeAny`
// for BudgetGuard — update to `Rule::before::<InferBoundary>` now that
// react_loop fires boundary rules.
pub struct BudgetGuard {
    /// Budget configuration.
    pub config: BudgetGuardConfig,
}

impl BudgetGuard {
    /// Create with default config (max 10 turns).
    pub fn new() -> Self {
        Self {
            config: BudgetGuardConfig::default(),
        }
    }

    /// Create with custom config.
    pub fn with_config(config: BudgetGuardConfig) -> Self {
        Self { config }
    }
}

impl Default for BudgetGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextOp for BudgetGuard {
    type Output = ();

    async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        if let Some(max_cost) = self.config.max_cost
            && ctx.metrics.cost > max_cost
        {
            tracing::warn!(
                budget_event = "cost_exceeded",
                current_cost = %ctx.metrics.cost,
                max_cost = %max_cost,
                "budget guard: cost limit exceeded"
            );
            return Err(EngineError::Halted {
                reason: format!("cost budget exceeded: {} > {}", ctx.metrics.cost, max_cost),
            });
        }

        if let Some(max_turns) = self.config.max_turns
            && ctx.metrics.turns_completed >= max_turns
        {
            tracing::warn!(
                budget_event = "turn_limit_reached",
                current_turns = ctx.metrics.turns_completed,
                max_turns = max_turns,
                "budget guard: turn limit exceeded"
            );
            return Err(EngineError::Halted {
                reason: format!(
                    "turn limit exceeded: {} >= {}",
                    ctx.metrics.turns_completed, max_turns
                ),
            });
        }

        if let Some(max_duration) = self.config.max_duration
            && ctx.metrics.start.elapsed() > max_duration
        {
            tracing::warn!(
                budget_event = "timeout_reached",
                elapsed_ms = ctx.metrics.start.elapsed().as_millis() as u64,
                max_duration_ms = max_duration.as_millis() as u64,
                "budget guard: duration exceeded"
            );
            return Err(EngineError::Halted {
                reason: format!(
                    "duration exceeded: {:?} > {:?}",
                    ctx.metrics.start.elapsed(),
                    max_duration
                ),
            });
        }

        if let Some(max_tool_calls) = self.config.max_tool_calls
            && ctx.metrics.tool_calls_total >= max_tool_calls
        {
            tracing::warn!(
                budget_event = "tool_limit_reached",
                current_tool_calls = ctx.metrics.tool_calls_total,
                max_tool_calls = max_tool_calls,
                "budget guard: tool call limit exceeded"
            );
            return Err(EngineError::Halted {
                reason: format!(
                    "tool call limit exceeded: {} >= {}",
                    ctx.metrics.tool_calls_total, max_tool_calls
                ),
            });
        }

        // Approach warnings — alert when >80% of any budget is consumed.
        if let Some(max_cost) = self.config.max_cost {
            let ratio = ctx.metrics.cost / max_cost;
            if ratio > Decimal::new(80, 2) {
                tracing::info!(
                    budget_event = "budget_warning",
                    cost_ratio = %ratio,
                    "budget guard: approaching cost limit"
                );
            }
        }
        if let Some(max_turns) = self.config.max_turns {
            let used = ctx.metrics.turns_completed as f64 / max_turns as f64;
            if used > 0.8 {
                tracing::info!(
                    budget_event = "budget_warning",
                    turn_ratio = used,
                    "budget guard: approaching turn limit"
                );
            }
        }
        if let Some(max_tool_calls) = self.config.max_tool_calls {
            let used = ctx.metrics.tool_calls_total as f64 / max_tool_calls as f64;
            if used > 0.8 {
                tracing::info!(
                    budget_event = "budget_warning",
                    tool_call_ratio = used,
                    "budget guard: approaching tool call limit"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn budget_guard_halts_on_turn_limit() {
        let mut ctx = Context::new();
        ctx.metrics.turns_completed = 10;

        let guard = BudgetGuard::new(); // default: max 10 turns
        let result = guard.execute(&mut ctx).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, EngineError::Halted { .. }));
    }

    #[tokio::test]
    async fn budget_guard_passes_under_limit() {
        let mut ctx = Context::new();
        ctx.metrics.turns_completed = 5;

        let guard = BudgetGuard::new();
        let result = guard.execute(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn budget_guard_cost_limit() {
        let mut ctx = Context::new();
        ctx.metrics.cost = Decimal::new(150, 2); // $1.50

        let guard = BudgetGuard::with_config(BudgetGuardConfig {
            max_cost: Some(Decimal::new(100, 2)), // $1.00
            max_turns: None,
            max_duration: None,
            max_tool_calls: None,
        });

        let result = guard.execute(&mut ctx).await;
        assert!(result.is_err());
    }
}
