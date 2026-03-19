//! Dispatch — the single invocation primitive.
//!
//! [`Dispatcher`] is the one way to invoke an operator. Orchestrators
//! implement it. Operators that compose hold `Arc<dyn Dispatcher>` as a
//! field (constructor injection).
//!
//! ## Why one trait
//!
//! Mature frameworks (Erlang, Akka, LangChain) converge on a single
//! invocation primitive. `pid ! Message`, `actorRef.tell()`,
//! `Runnable.invoke()`. There is no separate "orchestrator dispatch"
//! vs "operator dispatch" — one interface, used everywhere.
//!
//! ## Streaming by default
//!
//! Every dispatch returns a [`DispatchHandle`] — a streaming receiver of
//! [`DispatchEvent`]s. The simplest usage is `handle.collect().await` which
//! blocks until completion and returns the final [`OperatorOutput`]. But
//! callers that want streaming (progress updates, intermediate artifacts,
//! sub-dispatch tracking) can consume events incrementally via
//! `handle.recv()`.
//!
//! ## Composition via constructor injection
//!
//! Operators that don't compose never see dispatch infrastructure.
//! Operators that do compose receive `Arc<dyn Dispatcher>` at
//! construction time:
//!
//! ```rust,ignore
//! struct CoordinatorOp {
//!     dispatcher: Arc<dyn Dispatcher>,
//!     provider: Arc<dyn Provider>,
//! }
//!
//! impl Operator for CoordinatorOp {
//!     async fn execute(&self, input: OperatorInput, ctx: &DispatchContext) -> Result<OperatorOutput, OperatorError> {
//!         // delegate to a sibling — goes through orchestrator middleware
//!         let child_output = self.dispatcher
//!             .dispatch(&ctx, child_input)
//!             .await?
//!             .collect()
//!             .await
//!             .map_err(|e| OperatorError::non_retryable(e.to_string()))?;
//!         // ...
//!     }
//! }
//! ```
//!
//! The orchestrator passes itself (it implements `Dispatcher`) at
//! registration time. No circular dependency — operators are registered
//! first, then the orchestrator wraps itself as `Arc<dyn Dispatcher>`
//! and injects it into operators that need it.
//!
//! ## Depth tracking
//!
//! Not a framework concern. Erlang and Akka don't limit message-passing
//! depth. If you need it, add a [`DispatchMiddleware`](crate::middleware::DispatchMiddleware)
//! that tracks call depth per session.

use crate::content::Content;
use crate::dispatch_context::DispatchContext;
use crate::effect::Effect;
use crate::error::OrchError;
use crate::id::DispatchId;
use crate::operator::{OperatorInput, OperatorOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

/// The single invocation primitive for operators.
///
/// Every orchestrator implements this. Operators that need to invoke
/// siblings hold `Arc<dyn Dispatcher>` as a field.
///
/// The implementation decides routing: in-process, through middleware,
/// across gRPC, over HTTP. Callers don't know and don't care.
///
/// Returns a [`DispatchHandle`] for streaming events. Use
/// [`DispatchHandle::collect`] when you only need the final output.
#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Start a dispatch and return a streaming handle.
    ///
    /// The caller provides a [`DispatchContext`] carrying the dispatch ID,
    /// target operator, and optional trace/parent context.
    ///
    /// The handle emits [`DispatchEvent`]s as the dispatch progresses.
    /// Call [`DispatchHandle::collect`] to consume all events and return
    /// the terminal [`OperatorOutput`].
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
    ) -> Result<DispatchHandle, OrchError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH EVENTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Events emitted during a dispatch invocation.
