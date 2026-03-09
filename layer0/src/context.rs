//! Context management for operator message histories.
//!
//! This module provides [`OperatorContext`] — a typed, watcher-guarded container for
//! an operator's message history. Context is a first-class primitive: it tracks messages,
//! metadata, system prompts, and routes every mutation through registered [`ContextWatcher`]s
//! for observation and approval.
//!
//! ## Core types
//!
//! - [`OperatorContext`] — the mutable container (own this, pass it to the turn loop)
//! - [`ContextMessage`] — a message paired with its [`MessageMeta`]
//! - [`ContextWatcher`] — observer / gatekeeper trait
//! - [`ContextSnapshot`] — read-only introspection view
//! - [`ContextError`] — mutation errors (rejected or out-of-bounds)

use crate::content::Content;
use crate::id::OperatorId;
use crate::lifecycle::CompactionPolicy;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Per-message annotation attached to every message in an [`OperatorContext`].
///
/// All fields are public and directly settable. The [`Default`] implementation
/// uses [`CompactionPolicy::Normal`] and zeros/nones for everything else.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMeta {
    /// Compaction policy governing how this message survives context reduction.
    pub policy: CompactionPolicy,

    /// Source of the message, e.g. `"user"` or `"tool:shell"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Importance hint in the range 0.0–1.0. Higher values should survive compaction longer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,

    /// Monotonic version counter, incremented on each mutation via [`OperatorContext::transform`].
    pub version: u64,
}

impl Default for MessageMeta {
    fn default() -> Self {
        Self {
            policy: CompactionPolicy::Normal,
            source: None,
            salience: None,
            version: 0,
        }
    }
}

impl MessageMeta {
    /// Create metadata with the given policy and defaults for all other fields.
    pub fn with_policy(policy: CompactionPolicy) -> Self {
        Self {
            policy,
            ..Default::default()
        }
    }

    /// Set the source.
    pub fn set_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the salience score.
    pub fn set_salience(mut self, salience: f64) -> Self {
        self.salience = Some(salience);
        self
    }
}

/// Role of a message in the context window.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// System instruction.
    System,
    /// Human message.
    User,
    /// Model response.
    Assistant,
    /// Tool/sub-operator result.
    Tool {
        /// Name of the tool/operator.
        name: String,
        /// Provider-specific call ID for correlation.
        call_id: String,
    },
}

/// A message in an operator's context window.
///
/// Concrete type — not generic. Every message has a role, content,
/// and per-message metadata (compaction policy, salience, source).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Who produced this message.
    pub role: Role,
    /// The message payload.
    pub content: Content,
    /// Per-message annotation (compaction policy, salience, source, version).
    pub meta: MessageMeta,
}

impl Message {
    /// Create a new message with default metadata.
    pub fn new(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta::default(),
        }
    }

    /// Create a message with `CompactionPolicy::Pinned`.
    pub fn pinned(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta {
                policy: CompactionPolicy::Pinned,
                ..Default::default()
            },
        }
    }

    /// Rough token estimate: chars/4 for text, 1000 for images, +4 overhead per message.
    pub fn estimated_tokens(&self) -> usize {
        use crate::content::ContentBlock;
        let content_tokens = match &self.content {
            Content::Text(s) => s.len() / 4,
            Content::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len() / 4,
                    ContentBlock::ToolUse { input, .. } => input.to_string().len() / 4,
                    ContentBlock::ToolResult { content, .. } => content.len() / 4,
                    ContentBlock::Image { .. } => 1000,
                    ContentBlock::Custom { data, .. } => data.to_string().len() / 4,
                })
                .sum(),
        };
        content_tokens + 4 // per-message overhead
    }

    /// Extract all text content for similarity computation.
    pub fn text_content(&self) -> String {
        use crate::content::ContentBlock;
        match &self.content {
            Content::Text(s) => s.clone(),
            Content::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// A message paired with its metadata.
///
/// Parameterised over `M`, the concrete message type used by a particular operator.
/// Both fields are public so callers can construct messages directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessage<M> {
    /// The message payload.
    pub message: M,

    /// Per-message metadata (compaction policy, source, salience, version).
    pub meta: MessageMeta,
}

/// Where to inject a message into an [`OperatorContext`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Append to the end of the message list.
    Back,

    /// Prepend to the beginning of the message list.
    Front,

    /// Insert at the given 0-based index.
    ///
    /// An index equal to the current length is equivalent to [`Position::Back`].
    /// An index greater than the current length returns [`ContextError::OutOfBounds`].
    At(usize),
}

