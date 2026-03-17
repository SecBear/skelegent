use skg_context_engine::ContextEvent;
use tokio::sync::broadcast;

/// Stable observation adapter for a context event stream.
///
/// This wraps the raw broadcast receiver used by `skg-context-engine` so
/// orchestrator code can subscribe, poll, or drain events without depending on
/// Tokio's channel result types at every call site.
pub struct ContextObserver {
    rx: broadcast::Receiver<ContextEvent>,
}

impl ContextObserver {
    /// Wrap an existing context event receiver.
    pub fn new(rx: broadcast::Receiver<ContextEvent>) -> Self {
        Self { rx }
    }

    /// Subscribe to a broadcast sender that publishes context events.
    pub fn subscribe(tx: &broadcast::Sender<ContextEvent>) -> Self {
        Self::new(tx.subscribe())
    }

    /// Receive the next context event, waiting until one is available.
    ///
    /// Closed and lagged channel states are returned explicitly so callers do
    /// not mistake channel failure for a normal event.
    pub async fn recv(&mut self) -> Observation {
        match self.rx.recv().await {
            Ok(event) => Observation::Event(event),
            Err(broadcast::error::RecvError::Closed) => Observation::Closed,
            Err(broadcast::error::RecvError::Lagged(skipped)) => Observation::Lagged(skipped),
        }
    }

    /// Try to receive one context event without waiting.
    pub fn try_recv(&mut self) -> ObservationTry {
        match self.rx.try_recv() {
            Ok(event) => ObservationTry::Event(event),
            Err(broadcast::error::TryRecvError::Empty) => ObservationTry::Empty,
            Err(broadcast::error::TryRecvError::Closed) => ObservationTry::Closed,
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => ObservationTry::Lagged(skipped),
        }
    }

    /// Drain all currently buffered events without waiting for new ones.
    ///
    /// The returned status reports why draining stopped: the buffer may be
    /// empty, the observer may have lagged behind, or the stream may be closed.
    pub fn drain_available(&mut self) -> ObservationBatch {
        let mut events = Vec::new();
        let mut total_lagged: u64 = 0;

        loop {
            match self.try_recv() {
                ObservationTry::Event(event) => events.push(event),
                ObservationTry::Lagged(skipped) => {
                    // Broadcast channel repositioned — keep draining post-lag events
                    total_lagged += skipped;
                }
                ObservationTry::Empty => {
                    let status = if total_lagged > 0 {
                        ObservationTry::Lagged(total_lagged)
                    } else {
                        ObservationTry::Empty
                    };
                    return ObservationBatch { events, status };
                }
                ObservationTry::Closed => {
                    return ObservationBatch {
                        events,
                        status: ObservationTry::Closed,
                    };
                }
            }
        }
    }

    /// Recover the wrapped broadcast receiver.
    pub fn into_inner(self) -> broadcast::Receiver<ContextEvent> {
        self.rx
    }

    /// Borrow the wrapped broadcast receiver.
    pub fn receiver(&self) -> &broadcast::Receiver<ContextEvent> {
        &self.rx
    }
}

/// Outcome of waiting for the next observation.
#[derive(Debug)]
pub enum Observation {
    /// A context event was received.
    Event(ContextEvent),
    /// The observer fell behind and skipped this many events.
    Lagged(u64),
    /// All senders were dropped; no more events can arrive.
    Closed,
}

/// Outcome of a non-blocking observation poll.
#[derive(Debug, Clone)]
pub enum ObservationTry {
    /// A context event was received.
    Event(ContextEvent),
    /// No event is currently buffered.
    Empty,
    /// The observer fell behind and skipped this many events.
    Lagged(u64),
    /// All senders were dropped; no more events can arrive.
    Closed,
}

/// Batch of events drained from an observer without waiting.
#[derive(Debug, Clone)]
pub struct ObservationBatch {
    /// Events drained in FIFO order.
    pub events: Vec<ContextEvent>,
    /// Why draining stopped.
    pub status: ObservationTry,
}

#[cfg(test)]
mod tests {
    use super::*;
    use skg_context_engine::ContextMutation;
    use std::time::Instant;

    fn dummy_event() -> ContextEvent {
        ContextEvent {
            timestamp: Instant::now(),
            mutation: ContextMutation::MessagesSet {
                previous_len: 0,
                new_len: 0,
            },
        }
    }

    #[test]
    fn drain_available_returns_empty_on_no_events() {
        let (tx, _) = broadcast::channel::<ContextEvent>(16);
        let mut observer = ContextObserver::subscribe(&tx);
        let batch = observer.drain_available();
        assert!(batch.events.is_empty());
        assert!(matches!(batch.status, ObservationTry::Empty));
    }

    #[test]
    fn drain_available_collects_buffered_events() {
        let (tx, _) = broadcast::channel::<ContextEvent>(16);
        let mut observer = ContextObserver::subscribe(&tx);
        tx.send(dummy_event()).unwrap();
        tx.send(dummy_event()).unwrap();
        let batch = observer.drain_available();
        assert_eq!(batch.events.len(), 2);
        assert!(matches!(batch.status, ObservationTry::Empty));
    }

    #[test]
    fn drain_available_continues_past_lag() {
        // Capacity 2: sending 3 events causes the slow subscriber to lag
        let (tx, _) = broadcast::channel::<ContextEvent>(2);
        let mut observer = ContextObserver::subscribe(&tx);
        // Send 3 events into a capacity-2 channel — oldest is evicted
        tx.send(dummy_event()).unwrap();
        tx.send(dummy_event()).unwrap();
        tx.send(dummy_event()).unwrap();
        let batch = observer.drain_available();
        // Should have recovered post-lag events, not returned empty
        assert!(!batch.events.is_empty(), "should recover events after lag");
        assert!(
            matches!(batch.status, ObservationTry::Lagged(_)),
            "should report lag"
        );
    }

    #[test]
    fn drain_available_closed_after_lag_reports_closed() {
        let (tx, _) = broadcast::channel::<ContextEvent>(2);
        let mut observer = ContextObserver::subscribe(&tx);
        tx.send(dummy_event()).unwrap();
        tx.send(dummy_event()).unwrap();
        tx.send(dummy_event()).unwrap();
        // Drop sender so channel closes after lag
        drop(tx);
        let batch = observer.drain_available();
        // Closed takes priority over Lagged
        assert!(matches!(batch.status, ObservationTry::Closed));
    }
}