///
/// A dispatch produces zero or more intermediate events followed by
/// exactly one terminal event ([`Completed`](Self::Completed) or
/// [`Failed`](Self::Failed)). After the terminal event, no more
/// events are emitted and [`DispatchHandle::recv`] returns `None`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DispatchEvent {
    /// Intermediate progress (reasoning step, partial output, status update).
    ///
    /// The dispatch layer emits this when an operator produces an
    /// [`Effect::Progress`](crate::effect::Effect::Progress).
    Progress {
        /// Progress content.
        content: Content,
    },

    /// An intermediate deliverable produced during execution.
    ///
    /// Emitted when an operator produces an
    /// [`Effect::Artifact`](crate::effect::Effect::Artifact).
    ArtifactProduced {
        /// The artifact produced.
        artifact: Artifact,
    },

    /// An effect was emitted during operator execution.
    ///
    /// Emitted when [`EffectEmitter::effect`] is called by the dispatch layer.
    /// The [`DispatchHandle::collect`] method gathers these into
    /// [`OperatorOutput::effects`].
    EffectEmitted {
        /// The effect that was emitted.
        effect: Effect,
    },

    /// Dispatch completed with final output.
    ///
    /// Terminal event. No further events follow.
    Completed {
        /// Terminal output from the operator.
        output: OperatorOutput,
    },

    /// Dispatch failed.
    ///
    /// Terminal event. No further events follow.
    Failed {
        /// The error.
        #[serde(with = "orch_error_serde")]
        error: OrchError,
    },

    /// Operator is awaiting human approval for pending tool calls.
    ///
    /// Emitted when the dispatch layer suspends a run because one or more
    /// tool calls require a human decision before execution can proceed.
    /// The calling layer routes the [`crate::approval::ApprovalRequest`] to an approver
    /// (human, policy engine, or automated gate) and resumes the run with
    /// an [`crate::approval::ApprovalResponse`].
    AwaitingApproval(crate::approval::ApprovalRequest),
}

impl DispatchEvent {
    /// Returns the SSE event name for this dispatch event.
    ///
    /// Used as the `event:` field in the SSE wire format. Each variant maps to
    /// a stable `"dispatch.*"` string that frontend consumers can switch on.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Progress { .. } => "dispatch.progress",
            Self::ArtifactProduced { .. } => "dispatch.artifact_produced",
            Self::EffectEmitted { .. } => "dispatch.effect_emitted",
            Self::Completed { .. } => "dispatch.completed",
            Self::Failed { .. } => "dispatch.failed",
            Self::AwaitingApproval(_) => "dispatch.awaiting_approval",
        }
    }

    /// Formats this event as a Server-Sent Events line.
    ///
    /// Output format:
    /// ```text
    /// event: {event_type}
    /// data: {json}
    ///
    /// ```
    ///
    /// The trailing `\n\n` terminates the SSE event per the SSE spec (RFC).
    /// Serialization errors produce a JSON error object rather than panicking.
    pub fn to_sse_line(&self) -> String {
        let json = serde_json::to_string(self)
            .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {e}\"}}" ));
        format!("event: {}\ndata: {}\n\n", self.event_type(), json)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ARTIFACT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An intermediate deliverable produced during a dispatch.
///
/// Artifacts are distinct from the terminal output: they represent
/// supplementary outputs (files, structured data, named deliverables)
/// produced while the operator is still executing.
///
/// For streaming protocols (A2A `message/stream`), each artifact is
/// emitted as a separate event. The `append` and `last_chunk` fields
/// support incremental artifact construction.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Unique identifier within the dispatch.
    pub id: String,

    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Description of the artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The artifact content parts.
    pub parts: Vec<Content>,

    /// Optional metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// If true, this artifact's parts are appended to a prior artifact
    /// with the same ID rather than replacing it.
    #[serde(default)]
    pub append: bool,

    /// If true, no more chunks follow for this artifact ID.
    #[serde(default = "default_true")]
    pub last_chunk: bool,
}

fn default_true() -> bool {
    true
}

impl Artifact {
    /// Create a new artifact with the given ID and content parts.
    ///
    /// Defaults: `append = false`, `last_chunk = true` (single complete artifact).
    pub fn new(id: impl Into<String>, parts: Vec<Content>) -> Self {
        Self {
            id: id.into(),
            name: None,
            description: None,
            parts,
            metadata: None,
            append: false,
            last_chunk: true,
        }
    }