/// The verdict returned by a [`ContextWatcher`] in response to a proposed mutation.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum WatcherVerdict {
    /// Allow the operation to proceed.
    Allow,

    /// Reject the operation with a human-readable reason.
    Reject {
        /// The reason for rejection.
        reason: String,
    },
}

/// Observer and gatekeeper for mutations to an [`OperatorContext`].
///
/// All methods have default implementations that approve the operation (Allow or no-op).
/// Implementors override only the methods they care about.
///
/// Watchers are stored as `Arc<dyn ContextWatcher>` and **must** be `Send + Sync`.
/// Long-running or I/O-heavy watcher logic will add latency to every mutation; keep
/// implementations fast.
///
/// # Object safety
///
/// The trait is object-safe. `on_inject` receives the message as a type-erased
/// [`fmt::Debug`] reference (the concrete `ContextMessage<M>`) so implementations
/// can inspect its debug representation without requiring a generic method.
pub trait ContextWatcher: Send + Sync {
    /// Called before a message is injected.
    ///
    /// `msg` is the full [`ContextMessage<M>`] coerced to `&dyn fmt::Debug`.
    /// `pos` is the requested injection position.
    ///
    /// Return [`WatcherVerdict::Reject`] to abort the inject.
    fn on_inject(&self, msg: &dyn fmt::Debug, pos: Position) -> WatcherVerdict {
        let _ = (msg, pos);
        WatcherVerdict::Allow
    }

    /// Called before messages are removed (truncate or filter operations).
    ///
    /// `count` is the number of messages about to be removed.
    /// Return [`WatcherVerdict::Reject`] to abort the removal.
    fn on_remove(&self, count: usize) -> WatcherVerdict {
        let _ = count;
        WatcherVerdict::Allow
    }

    /// Called before a [`OperatorContext::replace_messages`] compaction runs.
    ///
    /// `message_count` is the number of messages currently in the context.
    /// Return [`WatcherVerdict::Reject`] to abort the compaction.
    fn on_pre_compact(&self, message_count: usize) -> WatcherVerdict {
        let _ = message_count;
        WatcherVerdict::Allow
    }

    /// Called after a [`OperatorContext::replace_messages`] compaction completes.
    ///
    /// `removed` is the number of messages dropped (`old_count - new_count`, clamped to 0).
    /// `remaining` is the count of messages now in the context.
    fn on_post_compact(&self, removed: usize, remaining: usize) {
        let _ = (removed, remaining);
    }
}

/// Read-only snapshot of an [`OperatorContext`] for introspection and logging.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// Number of messages currently in the context.
    pub message_count: usize,

    /// Metadata for each message, in order.
    pub message_metas: Vec<MessageMeta>,

    /// Whether a system prompt is currently set.
    pub has_system: bool,

    /// The operator this context belongs to.
    pub operator_id: OperatorId,

    /// Rough token estimate derived from the debug representation length of messages.
    ///
    /// Uses the heuristic `total_chars / 4`. Not suitable for billing — use it for
    /// soft pressure signals only.
    pub estimated_tokens: usize,
}

/// Errors returned by [`OperatorContext`] mutation methods.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    /// A [`ContextWatcher`] rejected the operation.
    #[error("rejected by watcher: {reason}")]
    Rejected {
        /// The rejection reason from the watcher.
        reason: String,
    },

    /// A position index was past the end of the message list.
    #[error("index {index} is out of bounds (len = {len})")]
    OutOfBounds {
        /// The index that was out of bounds.
        index: usize,

        /// The current length of the message list at the time of the error.
        len: usize,
    },
}

