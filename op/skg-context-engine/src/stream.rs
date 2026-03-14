//! Streaming observation types.
//!
//! Every [`Context`](crate::Context) mutation is emitted as a [`ContextEvent`]
//! through a [`tokio::sync::broadcast`] channel. Observers subscribe to the
//! channel and receive real-time visibility into operator execution.
//!
//! # Zero-cost when unobserved
//!
//! If no observers have subscribed, [`broadcast::Sender::send`] checks the
//! receiver count and returns immediately. An operator with no observers pays
//! approximately nothing for the streaming infrastructure.
//!
//! # Lossy by design
//!
//! [`broadcast::channel`] has a fixed buffer. Slow subscribers lose old events
//! (`Lagged` error). This is correct for real-time observation — a slow
//! observer should see current state, not queue stale events. Durable
//! observation (audit logs) should write to `StateStore` via a dedicated
//! subscriber, not rely on the broadcast channel.

use layer0::context::Message;
use skg_turn::stream::StreamEvent;
use std::sync::Arc;
use std::time::Instant;

/// A single mutation to context state.
///
/// The vocabulary is tied to [`Context`](crate::Context)'s structural API
/// (messages, metrics), NOT to the open-ended set of
/// [`ContextOp`](crate::ContextOp)s. New ops compose existing mutations —
/// `InjectSystem` calls `ctx.push_message()`, `Compact` calls
/// `ctx.set_messages()`. The stream shows the actual changes regardless of
/// which op caused them.
#[derive(Debug, Clone)]
pub enum ContextMutation {
    /// A message was appended to the context.
    MessagePushed(Arc<Message>),

    /// A message was inserted at a specific index.
    MessageInserted {
        /// The position where the message was inserted.
        index: usize,
        /// The message.
        message: Arc<Message>,
    },

    /// A message was replaced at a specific index.
    MessageReplaced {
        /// The position of the replaced message.
        index: usize,
        /// The new message.
        message: Arc<Message>,
    },

    /// The entire message buffer was replaced (e.g. compaction).
    MessagesSet {
        /// Number of messages before replacement.
        previous_len: usize,
        /// Number of messages after replacement.
        new_len: usize,
    },


    /// Metrics were updated (tokens, cost).
    MetricsUpdated {
        /// Total input tokens after update.
        tokens_in: u64,
        /// Total output tokens after update.
        tokens_out: u64,
    },

    /// A streaming inference token/chunk arrived.
    InferenceDelta(StreamEvent),
}

/// A timestamped context mutation event.
///
/// Subscribers can reconstruct exact context state at any point by replaying
/// events from the start.
#[derive(Debug, Clone)]
pub struct ContextEvent {
    /// When the mutation occurred.
    pub timestamp: Instant,
    /// What changed.
    pub mutation: ContextMutation,
}