    /// Set the human-readable name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Result of collecting all dispatch events, preserving intermediate events.
///
/// Unlike [`DispatchHandle::collect`], which discards intermediate events,
/// [`DispatchHandle::collect_all`] returns this struct containing both the
/// final [`OperatorOutput`] and the full ordered list of intermediate events.
pub struct CollectedDispatch {
    /// Final operator output.
    pub output: OperatorOutput,
    /// All intermediate events received before the terminal event, in order.
    /// Includes `Progress`, `ArtifactProduced`, and `EffectEmitted` events.
    pub events: Vec<DispatchEvent>,
}

impl std::fmt::Debug for CollectedDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectedDispatch")
            .field("output", &self.output)
            .field("events_count", &self.events.len())
            .finish()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH HANDLE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Handle to an in-flight dispatch.
///
/// Returned by [`Dispatcher::dispatch`]. Receives [`DispatchEvent`]s
/// as the dispatch progresses.
///
/// - For simple request/response usage: call [`collect`](Self::collect).
/// - For streaming: call [`recv`](Self::recv) in a loop.
/// - To cancel: call [`cancel`](Self::cancel). The operator will be
///   notified cooperatively — it may still produce a few more events
///   before stopping.
///
/// Dropping the handle unsubscribes from events but does NOT cancel
/// the dispatch. The dispatch continues to completion in the background.
/// Call [`cancel`](Self::cancel) explicitly to request termination.
pub struct DispatchHandle {
    /// Unique identifier for this dispatch.
    pub id: DispatchId,
    rx: mpsc::Receiver<DispatchEvent>,
    cancel_tx: watch::Sender<bool>,
}

impl DispatchHandle {
    /// Create a dispatch channel pair.
    ///
    /// Returns `(handle, sender)`. The orchestrator uses the sender to
    /// push events; the caller receives them through the handle.
    ///
    /// Channel capacity defaults to 64 events. Use [`channel_bounded`](Self::channel_bounded)
    /// for explicit control.
    pub fn channel(id: DispatchId) -> (Self, DispatchSender) {
        Self::channel_bounded(id, 64)
    }

    /// Create a dispatch channel with explicit buffer capacity.
    pub fn channel_bounded(id: DispatchId, capacity: usize) -> (Self, DispatchSender) {
        let (tx, rx) = mpsc::channel(capacity);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let handle = Self { id, rx, cancel_tx };
        let sender = DispatchSender { tx, cancel_rx };
        (handle, sender)
    }

    /// Receive the next event.
    ///
    /// Returns `None` when the dispatch has completed (after the sender
    /// is dropped or a terminal event is sent).
    pub async fn recv(&mut self) -> Option<DispatchEvent> {
        self.rx.recv().await
    }

    /// Request cooperative cancellation of this dispatch.
    ///
    /// The orchestrator checks [`DispatchSender::is_cancelled`] and
    /// should stop the operator as soon as possible. The dispatch may
    /// still produce events after cancellation is requested.
    pub fn cancel(&self) {
        // Ignore send errors — if the receiver is gone, cancellation is moot.
        self.cancel_tx.send(true).ok();
    }

    /// Subscribe to the cancellation signal.
    ///
    /// Orchestrator implementations use this to observe cancellation
    /// from within a spawned task. Completes when `cancel()` is called.
    pub fn cancel_rx(&self) -> watch::Receiver<bool> {
        self.cancel_tx.subscribe()
    }

