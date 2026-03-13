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
//!     async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
//!         // delegate to a sibling — goes through orchestrator middleware
//!         let child_output = self.dispatcher
//!             .dispatch(&OperatorId::new("summarizer"), child_input)
//!             .await?
//!             .collect()
//!             .await
//!             .map_err(|e| OperatorError::NonRetryable(e.to_string()))?;
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
use crate::error::OrchError;
use crate::id::{DispatchId, OperatorId};
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
    /// The handle emits [`DispatchEvent`]s as the dispatch progresses.
    /// Call [`DispatchHandle::collect`] to consume all events and return
    /// the terminal [`OperatorOutput`].
    async fn dispatch(
        &self,
        operator: &OperatorId,
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
        error: OrchError,
    },
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

        while let Some(event) = self.rx.recv().await {
            match event {
                DispatchEvent::Completed { output } => {
                    terminal_output = Some(output);
                }
                DispatchEvent::Failed { error } => {
                    terminal_error = Some(error);
                }
                // Intermediate events are discarded by collect().
                _ => {}
            }
        }

        if let Some(error) = terminal_error {
            Err(error)
        } else if let Some(output) = terminal_output {
            Ok(output)
        } else {
            Err(OrchError::DispatchFailed(
                "dispatch ended without terminal event".into(),
            ))
        }
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
    ///     result = operator.execute(input) => { /* handle result */ }
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
}

// Manual Debug impl.
impl std::fmt::Debug for DispatchSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchSender")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}
