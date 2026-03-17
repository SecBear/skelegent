//! The [`Context`] runtime — first-class mutable substrate for agentic systems.
//!
//! Context carries messages, typed extensions, metrics, and rules.
//! Every mutation goes through [`Context::run()`], which fires applicable rules.

use crate::error::EngineError;
use crate::op::{ContextOp, ErasedOp};
use crate::rule::Rule;
use crate::stream::{ContextEvent, ContextMutation};

use layer0::context::Message;
use rust_decimal::Decimal;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

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
/// extensions for cross-component state, accumulated
///
/// Every operation goes through [`Context::run()`], which dispatches the
/// operation and fires applicable rules before and after. This is how
/// hookability works: rules (budget guards, overwatch agents, telemetry
/// recorders) are participants with the same `&mut Context` power as
/// pipeline operations.
pub struct Context {
    /// The message buffer. This is what gets compiled and sent to the model.
    ///
    /// Access via [`messages()`](Self::messages) for reads and mutation methods
    /// ([`push_message`](Self::push_message), [`insert_message`](Self::insert_message),
    /// [`set_messages`](Self::set_messages)) for writes. Mutation methods emit
    /// to the observation stream; direct field access is not possible.
    messages: Vec<Message>,
    /// Typed arbitrary state for cross-component communication.
    pub extensions: Extensions,
    /// Accumulated metrics for this operator invocation.
    pub metrics: TurnMetrics,
    /// Reactive rules. Sorted by priority (highest first).
    rules: Vec<Rule>,
    /// True when executing a rule — prevents recursive rule firing.
    in_rule: bool,
    /// Observation stream sender. When present, every mutation emits a
    /// [`ContextEvent`] through this broadcast channel.
    stream_tx: Option<broadcast::Sender<ContextEvent>>,
    /// Intervention receiver. When present, pending interventions are
    /// drained and executed at the top of every [`Context::run()`] call.
    intervention_rx: Option<mpsc::Receiver<Box<dyn ErasedOp>>>,
}