    /// Consume all events and return the final output.
    ///
    /// This is the migration path for callers that don't need streaming.
    /// Equivalent to the old blocking `Dispatcher::dispatch` behavior.
    pub async fn collect(mut self) -> Result<OperatorOutput, OrchError> {
        let mut terminal_output = None;
        let mut terminal_error = None;
        let mut collected_effects = Vec::new();

        while let Some(event) = self.rx.recv().await {
            match event {
                DispatchEvent::EffectEmitted { effect } => {
                    collected_effects.push(effect);
                }
                DispatchEvent::Completed { output } => {
                    terminal_output = Some(output);
                }
                DispatchEvent::Failed { error } => {
                    terminal_error = Some(error);
                }
                // Intermediate events (Progress, ArtifactProduced) are
                // discarded by collect().
                _ => {}
            }
        }

        if let Some(error) = terminal_error {
            Err(error)
        } else if let Some(mut output) = terminal_output {
            // Effects from the channel are merged with any effects already set
            // on the output (e.g. by make_output()). extend() rather than
            // replace() ensures neither source is silently discarded.
            if !collected_effects.is_empty() {
                output.effects.extend(collected_effects);
            }
            Ok(output)
        } else {
            Err(OrchError::DispatchFailed(
                "dispatch ended without terminal event".into(),
            ))
        }
    }

    /// Wrap this handle with an event interceptor.
    ///
    /// Creates a new channel pair and spawns a background task that reads
    /// events from the inner handle, passes each to the interceptor callback,
    /// and forwards to the new channel. The returned handle has the same API
    /// surface — consumers (including A2A server) work transparently.
    ///
    /// The interceptor sees every event (`Progress`, `ArtifactProduced`, `Completed`,
    /// `Failed`, `EffectEmitted`) before it reaches the consumer. It cannot modify
    /// or block events — use middleware guards for that.
    ///
    /// Cancellation is propagated: if the outer (returned) handle is cancelled,
    /// the cancellation is forwarded to the inner handle's sender.
    pub fn intercept<F>(self, f: F) -> Self
    where
        F: Fn(&DispatchEvent) + Send + Sync + 'static,
    {
        let (new_handle, sender) = Self::channel(self.id.clone());
        // Subscribe to cancellation on the *new* handle so we can forward it.
        let mut outer_cancel = new_handle.cancel_tx.subscribe();
        let mut inner = self;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Propagate outer cancellation into the inner handle.
                    result = outer_cancel.changed() => {
                        if result.is_ok() && *outer_cancel.borrow() {
                            inner.cancel();
                        }
                        // Don't break here — keep draining events until the inner
                        // sender closes naturally.
                    }
                    event = inner.recv() => {
                        match event {
                            None => break, // inner sender dropped — done
                            Some(ev) => {
                                f(&ev);
                                if sender.send(ev).await.is_err() {
                                    break; // consumer dropped outer handle
                                }
                            }
                        }
                    }
                }
            }
        });
        new_handle
    }

    /// Consume all events, preserving intermediate events alongside the final output.
    ///
    /// Unlike [`collect`](Self::collect), this method retains all `Progress`,
    /// `ArtifactProduced`, and `EffectEmitted` events in the order they were received.
    ///
    /// `EffectEmitted` events appear in both the `events` vec and `output.effects`,
    /// consistent with how [`collect`](Self::collect) populates `output.effects`.
    pub async fn collect_all(mut self) -> Result<CollectedDispatch, OrchError> {
        let mut events = Vec::new();
        let mut terminal_output = None;
        let mut terminal_error = None;

        while let Some(event) = self.rx.recv().await {
            match event {
                DispatchEvent::Completed { output } => {
                    terminal_output = Some(output);
                }
                DispatchEvent::Failed { error } => {
                    terminal_error = Some(error);
                }
                // All intermediate events are preserved.
                other => {
                    events.push(other);
                }
            }
        }

        if let Some(error) = terminal_error {
            return Err(error);
        }

        let Some(mut output) = terminal_output else {
            return Err(OrchError::DispatchFailed(
                "dispatch ended without terminal event".into(),
            ));
        };

        // Populate output.effects from EffectEmitted events, same as collect().
        let collected_effects: Vec<Effect> = events
            .iter()
            .filter_map(|e| match e {
                DispatchEvent::EffectEmitted { effect } => Some(effect.clone()),
                _ => None,
            })
            .collect();

        if !collected_effects.is_empty() {
            output.effects.extend(collected_effects);
        }

        Ok(CollectedDispatch { output, events })
    }
}

