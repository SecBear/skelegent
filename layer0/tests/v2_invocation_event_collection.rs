use layer0::dispatch::{DispatchEvent, InvocationHandle};
use layer0::effect::{Effect, EffectKind};
use layer0::error::ErrorCode;
use layer0::id::DispatchId;
use layer0::intent::{Intent, IntentKind};
use layer0::operator::{Outcome, TerminalOutcome};
use layer0::{Content, OperatorOutput, Scope};
use serde_json::json;

fn completed_outcome() -> Outcome {
    Outcome::Terminal {
        terminal: TerminalOutcome::Completed,
    }
}

/// Helper to run a handle scenario: send events, then collect.
async fn collect_with_events(events: Vec<DispatchEvent>) -> Result<OperatorOutput, layer0::ProtocolError> {
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("collect-test"));
    tokio::spawn(async move {
        for event in events {
            if sender.send(event).await.is_err() {
                return;
            }
        }
    });
    handle.collect().await
}

// ── collect() preserves intents already on Completed output ─────────────────

#[tokio::test]
async fn collect_does_not_duplicate_intents_from_completed_output() {
    let mut output = OperatorOutput::new(Content::text("done"), completed_outcome());
    output.intents.push(Intent::new(IntentKind::DeleteMemory {
        scope: Scope::Global,
        key: "a".into(),
    }));

    let result = collect_with_events(vec![DispatchEvent::Completed { output }])
        .await
        .expect("collect");

    // Intents were already on the output — collect() should NOT duplicate them.
    assert_eq!(result.intents.len(), 1);
}

// ── collect() merges channel effects with output effects ────────────────────

#[tokio::test]
async fn collect_extends_effects_from_channel_and_output() {
    let channel_effect = Effect::new(EffectKind::Custom {
        name: "from-channel".into(),
        payload: json!({}),
    });
    let output_effect = Effect::new(EffectKind::Custom {
        name: "from-output".into(),
        payload: json!({}),
    });

    let mut output = OperatorOutput::new(Content::text("done"), completed_outcome());
    output.effects.push(output_effect);

    let result = collect_with_events(vec![
        DispatchEvent::EffectEmitted {
            effect: channel_effect,
        },
        DispatchEvent::Completed { output },
    ])
    .await
    .expect("collect");

    assert_eq!(result.effects.len(), 2);
}

// ── Missing terminal event error mapping ────────────────────────────────────

#[tokio::test]
async fn missing_terminal_event_returns_unavailable() {
    // Send only intermediate events, no terminal.
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("no-terminal"));
    tokio::spawn(async move {
        let _ = sender
            .send(DispatchEvent::Progress {
                content: Content::text("step 1"),
            })
            .await;
        // Drop sender without terminal event.
    });

    let err = handle.collect().await.expect_err("should fail");
    assert_eq!(err.code, ErrorCode::Unavailable);
}

#[tokio::test]
async fn failed_event_takes_precedence_over_completed() {
    // If both Failed and Completed are sent (shouldn't happen, but test defensively),
    // Failed takes precedence.
    let (handle, sender) = InvocationHandle::channel(DispatchId::new("both-terminals"));
    tokio::spawn(async move {
        let _ = sender
            .send(DispatchEvent::Failed {
                error: layer0::ProtocolError::internal("crash"),
            })
            .await;
        let _ = sender
            .send(DispatchEvent::Completed {
                output: OperatorOutput::new(Content::text("ok"), completed_outcome()),
            })
            .await;
    });

    let err = handle.collect().await.expect_err("should be error");
    assert_eq!(err.code, ErrorCode::Internal);
}
