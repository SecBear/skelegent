//! skg-turn-kit
//!
//! Planning and execution primitives for a single agent turn — focused on
//! sequencing and concurrency only. These traits are intentionally narrow and
//! do not bake in provider streaming, hooks, or operator concerns; those live
//! at higher layers (e.g., operator implementations).
//!
//! Contents:
//! - Concurrency and ConcurrencyDecider — classify tool calls as Shared vs Exclusive
//! - DispatchPlanner and BarrierPlanner — sequence tool calls into batches
//! - SteeringSource — optional source of mid-loop steering messages
//! - BatchExecutor — run batches, with a simple sequential baseline executor
//!
//! Example: planning with a barrier.
//! ```rust
//! use skg_turn_kit::{BarrierPlanner, Concurrency, ConcurrencyDecider, DispatchPlanner};
//!
//! struct SharedIfStartsWith;
//! impl ConcurrencyDecider for SharedIfStartsWith {
//!     fn concurrency(&self, tool_name: &str) -> Concurrency {
//!         if tool_name.starts_with("shared_") { Concurrency::Shared } else { Concurrency::Exclusive }
//!     }
//! }
//!
//! let calls = vec![
//!     ("1".to_string(), "shared_a".to_string(), serde_json::json!({})),
//!     ("2".to_string(), "exclusive".to_string(), serde_json::json!({})),
//!     ("3".to_string(), "shared_b".to_string(), serde_json::json!({})),
//! ];
//! let planner = BarrierPlanner;
//! let plan = planner.plan(&calls, &SharedIfStartsWith);
//! assert!(matches!(plan[0], skg_turn_kit::BatchItem::Shared(_)));
//! ```

use layer0::context::Message;

#[cfg(feature = "typed-output")]
pub mod typed_output;
#[cfg(feature = "typed-output")]
pub use typed_output::{OutputValidator, RETURN_RESULT_TOOL, ToolSchemaEntry, TypedOutput};

use serde_json::Value;
use std::path::PathBuf;

/// Concurrency hint for tool scheduling (strategy-defined).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concurrency {
    /// Safe to run alongside other shared tools in the same batch.
    Shared,
    /// Must run alone (barrier before and after).
    Exclusive,
}

/// Decide concurrency for a tool by name.
pub trait ConcurrencyDecider: Send + Sync {
    /// Return the concurrency class for a tool by name.
    fn concurrency(&self, tool_name: &str) -> Concurrency;
}

/// Mid-session context manipulation command.
///
/// Delivered via [`SteeringSource`] and executed by the operator before
/// the next inference call. These bypass the LLM entirely.
#[derive(Debug, Clone)]
pub enum ContextCommand {
    /// Promote message at `index` to `CompactionPolicy::Pinned`.
    ///
    /// Pinned messages survive all compaction. Index is into the current buffer.
    Pin {
        /// Zero-based index into the message buffer.
        message_index: usize,
    },
    /// Drop the `count` oldest non-pinned messages.
    ///
    /// Pinned messages are never dropped.
    DropOldest {
        /// How many messages to drop.
        count: usize,
    },
    /// Serialize the current context to a JSON file.
    SaveSnapshot {
        /// Destination path.
        path: PathBuf,
    },
    /// Replace the current context by deserializing from a JSON file.
    LoadSnapshot {
        /// Source path.
        path: PathBuf,
    },
    /// Clear all non-pinned messages from the buffer.
    ///
    /// Equivalent to `DropOldest { count: usize::MAX }`.
    ClearWorking,
}

/// A command returned by [`SteeringSource::drain`].
///
/// Either injects a message into the context or executes a
/// [`ContextCommand`] to manipulate the message buffer directly.
#[derive(Debug, Clone)]
pub enum SteeringCommand {
    /// Inject a message into the context at the steering boundary.
    Message(Message),
    /// Execute a context manipulation command.
    Context(ContextCommand),
}

/// Optional source of steering messages to inject mid-loop (provider-formatted).
///
/// Boundary: this is not a "hook" — it does not inspect internal state or
/// events. It is a narrow bridge for external steering signals only.
pub trait SteeringSource: Send + Sync {
    /// Drain any pending steering commands.
    ///
    /// Returns a mix of [`SteeringCommand::Message`] (messages to inject) and
    /// [`SteeringCommand::Context`] (context manipulation commands).
    fn drain(&self) -> Vec<SteeringCommand>;
}

/// Planned batches for a turn.
#[derive(Debug, Clone)]
pub enum BatchItem {
    /// Batch of tools that may execute in parallel (shared).
    /// Each entry is (id, name, input_json).
    Shared(Vec<(String, String, Value)>),
    /// A single tool that must execute alone (exclusive).
    Exclusive((String, String, Value)),
}