// Manual Debug impl because mpsc::Receiver and watch::Sender don't impl Debug.
impl std::fmt::Debug for DispatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Sender half of a dispatch channel.
///
/// Created by [`DispatchHandle::channel`]. The orchestrator uses this to
/// push [`DispatchEvent`]s to the caller's handle.
///
/// Drop the sender after sending the terminal event
/// ([`DispatchEvent::Completed`] or [`DispatchEvent::Failed`]) to signal
/// end-of-stream.
pub struct DispatchSender {
    tx: mpsc::Sender<DispatchEvent>,
    cancel_rx: watch::Receiver<bool>,
}

impl Clone for DispatchSender {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            cancel_rx: self.cancel_rx.clone(),
        }
    }
}

impl DispatchSender {
    /// Send an event to the dispatch handle.
    ///
    /// Returns `Err` with the unsent event if the handle has been dropped.
    pub async fn send(
        &self,
        event: DispatchEvent,
    ) -> Result<(), mpsc::error::SendError<DispatchEvent>> {
        self.tx.send(event).await
    }

    /// Check whether the caller has requested cancellation.
    pub fn is_cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    /// Wait until cancellation is requested.
    ///
    /// Useful in `tokio::select!` to race cancellation against work:
    ///
    /// ```rust,ignore
    /// tokio::select! {
    ///     result = operator.execute(input, &ctx) => { /* handle result */ }
    ///     _ = sender.cancelled() => { /* handle cancellation */ }
    /// }
    /// ```
    pub async fn cancelled(&mut self) {
        // If already cancelled, return immediately.
        if *self.cancel_rx.borrow() {
            return;
        }
        // Wait for the value to change to true.
        loop {
            if self.cancel_rx.changed().await.is_err() {
                // Sender (cancel_tx) dropped — handle gone, treat as cancelled.
                return;
            }
            if *self.cancel_rx.borrow() {
                return;
            }
        }
    }

    /// Clone the cancellation receiver.
    ///
    /// Used by [`EffectEmitter`] to observe cancellation without
    /// requiring `&mut self`.
    pub(crate) fn cancel_rx_clone(&self) -> watch::Receiver<bool> {
        self.cancel_rx.clone()
    }
}

// Manual Debug impl.
impl std::fmt::Debug for DispatchSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchSender")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EFFECT EMITTER
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Channel for streaming observable events during operator execution.
///
/// Used by the dispatch layer to wire a channel for streaming progress updates,
/// artifacts, and other observable events to the dispatch caller's [`DispatchHandle`]
/// in real-time. It is NOT passed to operators — operators declare effects via
/// `Context::push_effect()` / `Context::extend_effects()` instead.
///
/// # Design
///
/// This is the Rust equivalent of Python's `StreamWriter` (LangGraph)
/// or `yield` in an async generator (ADK, Autogen). The dispatch layer
/// streams intermediate observable events via the emitter; the
/// terminal result comes from the function return value. These are
/// genuinely different categories — intermediate observations vs.
/// final output — so two mechanisms is correct modeling.
///
/// The emitter wraps an `Option<DispatchSender>`: `None` when no
/// consumer is listening (tests, batch callers). Emission methods
/// become no-ops in that case — zero overhead.
pub struct EffectEmitter {
    sender: Option<DispatchSender>,
}

impl EffectEmitter {
    /// Create an emitter that forwards events to a dispatch handle.
    pub fn new(sender: DispatchSender) -> Self {
        Self {
            sender: Some(sender),
        }
    }

    /// Create a no-op emitter that discards all events.
    ///
    /// Use in tests or when no streaming consumer exists.
    pub fn noop() -> Self {
        Self { sender: None }
    }

