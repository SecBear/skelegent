//! Reactive rules — [`ContextOp`]s with triggers.
//!
//! Rules fire automatically during [`Context::run()`]. They have the same
//! power as pipeline operations (`&mut Context`), just different activation.

use crate::context::Context;
use crate::error::EngineError;
use crate::op::{ContextOp, ErasedOp};
use std::any::TypeId;

/// When a rule should fire.
pub enum Trigger {
    /// Fire before every `run()` call.
    BeforeAny,
    /// Fire after every `run()` call.
    AfterAny,
    /// Fire before a specific operation type.
    Before(TypeId),
    /// Fire after a specific operation type.
    After(TypeId),
    /// Fire when a predicate on the context is true.
    /// Evaluated at the start of every `run()` call.
    When(Box<dyn Fn(&Context) -> bool + Send + Sync>),
}

/// A reactive participant. Combines a [`ContextOp`] with a [`Trigger`].
///
/// Rules fire in priority order (highest first). Rules cannot trigger
/// other rules — the dispatch loop skips rule evaluation during rule
/// execution to prevent infinite recursion.
pub struct Rule {
    /// Human-readable name for debugging and tracing.
    pub name: String,
    /// When this rule fires.
    pub trigger: Trigger,
    /// Higher priority rules fire first.
    pub priority: i32,
    /// The operation to execute.
    pub(crate) op: Box<dyn ErasedOp>,
}

impl Rule {
    /// Create a new rule from a name, trigger, priority, and operation.
    pub fn new<O: ContextOp<Output = ()> + 'static>(
        name: impl Into<String>,
        trigger: Trigger,
        priority: i32,
        op: O,
    ) -> Self {
        Self {
            name: name.into(),
            trigger,
            priority,
            op: Box::new(op),
        }
    }

    /// Convenience: create a `Before(TypeId)` trigger for a specific op type.
    pub fn before<O: 'static>(
        name: impl Into<String>,
        priority: i32,
        op: impl ContextOp<Output = ()> + 'static,
    ) -> Self {
        Self::new(name, Trigger::Before(TypeId::of::<O>()), priority, op)
    }

    /// Convenience: create an `After(TypeId)` trigger for a specific op type.
    pub fn after<O: 'static>(
        name: impl Into<String>,
        priority: i32,
        op: impl ContextOp<Output = ()> + 'static,
    ) -> Self {
        Self::new(name, Trigger::After(TypeId::of::<O>()), priority, op)
    }

    /// Convenience: create a `When` trigger with a predicate.
    pub fn when(
        name: impl Into<String>,
        priority: i32,
        predicate: impl Fn(&Context) -> bool + Send + Sync + 'static,
        op: impl ContextOp<Output = ()> + 'static,
    ) -> Self {
        Self::new(name, Trigger::When(Box::new(predicate)), priority, op)
    }

    /// Check if this rule matches a "before" trigger for the given op type.
    pub(crate) fn matches_before(&self, op_type: TypeId) -> bool {
        matches!(&self.trigger, Trigger::BeforeAny)
            || matches!(&self.trigger, Trigger::Before(t) if *t == op_type)
    }

    /// Check if this rule matches an "after" trigger for the given op type.
    pub(crate) fn matches_after(&self, op_type: TypeId) -> bool {
        matches!(&self.trigger, Trigger::AfterAny)
            || matches!(&self.trigger, Trigger::After(t) if *t == op_type)
    }

    /// Check if this rule's When predicate matches.
    pub(crate) fn matches_when(&self, ctx: &Context) -> bool {
        match &self.trigger {
            Trigger::When(pred) => pred(ctx),
            _ => false,
        }
    }

    /// Execute this rule's operation.
    pub(crate) async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
        self.op.execute_erased(ctx).await
    }
}
