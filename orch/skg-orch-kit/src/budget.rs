use rust_decimal::Decimal;

/// Snapshot of orchestration-local spend against a hard budget limit.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetSnapshot {
    spent: Decimal,
    hard_limit: Decimal,
}

impl BudgetSnapshot {
    /// Create a new budget snapshot.
    pub fn new(spent: Decimal, hard_limit: Decimal) -> Self {
        Self { spent, hard_limit }
    }

    /// Spend already recorded against this budget.
    pub fn spent(&self) -> Decimal {
        self.spent
    }

    /// Hard upper limit for this budget.
    pub fn hard_limit(&self) -> Decimal {
        self.hard_limit
    }

    /// Remaining headroom before the hard limit is exhausted.
    pub fn remaining(&self) -> Decimal {
        (self.hard_limit - self.spent).max(Decimal::ZERO)
    }

    /// Decide whether additional spend can be admitted.
    pub fn decide(&self, additional_spend: Decimal) -> BudgetDecision {
        let remaining = self.remaining();

        if additional_spend <= remaining {
            BudgetDecision::Allow {
                remaining_after: remaining - additional_spend,
            }
        } else {
            BudgetDecision::Deny {
                deficit: additional_spend - remaining,
            }
        }
    }
}

/// Result of checking a hard budget limit.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetDecision {
    /// Additional work fits under the hard limit.
    Allow {
        /// Remaining headroom after admitting the work.
        remaining_after: Decimal,
    },
    /// Additional work would exhaust or exceed the hard limit.
    Deny {
        /// How far the proposed work would push the budget beyond zero headroom.
        deficit: Decimal,
    },
}

#[cfg(test)]
mod tests {
    use super::{BudgetDecision, BudgetSnapshot};
    use rust_decimal::Decimal;

    #[test]
    fn snapshot_accessors_return_recorded_values() {
        let snapshot = BudgetSnapshot::new(Decimal::new(325, 2), Decimal::new(500, 2));

        assert_eq!(snapshot.spent(), Decimal::new(325, 2));
        assert_eq!(snapshot.hard_limit(), Decimal::new(500, 2));
    }

    #[test]
    fn remaining_clamps_at_zero_when_spend_exceeds_limit() {
        let snapshot = BudgetSnapshot::new(Decimal::new(130, 2), Decimal::new(100, 2));

        assert_eq!(snapshot.remaining(), Decimal::ZERO);
    }

    #[test]
    fn decide_allows_work_that_fits_under_limit() {
        let snapshot = BudgetSnapshot::new(Decimal::new(325, 2), Decimal::new(500, 2));

        assert_eq!(
            snapshot.decide(Decimal::new(125, 2)),
            BudgetDecision::Allow {
                remaining_after: Decimal::new(50, 2),
            }
        );
    }

    #[test]
    fn decide_allows_work_that_exactly_consumes_remaining_budget() {
        let snapshot = BudgetSnapshot::new(Decimal::new(450, 2), Decimal::new(500, 2));

        assert_eq!(
            snapshot.decide(Decimal::new(50, 2)),
            BudgetDecision::Allow {
                remaining_after: Decimal::ZERO,
            }
        );
    }

    #[test]
    fn decide_denies_work_that_would_cross_limit() {
        let snapshot = BudgetSnapshot::new(Decimal::new(475, 2), Decimal::new(500, 2));

        assert_eq!(
            snapshot.decide(Decimal::new(50, 2)),
            BudgetDecision::Deny {
                deficit: Decimal::new(25, 2),
            }
        );
    }

    #[test]
    fn decide_denies_when_no_headroom_remains() {
        let snapshot = BudgetSnapshot::new(Decimal::new(500, 2), Decimal::new(500, 2));

        assert_eq!(
            snapshot.decide(Decimal::new(1, 2)),
            BudgetDecision::Deny {
                deficit: Decimal::new(1, 2),
            }
        );
    }
}