/// A watcher-guarded, typed container for an operator's message history.
///
/// `OperatorContext<M>` is the first-class primitive for managing what messages an
/// operator sees. Every structural mutation (inject, truncate, remove, compact) routes
/// through the registered [`ContextWatcher`]s in registration order before taking
/// effect. A single `Reject` verdict from any watcher aborts the operation and
/// returns [`ContextError::Rejected`].
///
/// # Type Parameter
///
/// `M` is the message type (e.g. an enum of user / assistant / tool messages).
/// It must be `Clone + fmt::Debug`. The `Debug` bound is required so the context
/// can pass messages to watchers and compute token estimates.
///
/// # Watcher invocation order
///
/// Watchers are invoked in registration order (first registered, first called).
/// The first watcher to return `Reject` wins; later watchers are not consulted.
pub struct OperatorContext<M: Clone + fmt::Debug> {
    operator_id: OperatorId,
    messages: Vec<ContextMessage<M>>,
    system: Option<String>,
    watchers: Vec<Arc<dyn ContextWatcher>>,
}

impl<M: Clone + fmt::Debug> OperatorContext<M> {
    /// Create an empty context for the given operator.
    pub fn new(operator_id: OperatorId) -> Self {
        Self {
            operator_id,
            messages: Vec::new(),
            system: None,
            watchers: Vec::new(),
        }
    }

    /// Register a watcher. Watchers are invoked in registration order.
    pub fn add_watcher(&mut self, watcher: Arc<dyn ContextWatcher>) {
        self.watchers.push(watcher);
    }

    /// Read-only slice of all messages, in order.
    pub fn messages(&self) -> &[ContextMessage<M>] {
        &self.messages
    }

    /// Number of messages currently in the context.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Returns `true` if there are no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// The current system prompt, if one is set.
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    /// The operator this context belongs to.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Build a read-only snapshot for introspection or logging.
    ///
    /// Token estimation uses `total_debug_chars / 4`; treat the value as a
    /// soft signal, not a billing figure.
    pub fn snapshot(&self) -> ContextSnapshot {
        let system_chars = self.system.as_ref().map(|s| s.len()).unwrap_or(0);
        let message_chars: usize = self
            .messages
            .iter()
            .map(|m| format!("{:?}", m.message).len())
            .sum();
        let estimated_tokens = (system_chars + message_chars) / 4;

        ContextSnapshot {
            message_count: self.messages.len(),
            message_metas: self.messages.iter().map(|m| m.meta.clone()).collect(),
            has_system: self.system.is_some(),
            operator_id: self.operator_id.clone(),
            estimated_tokens,
        }
    }

    /// Set the system prompt, replacing any existing one.
    pub fn set_system(&mut self, system: impl Into<String>) {
        self.system = Some(system.into());
    }

    /// Remove the system prompt.
    pub fn clear_system(&mut self) {
        self.system = None;
    }

    /// Inject a message at the given position.
    ///
    /// Calls `on_inject` on all registered watchers first. Returns
    /// [`ContextError::Rejected`] if any watcher rejects, or
    /// [`ContextError::OutOfBounds`] if [`Position::At`] exceeds the current
    /// length.
    pub fn inject(&mut self, msg: ContextMessage<M>, pos: Position) -> Result<(), ContextError> {
        for watcher in &self.watchers {
            match watcher.on_inject(&msg, pos) {
                WatcherVerdict::Allow => {}
                WatcherVerdict::Reject { reason } => {
                    return Err(ContextError::Rejected { reason });
                }
            }
        }

        match pos {
            Position::Back => self.messages.push(msg),
            Position::Front => self.messages.insert(0, msg),
            Position::At(idx) => {
                if idx > self.messages.len() {
                    return Err(ContextError::OutOfBounds {
                        index: idx,
                        len: self.messages.len(),
                    });
                }
                self.messages.insert(idx, msg);
            }
        }

        Ok(())
    }

    /// Remove and return the last `count` messages.
    ///
    /// Returns [`ContextError::OutOfBounds`] if `count` exceeds the current length.
    /// Fires `on_remove(count)` on all watchers when `count > 0`.
    pub fn truncate_back(&mut self, count: usize) -> Result<Vec<ContextMessage<M>>, ContextError> {
        if count > self.messages.len() {
            return Err(ContextError::OutOfBounds {
                index: count,
                len: self.messages.len(),
            });
        }

        if count > 0 {
            for watcher in &self.watchers {
                match watcher.on_remove(count) {
                    WatcherVerdict::Allow => {}
                    WatcherVerdict::Reject { reason } => {
                        return Err(ContextError::Rejected { reason });
                    }
                }
            }
        }

        let split_at = self.messages.len() - count;
        Ok(self.messages.drain(split_at..).collect())
    }