    /// Emit an intermediate progress event (reasoning step, partial output).
    ///
    /// No-op if no consumer is listening.
    pub async fn progress(&self, content: Content) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(DispatchEvent::Progress { content }).await;
        }
    }

    /// Emit an intermediate artifact produced during execution.
    ///
    /// No-op if no consumer is listening.
    pub async fn artifact(&self, artifact: Artifact) {
        let _ = self
            .emit(DispatchEvent::ArtifactProduced { artifact })
            .await;
    }

    /// Emit an effect through the dispatch channel.
    ///
    /// Used by the dispatch layer to stream effects through the dispatch
    /// channel. The dispatch handle's [`collect`](DispatchHandle::collect)
    /// method gathers emitted effects into [`OperatorOutput::effects`].
    ///
    /// No-op if no consumer is listening.
    pub async fn effect(&self, effect: Effect) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(DispatchEvent::EffectEmitted { effect }).await;
        }
    }

    /// Emit a raw [`DispatchEvent`].
    ///
    /// Prefer the typed methods ([`progress`](Self::progress),
    /// [`artifact`](Self::artifact)) for common cases. Use this for
    /// custom or future event types.
    ///
    /// Returns `Ok(())` if sent or no consumer, `Err` if the
    /// consumer dropped the handle.
    pub async fn emit(&self, event: DispatchEvent) -> Result<(), ()> {
        if let Some(ref sender) = self.sender {
            sender.send(event).await.map_err(|_| ())
        } else {
            Ok(())
        }
    }

    /// Check whether the dispatch caller has requested cancellation.
    ///
    /// Used by the dispatch layer to observe caller-side cancellation.
    /// `EffectEmitter` is not passed to operators — they use
    /// [`DispatchContext`](crate::dispatch_context::DispatchContext).
    pub fn is_cancelled(&self) -> bool {
        self.sender.as_ref().is_some_and(|s| s.is_cancelled())
    }

    /// Wait until the dispatch caller requests cancellation.
    ///
    /// Used by the dispatch layer to race cancellation against ongoing work via
    /// `tokio::select!`. Returns immediately when no consumer exists (no-op emitter).
    /// `EffectEmitter` is not passed to operators — they use
    /// [`DispatchContext`](crate::dispatch_context::DispatchContext).
    pub async fn cancelled(&self) {
        if let Some(ref sender) = self.sender {
            // Clone the cancel_rx to get a mutable receiver without
            // requiring &mut self.
            let mut rx = sender.cancel_rx_clone();
            if *rx.borrow() {
                return;
            }
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                if *rx.borrow() {
                    return;
                }
            }
        }
    }
}

impl std::fmt::Debug for EffectEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectEmitter")
            .field("active", &self.sender.is_some())
            .finish_non_exhaustive()
    }
}

