//! The [`Context`] runtime — first-class mutable substrate for agentic systems.
//!
//! Context carries messages, typed extensions, metrics, and intents.
//! Mutation is direct — no governance wrapper. If you want observation
//! or intervention, add middleware to the [`Pipeline`](crate::Pipeline).

use layer0::content::Content;
use layer0::context::{Message, Role};
use layer0::intent::Intent;
use rust_decimal::Decimal;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::time::Instant;

/// Typed arbitrary state carried alongside messages.
///
/// Components store their configuration, intermediate results, or
/// cross-component communication here. Provides type-safe dynamic storage
/// keyed by `TypeId`.
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Create empty extensions.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Insert a value. Overwrites any existing value of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(val));
    }

    /// Get a reference to a stored value by type.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Get a mutable reference to a stored value by type.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
    }

    /// Remove a stored value by type, returning it if present.
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast().ok())
            .map(|b| *b)
    }
}

impl Default for Extensions {
    fn default() -> Self {
        Self::new()
    }
}

/// Accumulated metrics for the current operator invocation.
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    /// Total input tokens consumed so far.
    pub tokens_in: u64,
    /// Total output tokens consumed so far.
    pub tokens_out: u64,
    /// Cumulative cost in USD.
    pub cost: Decimal,
    /// Number of completed inference turns.
    pub turns_completed: u32,
    /// Total tool calls dispatched.
    pub tool_calls_total: u32,
    /// Total tool calls that returned an error.
    pub tool_calls_failed: u32,
    /// When this operator invocation started.
    pub start: Instant,
}

impl Default for TurnMetrics {
    fn default() -> Self {
        Self {
            tokens_in: 0,
            tokens_out: 0,
            cost: Decimal::ZERO,
            turns_completed: 0,
            tool_calls_total: 0,
            tool_calls_failed: 0,
            start: Instant::now(),
        }
    }
}

impl TurnMetrics {
    /// Create fresh metrics with the clock starting now.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wall-clock elapsed since start.
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Total tokens (in + out).
    pub fn total_tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }
}