    /// Remove and return the first `count` messages.
    ///
    /// Returns [`ContextError::OutOfBounds`] if `count` exceeds the current length.
    /// Fires `on_remove(count)` on all watchers when `count > 0`.
    pub fn truncate_front(&mut self, count: usize) -> Result<Vec<ContextMessage<M>>, ContextError> {
        if count > self.messages.len() {
            return Err(ContextError::OutOfBounds {
                index: count,
                len: self.messages.len(),
            });
        }

        if count > 0 {
            for watcher in &self.watchers {
                match watcher.on_remove(count) {
                    WatcherVerdict::Allow => {}
                    WatcherVerdict::Reject { reason } => {
                        return Err(ContextError::Rejected { reason });
                    }
                }
            }
        }

        Ok(self.messages.drain(..count).collect())
    }

    /// Remove and return all messages matching `pred`.
    ///
    /// Fires `on_remove(n)` on all watchers when `n > 0` matching messages exist.
    /// Preserves the relative order of messages that are not removed.
    pub fn remove_where(
        &mut self,
        pred: impl Fn(&ContextMessage<M>) -> bool,
    ) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let count = self.messages.iter().filter(|m| pred(m)).count();

        if count > 0 {
            for watcher in &self.watchers {
                match watcher.on_remove(count) {
                    WatcherVerdict::Allow => {}
                    WatcherVerdict::Reject { reason } => {
                        return Err(ContextError::Rejected { reason });
                    }
                }
            }
        }

        let mut removed = Vec::new();
        let mut kept = Vec::new();
        for msg in self.messages.drain(..) {
            if pred(&msg) {
                removed.push(msg);
            } else {
                kept.push(msg);
            }
        }
        self.messages = kept;
        Ok(removed)
    }

    /// Apply `f` to every message, incrementing `meta.version` after each call.
    ///
    /// Version increments unconditionally regardless of whether `f` actually
    /// mutated the message — callers should use this only when a real mutation
    /// occurred.
    pub fn transform(&mut self, mut f: impl FnMut(&mut ContextMessage<M>)) {
        for msg in &mut self.messages {
            f(msg);
            msg.meta.version += 1;
        }
    }

    /// Return references to all messages matching `pred` without removing them.
    ///
    /// Non-destructive: the context is unchanged after this call.
    pub fn extract(&self, pred: impl Fn(&ContextMessage<M>) -> bool) -> Vec<&ContextMessage<M>> {
        self.messages.iter().filter(|m| pred(m)).collect()
    }

    /// Direct mutable access to the underlying message vector.
    ///
    /// **Bypasses all watcher checks.** Prefer the typed mutation methods
    /// (`inject`, `remove_where`, `transform`, …) unless you need fine-grained
    /// control that those methods don't expose.
    pub fn messages_mut(&mut self) -> &mut Vec<ContextMessage<M>> {
        &mut self.messages
    }

    /// Replace the entire message list, firing compact watchers.
    ///
    /// Fires `on_pre_compact(old_count)` before the swap and
    /// `on_post_compact(removed, new_count)` after, where
    /// `removed = old_count.saturating_sub(new_count)`.
    ///
    /// Returns the old messages on success, or [`ContextError::Rejected`] if any
    /// watcher's `on_pre_compact` rejects.
    pub fn replace_messages(
        &mut self,
        new: Vec<ContextMessage<M>>,
    ) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let old_count = self.messages.len();

        for watcher in &self.watchers {
            match watcher.on_pre_compact(old_count) {
                WatcherVerdict::Allow => {}
                WatcherVerdict::Reject { reason } => {
                    return Err(ContextError::Rejected { reason });
                }
            }
        }

        let new_count = new.len();
        let old = std::mem::replace(&mut self.messages, new);
        let removed = old_count.saturating_sub(new_count);

        for watcher in &self.watchers {
            watcher.on_post_compact(removed, new_count);
        }

        Ok(old)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context — concrete replacement for OperatorContext<M>
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A concrete, watcher-guarded container for an operator's message history.
///
/// `Context` replaces the generic `OperatorContext<M>` with a concrete type
/// using [`Message`] directly. Every structural mutation routes through
/// registered [`ContextWatcher`]s before taking effect.
///
/// # Compaction
///
/// Three compaction strategies are available as methods:
/// - [`compact_truncate`](Context::compact_truncate) — keep the last N messages
/// - [`compact_by_policy`](Context::compact_by_policy) — remove Normal, keep Pinned
/// - [`compact_with`](Context::compact_with) — caller-supplied closure
pub struct Context {
    operator_id: OperatorId,
    messages: Vec<Message>,
    watchers: Vec<Arc<dyn ContextWatcher>>,
}