/// Plan how to dispatch tool calls this turn (sequencing only).
pub trait DispatchPlanner: Send + Sync {
    /// Plan dispatch batches from an ordered list of tool calls. The planner
    /// must preserve relative order of application and introduce parallelism
    /// only for Shared batches. The decider classifies each tool.
    fn plan(
        &self,
        dispatch_requests: &[(String, String, Value)],
        decider: &dyn ConcurrencyDecider,
    ) -> Vec<BatchItem>;
}

/// Barrier planner: batches shared tools; flushes on exclusive.
pub struct BarrierPlanner;
impl DispatchPlanner for BarrierPlanner {
    fn plan(
        &self,
        dispatch_requests: &[(String, String, Value)],
        decider: &dyn ConcurrencyDecider,
    ) -> Vec<BatchItem> {
        let mut out = Vec::new();
        let mut pending_shared: Vec<(String, String, Value)> = Vec::new();
        for (id, name, input) in dispatch_requests.iter().cloned() {
            match decider.concurrency(&name) {
                Concurrency::Shared => pending_shared.push((id, name, input)),
                Concurrency::Exclusive => {
                    if !pending_shared.is_empty() {
                        out.push(BatchItem::Shared(std::mem::take(&mut pending_shared)));
                    }
                    out.push(BatchItem::Exclusive((id, name, input)));
                }
            }
        }
        if !pending_shared.is_empty() {
            out.push(BatchItem::Shared(pending_shared));
        }
        out
    }
}

/// Contract for executing planned batches.
///
/// Narrow by design: this concerns only execution of tool invocations with
/// concurrency semantics established by the planner. It does not include
/// operator concerns such as streaming, hooks, tracing, or budget control.
pub trait BatchExecutor: Send + Sync {
    /// Execute a full plan using the provided runner for individual tool calls.
    /// The runner receives (id, name, input_json) and returns an output value.
    fn exec_batches<F, O, E>(&self, plan: Vec<BatchItem>, f: F) -> Result<Vec<(String, O)>, E>
    where
        F: FnMut(String, String, Value) -> Result<O, E> + Send;
}

/// Baseline sequential executor: executes all tool calls in order, without
/// introducing any parallelism (Shared batches are executed one-by-one).
#[derive(Default, Debug, Clone, Copy)]
pub struct SequentialBatchExecutor;

impl BatchExecutor for SequentialBatchExecutor {
    fn exec_batches<F, O, E>(&self, plan: Vec<BatchItem>, mut f: F) -> Result<Vec<(String, O)>, E>
    where
        F: FnMut(String, String, Value) -> Result<O, E> + Send,
    {
        let mut outputs = Vec::new();
        for item in plan {
            match item {
                BatchItem::Exclusive((id, name, input)) => {
                    let out = f(id.clone(), name, input)?;
                    outputs.push((id, out));
                }
                BatchItem::Shared(batch) => {
                    for (id, name, input) in batch {
                        let out = f(id.clone(), name, input)?;
                        outputs.push((id, out));
                    }
                }
            }
        }
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Decider;
    impl ConcurrencyDecider for Decider {
        fn concurrency(&self, tool_name: &str) -> Concurrency {
            if tool_name.starts_with("s_") {
                Concurrency::Shared
            } else {
                Concurrency::Exclusive
            }
        }
    }

    #[test]
    fn plans_with_barrier() {
        let calls = vec![
            ("1".into(), "s_a".into(), serde_json::json!({})),
            ("2".into(), "x".into(), serde_json::json!({})),
            ("3".into(), "s_b".into(), serde_json::json!({})),
            ("4".into(), "s_c".into(), serde_json::json!({})),
        ];
        let plan = BarrierPlanner.plan(&calls, &Decider);
        assert!(matches!(plan[0], BatchItem::Shared(_)));
        assert!(matches!(plan[1], BatchItem::Exclusive(_)));
        assert!(matches!(plan[2], BatchItem::Shared(_)));
    }

    #[test]
    fn sequential_executor_executes_in_order() {
        let calls = vec![
            ("1".into(), "s_a".into(), serde_json::json!({})),
            ("2".into(), "x".into(), serde_json::json!({})),
        ];
        let plan = BarrierPlanner.plan(&calls, &Decider);
        let exec = SequentialBatchExecutor;
        let out = exec
            .exec_batches(plan, |id, name, _| Ok::<_, ()>((name.clone(), id.clone())))
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "1");
        assert_eq!(out[1].0, "2");
    }

    struct EmptySteering;
    impl SteeringSource for EmptySteering {
        fn drain(&self) -> Vec<super::SteeringCommand> {
            Vec::new()
        }
    }

    #[test]
    fn steering_source_compiles() {
        let s = EmptySteering;
        let drained = s.drain();
        assert!(drained.is_empty());
    }
}
