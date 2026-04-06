use tokio::sync::broadcast;

/// Stable observation adapter for a typed event stream.
///
/// This wraps a broadcast receiver so orchestrator code can subscribe,
/// poll, or drain events without depending on Tokio's channel result types
/// at every call site. The event type `T` is determined by the middleware
/// that publishes events; observers and publishers share the same `T`.
pub struct ContextObserver<T: Clone> {
    rx: broadcast::Receiver<T>,
}

impl<T: Clone> ContextObserver<T> {
    /// Wrap an existing broadcast receiver.
    pub fn new(rx: broadcast::Receiver<T>) -> Self {
        Self { rx }
    }

    /// Subscribe to a broadcast sender that publishes events.
    pub fn subscribe(tx: &broadcast::Sender<T>) -> Self {
        Self::new(tx.subscribe())
    }

    /// Receive the next event, waiting until one is available.
    ///
    /// Closed and lagged channel states are returned explicitly so callers do
    /// not mistake channel failure for a normal event.
    pub async fn recv(&mut self) -> Observation<T> {
        match self.rx.recv().await {
            Ok(event) => Observation::Event(event),
            Err(broadcast::error::RecvError::Closed) => Observation::Closed,
            Err(broadcast::error::RecvError::Lagged(skipped)) => Observation::Lagged(skipped),
        }
    }

    /// Try to receive one event without waiting.
    pub fn try_recv(&mut self) -> ObservationTry<T> {
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
    pub fn drain_available(&mut self) -> ObservationBatch<T> {
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
    pub fn into_inner(self) -> broadcast::Receiver<T> {
        self.rx
    }

    /// Borrow the wrapped broadcast receiver.
    pub fn receiver(&self) -> &broadcast::Receiver<T> {
        &self.rx
    }
}

/// Outcome of waiting for the next observation.
#[derive(Debug)]
pub enum Observation<T: Clone> {
    /// An event was received.
    Event(T),
    /// The observer fell behind and skipped this many events.
    Lagged(u64),
    /// All senders were dropped; no more events can arrive.
    Closed,
}

/// Outcome of a non-blocking observation poll.
#[derive(Debug, Clone)]
pub enum ObservationTry<T: Clone> {
    /// An event was received.
    Event(T),
    /// No event is currently buffered.
    Empty,
    /// The observer fell behind and skipped this many events.
    Lagged(u64),
    /// All senders were dropped; no more events can arrive.
    Closed,
}

/// Batch of events drained from an observer without waiting.
#[derive(Debug, Clone)]
pub struct ObservationBatch<T: Clone> {
    /// Events drained in FIFO order.
    pub events: Vec<T>,
    /// Why draining stopped.
    pub status: ObservationTry<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_available_returns_empty_on_no_events() {
        let (tx, _) = broadcast::channel::<u32>(16);
        let mut observer = ContextObserver::subscribe(&tx);
        let batch = observer.drain_available();
        assert!(batch.events.is_empty());
        assert!(matches!(batch.status, ObservationTry::Empty));
    }

    #[test]
    fn drain_available_collects_buffered_events() {
        let (tx, _) = broadcast::channel::<u32>(16);
        let mut observer = ContextObserver::subscribe(&tx);
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        let batch = observer.drain_available();
        assert_eq!(batch.events.len(), 2);
        assert!(matches!(batch.status, ObservationTry::Empty));
    }

    #[test]
    fn drain_available_continues_past_lag() {
        // Capacity 2: sending 3 events causes the slow subscriber to lag
        let (tx, _) = broadcast::channel::<u32>(2);
        let mut observer = ContextObserver::subscribe(&tx);
        // Send 3 events into a capacity-2 channel — oldest is evicted
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();
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
        let (tx, _) = broadcast::channel::<u32>(2);
        let mut observer = ContextObserver::subscribe(&tx);
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();
        // Drop sender so channel closes after lag
        drop(tx);
        let batch = observer.drain_available();
        // Closed takes priority over Lagged
        assert!(matches!(batch.status, ObservationTry::Closed));
    }
}