impl Context {
    /// Create an empty context for the given operator.
    pub fn new(operator_id: OperatorId) -> Self {
        Self {
            operator_id,
            messages: Vec::new(),
            watchers: Vec::new(),
        }
    }

    /// Register a watcher. Watchers are invoked in registration order.
    pub fn add_watcher(&mut self, watcher: Arc<dyn ContextWatcher>) {
        self.watchers.push(watcher);
    }

    /// Read-only slice of all messages, in order.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Number of messages currently in the context.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Returns `true` if there are no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// The operator this context belongs to.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Rough token estimate for the entire context.
    pub fn estimated_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.estimated_tokens()).sum()
    }

    /// Push a message to the end of the context.
    ///
    /// Calls `on_inject` on all registered watchers. Returns
    /// [`ContextError::Rejected`] if any watcher rejects.
    pub fn push(&mut self, msg: Message) -> Result<(), ContextError> {
        for watcher in &self.watchers {
            match watcher.on_inject(&msg, Position::Back) {
                WatcherVerdict::Allow => {}
                WatcherVerdict::Reject { reason } => {
                    return Err(ContextError::Rejected { reason });
                }
            }
        }
        self.messages.push(msg);
        Ok(())
    }

    /// Insert a message at the given position.
    ///
    /// Calls `on_inject` on all registered watchers. Returns
    /// [`ContextError::Rejected`] if any watcher rejects, or
    /// [`ContextError::OutOfBounds`] if the index exceeds the current length.
    pub fn insert(&mut self, msg: Message, pos: Position) -> Result<(), ContextError> {
        for watcher in &self.watchers {
            match watcher.on_inject(&msg, pos) {
                WatcherVerdict::Allow => {}
                WatcherVerdict::Reject { reason } => {
                    return Err(ContextError::Rejected { reason });
                }
            }
        }
        match pos {
            Position::Back => self.messages.push(msg),
            Position::Front => self.messages.insert(0, msg),
            Position::At(idx) => {
                if idx > self.messages.len() {
                    return Err(ContextError::OutOfBounds {
                        index: idx,
                        len: self.messages.len(),
                    });
                }
                self.messages.insert(idx, msg);
            }
        }
        Ok(())
    }

    /// Keep the last `keep` messages, returning the removed ones.
    ///
    /// Fires compact watchers. Does not respect compaction policy —
    /// use [`compact_by_policy`](Context::compact_by_policy) for that.
    pub fn compact_truncate(&mut self, keep: usize) -> Vec<Message> {
        if keep >= self.messages.len() {
            return Vec::new();
        }
        let old_count = self.messages.len();
        for watcher in &self.watchers {
            watcher.on_pre_compact(old_count);
        }
        let split = self.messages.len() - keep;
        let removed: Vec<Message> = self.messages.drain(..split).collect();
        for watcher in &self.watchers {
            watcher.on_post_compact(removed.len(), self.messages.len());
        }
        removed
    }

    /// Remove all messages with `CompactionPolicy::Normal`, keep `Pinned`.
    ///
    /// Returns the removed messages. Fires compact watchers.
    pub fn compact_by_policy(&mut self) -> Vec<Message> {
        let old_count = self.messages.len();
        for watcher in &self.watchers {
            watcher.on_pre_compact(old_count);
        }
        let mut kept = Vec::new();
        let mut removed = Vec::new();
        for msg in self.messages.drain(..) {
            if matches!(msg.meta.policy, CompactionPolicy::Pinned) {
                kept.push(msg);
            } else {
                removed.push(msg);
            }
        }
        self.messages = kept;
        for watcher in &self.watchers {
            watcher.on_post_compact(removed.len(), self.messages.len());
        }
        removed
    }

    /// Compact using a caller-supplied closure.
    ///
    /// The closure receives `&[Message]` and returns the messages to keep.
    /// Returns the removed messages. Fires compact watchers.
    pub fn compact_with(&mut self, f: impl FnOnce(&[Message]) -> Vec<Message>) -> Vec<Message> {
        let old_count = self.messages.len();
        for watcher in &self.watchers {
            watcher.on_pre_compact(old_count);
        }
        let new_messages = f(&self.messages);
        let old = std::mem::replace(&mut self.messages, new_messages);
        // Determine removed: old messages not in new set
        let removed_count = old.len().saturating_sub(self.messages.len());
        let removed = old;
        for watcher in &self.watchers {
            watcher.on_post_compact(removed_count, self.messages.len());
        }
        removed
    }

    /// Direct mutable access to the underlying message vector.
    ///
    /// **Bypasses all watcher checks.**
    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    /// Build a read-only snapshot for introspection or logging.
    pub fn snapshot(&self) -> ContextSnapshot {
        let estimated_tokens = self.estimated_tokens();
        ContextSnapshot {
            message_count: self.messages.len(),
            message_metas: self.messages.iter().map(|m| m.meta.clone()).collect(),
            has_system: self.messages.iter().any(|m| matches!(m.role, Role::System)),
            operator_id: self.operator_id.clone(),
            estimated_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    type TestMsg = String;

    fn make_msg(s: &str) -> ContextMessage<TestMsg> {
        ContextMessage {
            message: s.to_string(),
            meta: MessageMeta::default(),
        }
    }

    #[test]
    fn new_context_is_empty() {
        let ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("agent-1"));
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);
        assert!(ctx.messages().is_empty());
    }

    #[test]
    fn inject_back_appends_in_order() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("first"), Position::Back).unwrap();
        ctx.inject(make_msg("second"), Position::Back).unwrap();
        assert_eq!(ctx.messages()[0].message, "first");
        assert_eq!(ctx.messages()[1].message, "second");
    }

    #[test]
    fn inject_front_prepends() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("first"), Position::Back).unwrap();
        ctx.inject(make_msg("second"), Position::Front).unwrap();
        assert_eq!(ctx.messages()[0].message, "second");
        assert_eq!(ctx.messages()[1].message, "first");
    }

    #[test]
    fn inject_at_inserts_at_index() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        ctx.inject(make_msg("c"), Position::Back).unwrap();
        ctx.inject(make_msg("b"), Position::At(1)).unwrap();
        assert_eq!(ctx.messages()[0].message, "a");
        assert_eq!(ctx.messages()[1].message, "b");
        assert_eq!(ctx.messages()[2].message, "c");
    }

    #[test]
    fn inject_out_of_bounds_returns_error() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        let err = ctx.inject(make_msg("x"), Position::At(5)).unwrap_err();
        assert!(matches!(
            err,
            ContextError::OutOfBounds { index: 5, len: 0 }
        ));
        // Context must remain unchanged after the error.
        assert!(ctx.is_empty());
    }

    #[test]
    fn truncate_back_removes_from_end() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        ctx.inject(make_msg("b"), Position::Back).unwrap();
        ctx.inject(make_msg("c"), Position::Back).unwrap();

        let removed = ctx.truncate_back(2).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].message, "b");
        assert_eq!(removed[1].message, "c");
        assert_eq!(ctx.len(), 1);
        assert_eq!(ctx.messages()[0].message, "a");
    }

    #[test]
    fn truncate_back_out_of_bounds_returns_error() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        let err = ctx.truncate_back(5).unwrap_err();
        assert!(matches!(
            err,
            ContextError::OutOfBounds { index: 5, len: 1 }
        ));
        assert_eq!(ctx.len(), 1); // unchanged
    }

    #[test]
    fn truncate_front_removes_from_start() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        ctx.inject(make_msg("b"), Position::Back).unwrap();
        ctx.inject(make_msg("c"), Position::Back).unwrap();

        let removed = ctx.truncate_front(2).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].message, "a");
        assert_eq!(removed[1].message, "b");
        assert_eq!(ctx.len(), 1);
        assert_eq!(ctx.messages()[0].message, "c");
    }

    #[test]
    fn truncate_front_out_of_bounds_returns_error() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        let err = ctx.truncate_front(5).unwrap_err();
        assert!(matches!(
            err,
            ContextError::OutOfBounds { index: 5, len: 1 }
        ));
        assert_eq!(ctx.len(), 1); // unchanged
    }

    #[test]
    fn watcher_can_reject_inject() {
        struct RejectAll;

        impl ContextWatcher for RejectAll {
            fn on_inject(&self, _msg: &dyn fmt::Debug, _pos: Position) -> WatcherVerdict {
                WatcherVerdict::Reject {
                    reason: "policy violation".into(),
                }
            }
        }

        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.add_watcher(Arc::new(RejectAll));

        let err = ctx.inject(make_msg("blocked"), Position::Back).unwrap_err();
        assert!(matches!(err, ContextError::Rejected { .. }));
        // Injection must have been rolled back.
        assert!(ctx.is_empty());
    }

    #[test]
    fn snapshot_captures_state() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("my-agent"));
        ctx.set_system("You are helpful.");
        ctx.inject(make_msg("hello"), Position::Back).unwrap();

        let snap = ctx.snapshot();
        assert_eq!(snap.message_count, 1);
        assert!(snap.has_system);
        assert_eq!(snap.operator_id.as_str(), "my-agent");
        assert_eq!(snap.message_metas.len(), 1);
    }

    #[test]
    fn transform_increments_version() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("msg"), Position::Back).unwrap();
        assert_eq!(ctx.messages()[0].meta.version, 0);

        ctx.transform(|_| {});
        assert_eq!(ctx.messages()[0].meta.version, 1);

        ctx.transform(|_| {});
        assert_eq!(ctx.messages()[0].meta.version, 2);
    }

    #[test]
    fn replace_messages_fires_compact_watchers() {
        let pre_called = Arc::new(AtomicBool::new(false));
        let post_called = Arc::new(AtomicBool::new(false));

        struct CompactWatcher {
            pre: Arc<AtomicBool>,
            post: Arc<AtomicBool>,
        }

        impl ContextWatcher for CompactWatcher {
            fn on_pre_compact(&self, _message_count: usize) -> WatcherVerdict {
                self.pre.store(true, Ordering::SeqCst);
                WatcherVerdict::Allow
            }

            fn on_post_compact(&self, _removed: usize, _remaining: usize) {
                self.post.store(true, Ordering::SeqCst);
            }
        }

        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.add_watcher(Arc::new(CompactWatcher {
            pre: Arc::clone(&pre_called),
            post: Arc::clone(&post_called),
        }));

        ctx.inject(make_msg("old"), Position::Back).unwrap();
        let old = ctx.replace_messages(vec![make_msg("new")]).unwrap();

        assert!(
            pre_called.load(Ordering::SeqCst),
            "on_pre_compact not called"
        );
        assert!(
            post_called.load(Ordering::SeqCst),
            "on_post_compact not called"
        );
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].message, "old");
        assert_eq!(ctx.messages()[0].message, "new");
    }

    #[test]
    fn remove_where_filters_correctly() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("keep"), Position::Back).unwrap();
        ctx.inject(make_msg("remove_me"), Position::Back).unwrap();
        ctx.inject(make_msg("also keep"), Position::Back).unwrap();

        let removed = ctx.remove_where(|m| m.message.contains("remove")).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].message, "remove_me");
        assert_eq!(ctx.len(), 2);
        assert_eq!(ctx.messages()[0].message, "keep");
        assert_eq!(ctx.messages()[1].message, "also keep");
    }

    #[test]
    fn extract_is_non_destructive() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        ctx.inject(make_msg("a"), Position::Back).unwrap();
        ctx.inject(make_msg("b"), Position::Back).unwrap();
        ctx.inject(make_msg("c"), Position::Back).unwrap();

        let found = ctx.extract(|m| m.message != "b");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].message, "a");
        assert_eq!(found[1].message, "c");
        // Context must be unchanged.
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn system_prompt_lifecycle() {
        let mut ctx: OperatorContext<TestMsg> = OperatorContext::new(OperatorId::from("a"));
        assert!(ctx.system().is_none());

        ctx.set_system("Hello, system!");
        assert_eq!(ctx.system(), Some("Hello, system!"));

        ctx.clear_system();
        assert!(ctx.system().is_none());
    }

    #[test]
    fn message_construction_and_role_variants() {
        use crate::content::Content;
        use crate::lifecycle::CompactionPolicy;

        let msg = Message {
            role: Role::User,
            content: Content::text("hello"),
            meta: MessageMeta::default(),
        };
        assert!(matches!(msg.role, Role::User));

        let tool_msg = Message {
            role: Role::Tool {
                name: "shell".into(),
                call_id: "tc_1".into(),
            },
            content: Content::text("output"),
            meta: MessageMeta::default(),
        };
        assert!(matches!(tool_msg.role, Role::Tool { .. }));

        let pinned = Message::pinned(Role::System, Content::text("system"));
        assert!(matches!(pinned.meta.policy, CompactionPolicy::Pinned));
    }

    #[test]
    fn message_serde_roundtrip() {
        use crate::content::Content;

        let msg = Message {
            role: Role::Assistant,
            content: Content::text("hi"),
            meta: MessageMeta::default(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let rt: Message = serde_json::from_str(&json).unwrap();
        assert!(matches!(rt.role, Role::Assistant));
    }

    #[test]
    fn message_estimated_tokens() {
        use crate::content::Content;

        // 20 chars / 4 = 5, + 4 overhead = 9
        let msg = Message::new(Role::User, Content::text("12345678901234567890"));
        assert_eq!(msg.estimated_tokens(), 9);
    }

    #[test]
    fn message_text_content_extraction() {
        use crate::content::Content;

        let msg = Message::new(Role::User, Content::text("hello world"));
        assert_eq!(msg.text_content(), "hello world");
    }

    // --- Context tests (Phase 2) ---

    #[test]
    fn context_push_and_read() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("agent-1"));
        ctx.push(Message::new(Role::User, Content::text("hello")))
            .unwrap();
        ctx.push(Message::new(Role::Assistant, Content::text("hi")))
            .unwrap();
        assert_eq!(ctx.len(), 2);
        assert!(matches!(ctx.messages()[0].role, Role::User));
        assert!(matches!(ctx.messages()[1].role, Role::Assistant));
    }

    #[test]
    fn context_compact_truncate() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("a"));
        for i in 0..10 {
            ctx.push(Message::new(
                Role::User,
                Content::text(format!("msg {}", i)),
            ))
            .unwrap();
        }
        let removed = ctx.compact_truncate(3);
        assert_eq!(removed.len(), 7);
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn context_compact_by_policy_preserves_pinned() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("a"));
        ctx.push(Message::pinned(
            Role::System,
            Content::text("you are helpful"),
        ))
        .unwrap();
        for i in 0..5 {
            ctx.push(Message::new(
                Role::User,
                Content::text(format!("msg {}", i)),
            ))
            .unwrap();
        }
        let removed = ctx.compact_by_policy();
        assert_eq!(ctx.len(), 1);
        assert!(matches!(ctx.messages()[0].role, Role::System));
        assert_eq!(removed.len(), 5);
    }

    #[test]
    fn context_compact_with_closure() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("a"));
        for i in 0..6 {
            ctx.push(Message::new(
                Role::User,
                Content::text(format!("msg {}", i)),
            ))
            .unwrap();
        }
        let removed = ctx.compact_with(|msgs| {
            msgs.iter()
                .enumerate()
                .filter(|(i, _)| i % 2 == 0)
                .map(|(_, m)| m.clone())
                .collect()
        });
        assert_eq!(ctx.len(), 3);
        // compact_with returns the old messages, not the removed ones
        assert_eq!(removed.len(), 6);
    }

    #[test]
    fn context_snapshot() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("my-agent"));
        ctx.push(Message::pinned(Role::System, Content::text("system")))
            .unwrap();
        ctx.push(Message::new(Role::User, Content::text("hello")))
            .unwrap();

        let snap = ctx.snapshot();
        assert_eq!(snap.message_count, 2);
        assert!(snap.has_system);
        assert_eq!(snap.operator_id.as_str(), "my-agent");
        assert_eq!(snap.message_metas.len(), 2);
    }

    #[test]
    fn context_estimated_tokens() {
        use crate::content::Content;

        let mut ctx = Context::new(OperatorId::from("a"));
        // 20 chars / 4 = 5, + 4 overhead = 9 per message
        ctx.push(Message::new(
            Role::User,
            Content::text("12345678901234567890"),
        ))
        .unwrap();
        ctx.push(Message::new(
            Role::User,
            Content::text("12345678901234567890"),
        ))
        .unwrap();
        assert_eq!(ctx.estimated_tokens(), 18);
    }
}