// Serde adapter for OrchError: serializes as Display string, deserializes as DispatchFailed.
// OrchError cannot derive Serialize/Deserialize because several variants hold Box<dyn Error>.
// For SSE consumers the error message is sufficient.
mod orch_error_serde {
    use crate::error::OrchError;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(error: &OrchError, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&error.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OrchError, D::Error> {
        let msg = String::deserialize(d)?;
        Ok(OrchError::DispatchFailed(msg))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::id::DispatchId;
    use crate::operator::OperatorOutput;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    /// Helper: create a handle that immediately emits the given events then completes.
    fn make_handle_with_events(events: Vec<DispatchEvent>) -> DispatchHandle {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("test-intercept"));
        tokio::spawn(async move {
            for ev in events {
                if sender.send(ev).await.is_err() {
                    return;
                }
            }
        });
        handle
    }

    fn progress_event(text: &str) -> DispatchEvent {
        DispatchEvent::Progress {
            content: Content::text(text),
        }
    }

    fn completed_event() -> DispatchEvent {
        DispatchEvent::Completed {
            output: OperatorOutput::new(
                Content::text("done"),
                crate::operator::ExitReason::Complete,
            ),
        }
    }

    #[tokio::test]
    async fn handle_intercept_sees_all_events() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let handle = make_handle_with_events(vec![
            progress_event("a"),
            progress_event("b"),
            progress_event("c"),
            completed_event(),
        ]);

        let intercepted = handle.intercept(move |_ev| {
            c.fetch_add(1, Ordering::SeqCst);
        });

        let _ = intercepted.collect().await.unwrap();

        // Allow the spawn to flush — the intercept task runs concurrently.
        tokio::task::yield_now().await;

        // 4 events: 3 progress + 1 completed
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn handle_intercept_transparent_to_consumer() {
        let handle = make_handle_with_events(vec![
            progress_event("step1"),
            progress_event("step2"),
            completed_event(),
        ]);

        let intercepted = handle.intercept(|_ev| { /* no-op */ });

        // collect() should still work exactly as without interception.
        let output = intercepted.collect().await.unwrap();
        assert_eq!(output.message.as_text().unwrap(), "done");
    }

    #[tokio::test]
    async fn handle_intercept_chains() {
        let counter_a = Arc::new(AtomicU32::new(0));
        let counter_b = Arc::new(AtomicU32::new(0));
        let ca = counter_a.clone();
        let cb = counter_b.clone();

        let handle = make_handle_with_events(vec![progress_event("x"), completed_event()]);

        // Chain two interceptors.
        let intercepted = handle
            .intercept(move |_| {
                ca.fetch_add(1, Ordering::SeqCst);
            })
            .intercept(move |_| {
                cb.fetch_add(1, Ordering::SeqCst);
            });

        let _ = intercepted.collect().await.unwrap();
        tokio::task::yield_now().await;

        // Both interceptors see all events: 1 progress + 1 completed = 2 each.
        assert_eq!(counter_a.load(Ordering::SeqCst), 2);
        assert_eq!(counter_b.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn handle_intercept_propagates_cancellation() {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("cancel-test"));
        let s = sender.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let _ = s
                .send(DispatchEvent::Completed {
                    output: OperatorOutput::new(
                        Content::text("should not reach"),
                        crate::operator::ExitReason::Complete,
                    ),
                })
                .await;
        });
        let intercepted = handle.intercept(|_| {});
        intercepted.cancel();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            sender.is_cancelled(),
            "inner sender should be cancelled after outer cancel"
        );
    }

    #[tokio::test]
    async fn handle_intercept_exits_when_consumer_drops() {
        let events_seen = Arc::new(AtomicU32::new(0));
        let seen = events_seen.clone();
        let handle = make_handle_with_events(vec![
            progress_event("a"),
            progress_event("b"),
            progress_event("c"),
            completed_event(),
        ]);
        let intercepted = handle.intercept(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        drop(intercepted);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn handle_intercept_panicking_callback_does_not_hang() {
        let handle = make_handle_with_events(vec![progress_event("a"), completed_event()]);
        let intercepted = handle.intercept(|_| panic!("boom"));
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), intercepted.collect()).await;
        assert!(
            result.is_ok(),
            "collect should not hang when callback panics"
        );
    }

    /// collect() must extend output.effects rather than replace them.
    ///
    /// Scenario: the operator sets output.effects in make_output() (e.g. WriteMemory),
    /// and ALSO emits effects via the EffectEmitted channel (e.g. a Signal). Both
    /// must appear in the final output — neither should shadow the other.
    #[tokio::test]
    async fn collect_extends_output_effects_not_replaces() {
        use crate::effect::Effect;

        let channel_effect = Effect::Custom {
            effect_type: "channel-effect".into(),
            data: serde_json::json!({}),
        };
        let output_effect = Effect::Custom {
            effect_type: "output-effect".into(),
            data: serde_json::json!({}),
        };

        let (handle, sender) = DispatchHandle::channel(DispatchId::new("extend-test"));
        tokio::spawn(async move {
            // Emit an effect via the channel.
            let _ = sender
                .send(DispatchEvent::EffectEmitted {
                    effect: channel_effect,
                })
                .await;
            // Complete with an output that already carries its own effect.
            let mut output =
                OperatorOutput::new(Content::text("done"), crate::operator::ExitReason::Complete);
            output.effects.push(output_effect);
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });

        let result = handle.collect().await.unwrap();

        // Both effects must survive — channel effects extend, not replace.
        assert_eq!(
            result.effects.len(),
            2,
            "expected both effects; got {:?}",
            result.effects
        );
        let types: Vec<&str> = result
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::Custom { effect_type, .. } => Some(effect_type.as_str()),
                _ => None,
            })
            .collect();
        assert!(types.contains(&"output-effect"), "output-effect missing");
        assert!(types.contains(&"channel-effect"), "channel-effect missing");
    }

    #[test]
    fn dispatch_event_serde_round_trip() {
        use crate::approval::ApprovalRequest;
        use crate::content::Content;
        use crate::effect::Effect;
        use crate::error::OrchError;
        use crate::operator::{ExitReason, OperatorOutput};

        // Helper: serialize then deserialize and assert no panic.
        fn round_trip(ev: DispatchEvent) {
            let json = serde_json::to_string(&ev).expect("serialize");
            assert!(!json.is_empty());
            let _: DispatchEvent = serde_json::from_str(&json).expect("deserialize");
        }

        round_trip(DispatchEvent::Progress { content: Content::text("thinking") });
        round_trip(DispatchEvent::ArtifactProduced {
            artifact: Artifact::new("a1", vec![Content::text("output")]),
        });
        round_trip(DispatchEvent::EffectEmitted {
            effect: Effect::Custom {
                effect_type: "ping".into(),
                data: serde_json::json!({}),
            },
        });
        round_trip(DispatchEvent::Completed {
            output: OperatorOutput::new(Content::text("done"), ExitReason::Complete),
        });
        // Failed: error serialized as string, deserializes as DispatchFailed.
        round_trip(DispatchEvent::Failed {
            error: OrchError::DispatchFailed("something went wrong".into()),
        });
        round_trip(DispatchEvent::AwaitingApproval(ApprovalRequest::new(
            "run-1", "wp-1", vec![],
        )));
    }

    #[test]
    fn event_type_names() {
        use crate::approval::ApprovalRequest;
        use crate::content::Content;
        use crate::effect::Effect;
        use crate::error::OrchError;
        use crate::operator::{ExitReason, OperatorOutput};

        assert_eq!(
            DispatchEvent::Progress { content: Content::text("x") }.event_type(),
            "dispatch.progress"
        );
        assert_eq!(
            DispatchEvent::ArtifactProduced { artifact: Artifact::new("a", vec![]) }
                .event_type(),
            "dispatch.artifact_produced"
        );
        assert_eq!(
            DispatchEvent::EffectEmitted {
                effect: Effect::Custom {
                    effect_type: "x".into(),
                    data: serde_json::json!({}),
                },
            }
            .event_type(),
            "dispatch.effect_emitted"
        );
        assert_eq!(
            DispatchEvent::Completed {
                output: OperatorOutput::new(Content::text("done"), ExitReason::Complete),
            }
            .event_type(),
            "dispatch.completed"
        );
        assert_eq!(
            DispatchEvent::Failed {
                error: OrchError::DispatchFailed("x".into()),
            }
            .event_type(),
            "dispatch.failed"
        );
        assert_eq!(
            DispatchEvent::AwaitingApproval(ApprovalRequest::new("r", "w", vec![]))
                .event_type(),
            "dispatch.awaiting_approval"
        );
    }

    #[test]
    fn sse_format() {
        use crate::content::Content;

        let ev = DispatchEvent::Progress { content: Content::text("step 1") };
        let line = ev.to_sse_line();
        assert!(
            line.starts_with("event: dispatch.progress\ndata: "),
            "unexpected prefix: {line:?}"
        );
        assert!(line.ends_with("\n\n"), "missing trailing blank: {line:?}");
        // Extract and validate the JSON payload.
        let data_start = line.find("data: ").unwrap() + 6;
        let data_end = line.len() - 2; // strip trailing \n\n
        let data = &line[data_start..data_end];
        let v: serde_json::Value = serde_json::from_str(data).expect("valid JSON");
        assert_eq!(v["type"], "progress");
    }

}
