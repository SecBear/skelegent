use layer0::dispatch::{DispatchEvent, InvocationHandle};
use layer0::error::ErrorCode;
use layer0::id::DispatchId;
use layer0::intent::{Intent, IntentKind};
use layer0::operator::{Outcome, TerminalOutcome};
use layer0::{Content, OperatorOutput, Scope};

fn completed_outcome() -> Outcome {
    Outcome::Terminal {
        terminal: TerminalOutcome::Completed,
    }
}

// ── collect() preserves intents from terminal output ────────────────────────

#[tokio::test]
async fn collect_preserves_intents_from_completed_output() {
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("intent-test"));
    tokio::spawn(async move {
        let mut output = OperatorOutput::new(Content::text("done"), completed_outcome());
        output.intents.push(Intent::new(IntentKind::DeleteMemory {
            scope: Scope::Global,
            key: "k".into(),
        }));
        let _ = sender.send(DispatchEvent::Completed { output }).await;
    });

    let result = handle.collect().await.expect("collect");
    assert_eq!(result.intents.len(), 1);
    assert!(matches!(result.intents[0].kind, IntentKind::DeleteMemory { .. }));
}

// ── Missing terminal event returns Unavailable error ────────────────────────

#[tokio::test]
async fn missing_terminal_event_returns_error() {
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("no-terminal"));
    // Drop the sender without sending a terminal event.
    drop(sender);

    let err = handle.collect().await.expect_err("should fail");
    // Current behavior: dispatch ended without terminal event → Unavailable.
    assert_eq!(err.code, ErrorCode::Unavailable);
}

#[tokio::test]
async fn missing_terminal_after_cancellation_returns_error() {
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("cancelled"));
    handle.cancel();
    // Drop sender after cancellation to simulate no terminal event.
    drop(sender);

    let err = handle.collect().await.expect_err("should fail after cancel");
    // Missing terminal event after cancellation also returns Unavailable.
    assert_eq!(err.code, ErrorCode::Unavailable);
}

// ── Failed event propagates ProtocolError ───────────────────────────────────

#[tokio::test]
async fn collect_propagates_failed_event_as_error() {
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("fail-test"));
    tokio::spawn(async move {
        let _ = sender
            .send(DispatchEvent::Failed {
                error: layer0::ProtocolError::internal("boom"),
            })
            .await;
    });

    let err = handle.collect().await.expect_err("should be error");
    assert_eq!(err.code, ErrorCode::Internal);
    assert_eq!(err.message, "boom");
}