impl Context {
    /// Create an empty context with no rules.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            extensions: Extensions::new(),
            metrics: TurnMetrics::new(),
            rules: Vec::new(),
            in_rule: false,
            stream_tx: None,
            intervention_rx: None,
        }
    }

    /// Create a context with the given rules.
    pub fn with_rules(rules: Vec<Rule>) -> Self {
        let mut ctx = Self::new();
        ctx.rules = rules;
        ctx.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        ctx
    }

    /// Attach an observation stream to this context.
    ///
    /// Every mutation through the mutation methods will emit a
    /// [`ContextEvent`] on this channel. Subscribers receive real-time
    /// visibility into operator execution.
    ///
    /// Returns `&mut Self` for chaining.
    pub fn with_stream(&mut self, tx: broadcast::Sender<ContextEvent>) -> &mut Self {
        self.stream_tx = Some(tx);
        self
    }

    /// Attach an intervention channel to this context.
    ///
    /// Pending interventions are drained and executed at the top of every
    /// [`Context::run()`] call. The sending side is held by supervisors,
    /// middleware, or external interfaces.
    ///
    /// Returns `&mut Self` for chaining.
    pub fn with_intervention(&mut self, rx: mpsc::Receiver<Box<dyn ErasedOp>>) -> &mut Self {
        self.intervention_rx = Some(rx);
        self
    }

    /// Get a clone of the stream sender, if one is attached.
    ///
    /// Useful for ops that need to emit custom events (e.g. inference
    /// streaming emits [`ContextMutation::InferenceDelta`] directly).
    pub fn stream_sender(&self) -> Option<&broadcast::Sender<ContextEvent>> {
        self.stream_tx.as_ref()
    }

    // ── Read accessors ────────────────────────────────────────────

    /// The message buffer (read-only).
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    // ── Mutation methods (emit to observation stream) ──────────────

    /// Append a message to the context.
    ///
    /// Emits [`ContextMutation::MessagePushed`] to the observation stream.
    pub fn push_message(&mut self, msg: Message) {
        let arc = Arc::new(msg);
        self.messages.push((*arc).clone());
        self.emit(ContextMutation::MessagePushed(arc));
    }

    /// Insert a message at a specific index.
    ///
    /// Emits [`ContextMutation::MessageInserted`] to the observation stream.
    ///
    /// # Panics
    ///
    /// Panics if `index > self.messages.len()`.
    pub fn insert_message(&mut self, index: usize, msg: Message) {
        let arc = Arc::new(msg);
        self.messages.insert(index, (*arc).clone());
        self.emit(ContextMutation::MessageInserted {
            index,
            message: arc,
        });
    }

    /// Replace a message at a specific index.
    ///
    /// Emits [`ContextMutation::MessageReplaced`] to the observation stream.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.messages.len()`.
    pub fn replace_message(&mut self, index: usize, msg: Message) {
        let arc = Arc::new(msg);
        self.messages[index] = (*arc).clone();
        self.emit(ContextMutation::MessageReplaced {
            index,
            message: arc,
        });
    }

    /// Replace the entire message buffer.
    ///
    /// Emits [`ContextMutation::MessagesSet`] to the observation stream.
    /// Used by compaction, which replaces all messages with a compacted set.
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        let previous_len = self.messages.len();
        self.messages = messages;
        self.emit(ContextMutation::MessagesSet {
            previous_len,
            new_len: self.messages.len(),
        });
    }

    /// Append multiple messages. Each emits [`ContextMutation::MessagePushed`].
    pub fn extend_messages(&mut self, msgs: impl IntoIterator<Item = Message>) {
        for msg in msgs {
            self.push_message(msg);
        }
    }

    /// Emit a context event to the observation stream.
    ///
    /// No-op if no stream sender is attached. If all receivers have been
    /// dropped, the send silently fails (fire-and-forget).
    fn emit(&self, mutation: ContextMutation) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.send(ContextEvent {
                timestamp: Instant::now(),
                mutation,
            });
        }
    }

    // ── Rules and structure ──────────────────────────────────────────

    /// Add a rule to this context. Rules are re-sorted by priority.
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
        self.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Number of rules currently attached.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Estimated token count of all messages.
    pub fn token_count(&self) -> usize {
        self.messages.iter().map(|m| m.estimated_tokens()).sum()
    }

    /// Execute a context operation, firing applicable rules before and after.
    ///
    /// This is the central dispatch point. The sequence is:
    ///
    /// 0. Drain pending interventions (if intervention channel is attached)
    /// 1. Fire `Before` rules matching this op type (highest priority first)
    /// 2. Fire `When` rules whose predicates are true
    /// 3. Execute the operation
    /// 4. Fire `After` rules matching this op type
    ///
    /// Interventions are processed at every `run()` boundary — which happens
    /// constantly during operator execution (every inject, every inference,
    /// every tool call, every compaction). Intervention latency is bounded by
    /// the duration of one ContextOp execution, not one full turn.
    ///
    /// Rules cannot trigger other rules — when executing inside a rule,
    /// the rule dispatch is skipped (and interventions are not drained).
    pub async fn run<O: ContextOp + 'static>(&mut self, op: O) -> Result<O::Output, EngineError> {
        let op_type = TypeId::of::<O>();
        self.enter_governed(op_type).await?;
        let output = op.execute(self).await?;
        self.exit_governed(op_type).await?;
        Ok(output)
    }

    /// Enter a typed governance boundary for non-`ContextOp` work.
    ///
    /// This reuses the same intervention drain + before/when sequence as
    /// [`Context::run()`], but the boundary type is a marker rather than a real
    /// operation. Use this for external discontinuities such as provider calls
    /// that must still be targetable by rules.
    pub(crate) async fn enter_boundary<B: 'static>(&mut self) -> Result<(), EngineError> {
        self.enter_governed(TypeId::of::<B>()).await
    }

    /// Finish a typed governance boundary for non-`ContextOp` work.
    ///
    /// Call this only after the guarded work has succeeded. This matches
    /// [`Context::run()`], which only fires `After` rules when the inner work
    /// completes without error.
    pub(crate) async fn exit_boundary<B: 'static>(&mut self) -> Result<(), EngineError> {
        self.exit_governed(TypeId::of::<B>()).await
    }

    async fn enter_governed(&mut self, op_type: TypeId) -> Result<(), EngineError> {
        if !self.in_rule {
            // Drain pending interventions
            self.drain_interventions().await?;

            // Fire "before" rules
            self.fire_before_rules(op_type).await?;
            self.fire_when_rules().await?;
        }

        Ok(())
    }

    async fn exit_governed(&mut self, op_type: TypeId) -> Result<(), EngineError> {
        if !self.in_rule {
            self.fire_after_rules(op_type).await?;
        }

        Ok(())
    }

    /// Drain and execute all pending interventions.
    ///
    /// Each intervention is a `Box<dyn ErasedOp>` — any ContextOp sent
    /// through the intervention channel. They are executed in FIFO order
    /// as normal context ops (with full `&mut Context` power).
    async fn drain_interventions(&mut self) -> Result<(), EngineError> {
        // Take the receiver out to avoid borrow conflict —
        // execute_erased needs &mut self, but rx is inside self.
        let mut rx = match self.intervention_rx.take() {
            Some(rx) => rx,
            None => return Ok(()),
        };

        while let Ok(intervention) = rx.try_recv() {
            intervention.execute_erased(self).await?;
        }

        // Put it back.
        self.intervention_rx = Some(rx);
        Ok(())
    }

    /// Drain and execute all pending interventions.
    ///
    /// Each intervention is a `Box<dyn ErasedOp>` — any ContextOp sent
    /// through the intervention channel. They are executed in FIFO order
    /// as normal context ops (with full `&mut Context` power).
    async fn drain_interventions(&mut self) -> Result<(), EngineError> {
        // Take the receiver out to avoid borrow conflict —
        // execute_erased needs &mut self, but rx is inside self.
        let mut rx = match self.intervention_rx.take() {
            Some(rx) => rx,
            None => return Ok(()),
        };

        while let Ok(intervention) = rx.try_recv() {
            intervention.execute_erased(self).await?;
        }

        // Put it back.
        self.intervention_rx = Some(rx);
        Ok(())
    }

    /// Fire rules that match `Before` for the given op type.
    pub(crate) async fn fire_before_rules(&mut self, op_type: TypeId) -> Result<(), EngineError> {
        // Collect indices of matching rules to avoid borrow issues
        let indices: Vec<usize> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches_before(op_type))
            .map(|(i, _)| i)
            .collect();

        self.in_rule = true;
        for i in indices {
            // Safety: we're iterating by index and `execute` borrows `ctx`
            // but not `rules` directly. We need unsafe-free approach:
            // temporarily take the op out, execute, put back.
            // Actually, Rule::execute takes &self for the rule and &mut Context.
            // But Rule is inside Context. We need to work around this.
            //
            // Solution: take rules out, execute, put back.
            let rules = std::mem::take(&mut self.rules);
            let result = rules[i].execute(self).await;
            self.rules = rules;
            result?;
        }
        self.in_rule = false;
        Ok(())
    }

    /// Fire rules whose `When` predicates match.
    async fn fire_when_rules(&mut self) -> Result<(), EngineError> {
        // Take rules out to avoid borrow conflict (predicate needs &Context,
        // but rules are inside Context)
        let rules = std::mem::take(&mut self.rules);
        let indices: Vec<usize> = rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                // Evaluate predicate against self (minus rules, which are taken out)
                r.matches_when(self)
            })
            .map(|(i, _)| i)
            .collect();
        self.rules = rules;

        self.in_rule = true;
        for i in indices {
            let rules = std::mem::take(&mut self.rules);
            let result = rules[i].execute(self).await;
            self.rules = rules;
            result?;
        }
        self.in_rule = false;
        Ok(())
    }

    /// Fire rules that match `After` for the given op type.
    pub(crate) async fn fire_after_rules(&mut self, op_type: TypeId) -> Result<(), EngineError> {
        let indices: Vec<usize> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches_after(op_type))
            .map(|(i, _)| i)
            .collect();

        self.in_rule = true;
        for i in indices {
            let rules = std::mem::take(&mut self.rules);
            let result = rules[i].execute(self).await;
            self.rules = rules;
            result?;
        }
        self.in_rule = false;
        Ok(())
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
    use crate::op::ContextOp;
    use async_trait::async_trait;
    use layer0::content::Content;
    use layer0::context::Role;

    struct AddMessage {
        msg: Message,
    }

    #[async_trait]
    impl ContextOp for AddMessage {
        type Output = ();
        async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
            ctx.push_message(self.msg.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn basic_run_adds_message() {
        let mut ctx = Context::new();
        let msg = Message::new(Role::User, Content::text("hello"));
        ctx.run(AddMessage { msg: msg.clone() }).await.unwrap();
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "hello");
    }

    #[tokio::test]
    async fn rules_fire_before_and_after() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let counter = Arc::new(AtomicU32::new(0));

        struct CountOp {
            counter: Arc<AtomicU32>,
        }

        #[async_trait]
        impl ContextOp for CountOp {
            type Output = ();
            async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
                self.counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let before_counter = counter.clone();
        let after_counter = counter.clone();

        let rules = vec![
            Rule::before::<AddMessage>(
                "count_before",
                10,
                CountOp {
                    counter: before_counter,
                },
            ),
            Rule::after::<AddMessage>(
                "count_after",
                10,
                CountOp {
                    counter: after_counter,
                },
            ),
        ];

        let mut ctx = Context::with_rules(rules);
        let msg = Message::new(Role::User, Content::text("test"));
        ctx.run(AddMessage { msg }).await.unwrap();

        // Before + After = 2 increments
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn when_rule_fires_on_predicate() {
        struct InjectWarning;

        #[async_trait]
        impl ContextOp for InjectWarning {
            type Output = ();
            async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
                ctx.push_message(Message::new(
                    Role::User,
                    Content::text("[WARNING] context large"),
                ));
                Ok(())
            }
        }

        let rules = vec![Rule::when(
            "warn_large",
            10,
            |ctx| ctx.messages().len() >= 3,
            InjectWarning,
        )];

        let mut ctx = Context::with_rules(rules);

        // Add 3 messages to trigger the When rule
        for _ in 0..3 {
            ctx.push_message(Message::new(Role::User, Content::text("filler")));
        }

        // Now run an op — the When rule should fire
        let msg = Message::new(Role::User, Content::text("trigger"));
        ctx.run(AddMessage { msg }).await.unwrap();

        // 3 filler + 1 warning (from When rule) + 1 trigger (from AddMessage) = 5
        assert_eq!(ctx.messages().len(), 5);
        assert!(ctx.messages()[3].text_content().contains("WARNING"));
    }

    #[tokio::test]
    async fn halt_error_stops_execution() {
        struct HaltOp;

        #[async_trait]
        impl ContextOp for HaltOp {
            type Output = ();
            async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
                Err(EngineError::Halted {
                    reason: "budget exceeded".into(),
                })
            }
        }

        let rules = vec![Rule::before::<AddMessage>("halt", 100, HaltOp)];

        let mut ctx = Context::with_rules(rules);
        let msg = Message::new(Role::User, Content::text("should not appear"));
        let result = ctx.run(AddMessage { msg }).await;

        assert!(result.is_err());
        assert!(ctx.messages().is_empty()); // op never executed
    }

    #[tokio::test]
    async fn metrics_default_values() {
        let m = TurnMetrics::new();
        assert_eq!(m.tokens_in, 0);
        assert_eq!(m.tokens_out, 0);
        assert_eq!(m.cost, Decimal::ZERO);
        assert_eq!(m.turns_completed, 0);
        assert_eq!(m.total_tokens(), 0);
    }

    // ── Streaming observation tests ──────────────────────────

    #[tokio::test]
    async fn push_message_emits_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let mut ctx = Context::new();
        ctx.with_stream(tx);

        ctx.push_message(Message::new(Role::User, Content::text("hello")));

        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "hello");

        let event = rx.try_recv().unwrap();
        match event.mutation {
            ContextMutation::MessagePushed(msg) => {
                assert_eq!(msg.text_content(), "hello");
            }
            other => panic!("expected MessagePushed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn insert_message_emits_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::System, Content::text("sys")));
        ctx.with_stream(tx);

        ctx.insert_message(1, Message::new(Role::User, Content::text("injected")));

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[1].text_content(), "injected");

        let event = rx.try_recv().unwrap();
        match event.mutation {
            ContextMutation::MessageInserted { index, message } => {
                assert_eq!(index, 1);
                assert_eq!(message.text_content(), "injected");
            }
            other => panic!("expected MessageInserted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replace_message_emits_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("old")));
        ctx.with_stream(tx);

        ctx.replace_message(0, Message::new(Role::User, Content::text("new")));

        assert_eq!(ctx.messages()[0].text_content(), "new");

        let event = rx.try_recv().unwrap();
        match event.mutation {
            ContextMutation::MessageReplaced { index, message } => {
                assert_eq!(index, 0);
                assert_eq!(message.text_content(), "new");
            }
            other => panic!("expected MessageReplaced, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_messages_emits_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("a")));
        ctx.push_message(Message::new(Role::User, Content::text("b")));
        ctx.push_message(Message::new(Role::User, Content::text("c")));
        ctx.with_stream(tx);

        ctx.set_messages(vec![Message::new(Role::System, Content::text("compacted"))]);

        assert_eq!(ctx.messages().len(), 1);

        let event = rx.try_recv().unwrap();
        match event.mutation {
            ContextMutation::MessagesSet {
                previous_len,
                new_len,
            } => {
                assert_eq!(previous_len, 3);
                assert_eq!(new_len, 1);
            }
            other => panic!("expected MessagesSet, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_stream_sender_means_no_cost() {
        // Without a stream sender, mutation methods still work.
        let mut ctx = Context::new();
        ctx.push_message(Message::new(Role::User, Content::text("a")));
        ctx.insert_message(0, Message::new(Role::System, Content::text("sys")));
        ctx.replace_message(1, Message::new(Role::User, Content::text("b")));
        ctx.set_messages(vec![Message::new(Role::User, Content::text("c"))]);
        assert_eq!(ctx.messages().len(), 1);
        assert_eq!(ctx.messages()[0].text_content(), "c");
    }

    #[tokio::test]
    async fn dropped_receivers_do_not_panic() {
        let (tx, rx) = broadcast::channel(16);
        let mut ctx = Context::new();
        ctx.with_stream(tx);
        drop(rx); // All receivers gone.

        // Should not panic — fire-and-forget.
        ctx.push_message(Message::new(Role::User, Content::text("orphan")));
        assert_eq!(ctx.messages().len(), 1);
    }

    // ── Intervention tests ────────────────────────────────────

    #[tokio::test]
    async fn intervention_drains_before_op() {
        let (itx, irx) = mpsc::channel::<Box<dyn crate::op::ErasedOp>>(16);
        let mut ctx = Context::new();
        ctx.with_intervention(irx);

        // Send an intervention that injects a system message.
        struct InjectSys;
        #[async_trait]
        impl ContextOp for InjectSys {
            type Output = ();
            async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
                ctx.push_message(Message::new(
                    Role::System,
                    Content::text("injected by supervisor"),
                ));
                Ok(())
            }
        }
        itx.send(Box::new(InjectSys)).await.unwrap();

        // Now run a normal op.
        let msg = Message::new(Role::User, Content::text("user msg"));
        ctx.run(AddMessage { msg }).await.unwrap();

        // The intervention should have fired first.
        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].text_content(), "injected by supervisor");
        assert_eq!(ctx.messages()[1].text_content(), "user msg");
    }

    #[tokio::test]
    async fn intervention_error_propagates() {
        let (itx, irx) = mpsc::channel::<Box<dyn crate::op::ErasedOp>>(16);
        let mut ctx = Context::new();
        ctx.with_intervention(irx);

        struct HaltIntervention;
        #[async_trait]
        impl ContextOp for HaltIntervention {
            type Output = ();
            async fn execute(&self, _ctx: &mut Context) -> Result<(), EngineError> {
                Err(EngineError::Halted {
                    reason: "supervisor halted".into(),
                })
            }
        }
        itx.send(Box::new(HaltIntervention)).await.unwrap();

        let msg = Message::new(Role::User, Content::text("should not appear"));
        let result = ctx.run(AddMessage { msg }).await;

        assert!(result.is_err());
        assert!(ctx.messages().is_empty()); // Neither intervention nor op added anything useful.
    }

    #[tokio::test]
    async fn multiple_interventions_drain_in_order() {
        let (itx, irx) = mpsc::channel::<Box<dyn crate::op::ErasedOp>>(16);
        let mut ctx = Context::new();
        ctx.with_intervention(irx);

        struct InjectMsg(String);
        #[async_trait]
        impl ContextOp for InjectMsg {
            type Output = ();
            async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
                ctx.push_message(Message::new(Role::System, Content::text(&self.0)));
                Ok(())
            }
        }
        itx.send(Box::new(InjectMsg("first".into()))).await.unwrap();
        itx.send(Box::new(InjectMsg("second".into())))
            .await
            .unwrap();

        let msg = Message::new(Role::User, Content::text("user"));
        ctx.run(AddMessage { msg }).await.unwrap();

        assert_eq!(ctx.messages().len(), 3);
        assert_eq!(ctx.messages()[0].text_content(), "first");
        assert_eq!(ctx.messages()[1].text_content(), "second");
        assert_eq!(ctx.messages()[2].text_content(), "user");
    }

    #[tokio::test]
    async fn no_intervention_channel_is_noop() {
        let mut ctx = Context::new();
        // No intervention channel attached — should work fine.
        let msg = Message::new(Role::User, Content::text("hello"));
        ctx.run(AddMessage { msg }).await.unwrap();
        assert_eq!(ctx.messages().len(), 1);
    }

    #[tokio::test]
    async fn stream_and_intervention_together() {
        let (stx, mut srx) = broadcast::channel(16);
        let (itx, irx) = mpsc::channel::<Box<dyn crate::op::ErasedOp>>(16);
        let mut ctx = Context::new();
        ctx.with_stream(stx);
        ctx.with_intervention(irx);

        struct InjectViaIntervention;
        #[async_trait]
        impl ContextOp for InjectViaIntervention {
            type Output = ();
            async fn execute(&self, ctx: &mut Context) -> Result<(), EngineError> {
                ctx.push_message(Message::new(Role::System, Content::text("from supervisor")));
                Ok(())
            }
        }
        itx.send(Box::new(InjectViaIntervention)).await.unwrap();

        let msg = Message::new(Role::User, Content::text("from user"));
        ctx.run(AddMessage { msg }).await.unwrap();

        // Both messages present.
        assert_eq!(ctx.messages().len(), 2);

        // Stream should have captured the intervention's push_message event.
        let event = srx.try_recv().unwrap();
        match event.mutation {
            ContextMutation::MessagePushed(msg) => {
                assert_eq!(msg.text_content(), "from supervisor");
            }
            other => panic!("expected MessagePushed from intervention, got {other:?}"),
        }
    }
}
