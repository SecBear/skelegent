use skg_context_engine::{Context, EngineError, Middleware};
use skg_orch_kit::{
    ContextIntervenor, ContextObserver, InterventionSendError, Observation, ObservationTry,
};
use std::future::Future;
use tokio::sync::{broadcast, mpsc};

// ---------------------------------------------------------------------------
// Minimal middleware for testing the intervenor channel
// ---------------------------------------------------------------------------

struct NamedMiddleware(&'static str);

impl Middleware for NamedMiddleware {
    fn process(&self, _ctx: &mut Context) -> impl Future<Output = Result<(), EngineError>> + Send {
        std::future::ready(Ok(()))
    }

    fn name(&self) -> &str {
        self.0
    }
}

// ---------------------------------------------------------------------------
// ContextIntervenor tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn intervenor_sends_middleware_to_receiver() {
    let (tx, mut rx) = mpsc::channel(8);
    let intervenor = ContextIntervenor::new(tx);

    intervenor
        .send(NamedMiddleware("supervisor note"))
        .await
        .expect("send succeeds while receiver is attached");

    let received = rx.recv().await.expect("should receive one middleware");
    assert_eq!(received.name(), "supervisor note");
}

#[tokio::test]
async fn intervenor_reports_closed_channel() {
    let (tx, rx) = mpsc::channel(1);
    let intervenor = ContextIntervenor::new(tx);
    drop(rx);

    let err = intervenor
        .send(NamedMiddleware("will fail"))
        .await
        .expect_err("closed worker channel must be reported");
    assert!(matches!(err, InterventionSendError::Closed));
}

#[tokio::test]
async fn intervenor_send_erased_delivers_to_receiver() {
    use skg_context_engine::ErasedMiddleware;

    let (tx, mut rx) = mpsc::channel(8);
    let intervenor = ContextIntervenor::new(tx);

    let boxed: Box<dyn ErasedMiddleware> = Box::new(NamedMiddleware("erased"));
    intervenor
        .send_erased(boxed)
        .await
        .expect("send_erased succeeds while receiver is attached");

    let received = rx.recv().await.expect("should receive erased middleware");
    assert_eq!(received.name(), "erased");
}

// ---------------------------------------------------------------------------
// ContextObserver tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn observer_drains_buffered_events() {
    let (tx, _) = broadcast::channel::<String>(8);
    let mut observer = ContextObserver::subscribe(&tx);

    tx.send("event-one".to_string()).unwrap();
    tx.send("event-two".to_string()).unwrap();

    let batch = observer.drain_available();
    assert_eq!(batch.events.len(), 2);
    assert_eq!(batch.events[0], "event-one");
    assert_eq!(batch.events[1], "event-two");
    assert!(matches!(batch.status, ObservationTry::Empty));
}

#[tokio::test]
async fn observer_reports_closed_channel() {
    let (tx, _) = broadcast::channel::<String>(8);
    let mut observer = ContextObserver::subscribe(&tx);
    drop(tx);

    assert!(matches!(observer.recv().await, Observation::Closed));
}

#[tokio::test]
async fn observer_reports_lag_when_slow() {
    // Capacity 2: sending 3 events causes a slow subscriber to lag
    let (tx, _) = broadcast::channel::<u32>(2);
    let mut observer = ContextObserver::subscribe(&tx);

    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap();

    let batch = observer.drain_available();
    assert!(!batch.events.is_empty(), "should recover post-lag events");
    assert!(
        matches!(batch.status, ObservationTry::Lagged(_)),
        "should report lag in status"
    );
}