/// The mutable substrate for agentic systems.
///
/// Context carries everything an agent needs: the message buffer, typed
/// extensions for cross-component state, accumulated metrics, and intents.
///
/// Mutations are direct — no governance wrapper. If you need observation,
/// budget guards, or intervention, compose them as
/// [`Middleware`](crate::Middleware) in a [`Pipeline`](crate::Pipeline).
pub struct Context {
    /// The message buffer. This is what gets compiled and sent to the model.
    messages: Vec<Message>,
    /// Typed arbitrary state for cross-component communication.
    pub extensions: Extensions,
    /// Accumulated metrics for this operator invocation.
    pub metrics: TurnMetrics,
    /// Intents declared during this operator invocation.
    /// Drained into `OperatorOutput::intents` by the caller.
    intents: Vec<Intent>,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            extensions: Extensions::new(),
            metrics: TurnMetrics::new(),
            intents: Vec::new(),
        }
    }

    // ── Read accessors ────────────────────────────────────────────

    /// The message buffer (read-only).
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Estimated token count of all messages.
    pub fn token_count(&self) -> usize {
        self.messages.iter().map(|m| m.estimated_tokens()).sum()
    }

    // ── Mutation methods (direct) ─────────────────────────────────

    /// Append a message to the context.
    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    /// Insert a message at a specific index.
    ///
    /// # Panics
    ///
    /// Panics if `index > self.messages.len()`.
    pub fn insert_message(&mut self, index: usize, msg: Message) {
        self.messages.insert(index, msg);
    }

    /// Replace a message at a specific index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.messages.len()`.
    pub fn replace_message(&mut self, index: usize, msg: Message) {
        self.messages[index] = msg;
    }

    /// Replace the entire message buffer.
    ///
    /// Used by compaction, which replaces all messages with a compacted set.
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Append multiple messages. Each is appended in order.
    pub fn extend_messages(&mut self, msgs: impl IntoIterator<Item = Message>) {
        for msg in msgs {
            self.push_message(msg);
        }
    }

    // ── Convenience injection methods (direct, non-async) ─────────

    /// Inject or replace the system prompt.
    ///
    /// If a system message already exists at position 0, it is replaced.
    /// Otherwise a new system message is inserted at position 0.
    pub fn inject_system(&mut self, prompt: &str) {
        let system_msg = Message::new(Role::System, Content::text(prompt));
        if self
            .messages
            .first()
            .is_some_and(|m| m.role == Role::System)
        {
            self.replace_message(0, system_msg);
        } else {
            self.insert_message(0, system_msg);
        }
    }

    /// Append a message to context.
    pub fn inject_message(&mut self, msg: Message) {
        self.push_message(msg);
    }

    /// Append multiple messages to context.
    pub fn inject_messages(&mut self, msgs: Vec<Message>) {
        self.extend_messages(msgs);
    }

    /// Append an inference response as an assistant message and update metrics.
    pub fn append_response(&mut self, response: &skg_turn::infer::InferResponse) {
        self.push_message(response.to_message());
        self.metrics.tokens_in += response.usage.input_tokens;
        self.metrics.tokens_out += response.usage.output_tokens;
        if let Some(cost) = response.cost {
            self.metrics.cost += cost;
        }
    }

    // ── Intents ─────────────────────────────────────────────────────

    /// Declare an intent. Stored until drained into OperatorOutput.
    pub fn push_intent(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// Declare multiple intents.
    pub fn extend_intents(&mut self, intents: impl IntoIterator<Item = Intent>) {
        for intent in intents {
            self.push_intent(intent);
        }
    }

    /// Read current intents without draining.
    pub fn intents(&self) -> &[Intent] {
        &self.intents
    }

    /// Drain all intents (transfers ownership to caller).
    pub fn drain_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::context::Role;

    #[test]
    fn push_message_adds_to_end() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "hello");
    }

    #[test]
    fn inject_system_inserts_at_start() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("hello")));
        ctx.inject_system("You are helpful.");
        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].role, Role::System);
        assert_eq!(ctx.messages()[0].text_content(), "You are helpful.");
    }

    #[test]
    fn inject_system_replaces_existing() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("old")));
        ctx.push_message(Message::new(Role::User, Content::text("hello")));
        ctx.inject_system("new");
        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].text_content(), "new");
    }

    #[test]
    fn set_messages_replaces_all() {
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("a")));
        ctx.push_message(Message::new(Role::User, Content::text("b")));
        ctx.set_messages(vec![Message::new(Role::System, Content::text("compacted"))]);
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "compacted");
    }

    #[test]
    fn extend_messages_appends_all() {
        let mut ctx = Context::new();
        ctx.extend_messages(vec![
            Message::new(Role::User, Content::text("a")),
            Message::new(Role::Assistant, Content::text("b")),
        ]);
        assert_eq!(ctx.messages().len(), 2);
    }

    #[test]
    fn metrics_default_values() {
        let m = TurnMetrics::new();
        assert_eq!(m.tokens_in, 0);
        assert_eq!(m.tokens_out, 0);
        assert_eq!(m.cost, Decimal::ZERO);
        assert_eq!(m.turns_completed, 0);
        assert_eq!(m.total_tokens(), 0);
    }

    #[test]
    fn push_intent_stores_and_drains() {
        use layer0::intent::{Intent, IntentKind};
        let mut ctx = Context::new();
        let intent = Intent::new(IntentKind::DeleteMemory {
            scope: layer0::Scope::Global,
            key: "test_key".into(),
        });
        ctx.push_intent(intent.clone());
        assert_eq!(ctx.intents().len(), 1);
        let drained = ctx.drain_intents();
        assert_eq!(drained.len(), 1);
        assert!(ctx.intents().is_empty());
    }

    #[test]
    fn extend_intents_stores_multiple() {
        use layer0::intent::{Intent, IntentKind};
        let mut ctx = Context::new();
        let i1 = Intent::new(IntentKind::DeleteMemory {
            scope: layer0::Scope::Global,
            key: "a".into(),
        });
        let i2 = Intent::new(IntentKind::DeleteMemory {
            scope: layer0::Scope::Global,
            key: "b".into(),
        });
        ctx.extend_intents(vec![i1, i2]);
        assert_eq!(ctx.intents().len(), 2);
    }

    #[test]
    fn append_response_updates_metrics() {
        let mut ctx = Context::new();
        let resp = skg_turn::test_utils::make_text_response("Hello!");
        ctx.append_response(&resp);
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].role, Role::Assistant);
    }
}
