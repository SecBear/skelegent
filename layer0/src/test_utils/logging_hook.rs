//! LoggingHook — logs every event and always returns Continue.

use crate::error::HookError;
use crate::hook::{Hook, HookAction, HookContext, HookPoint};
use async_trait::async_trait;
use std::sync::Mutex;

/// A recorded hook event for inspection in tests.
#[derive(Debug, Clone)]
pub struct RecordedEvent {
    /// The hook point that fired.
    pub point: HookPoint,
    /// Tokens used at the time of the event.
    pub tokens_used: u64,
    /// Turns completed at the time of the event.
    pub turns_completed: u32,
}

/// A hook that records every event and always returns [`HookAction::Continue`].
/// Use `.events()` to inspect what was recorded.
pub struct LoggingHook {
    points: Vec<HookPoint>,
    events: Mutex<Vec<RecordedEvent>>,
}

impl LoggingHook {
    /// Create a new LoggingHook that fires at all hook points.
    pub fn new() -> Self {
        Self {
            points: vec![
                HookPoint::PreInference,
                HookPoint::PostInference,
                HookPoint::PreSubDispatch,
                HookPoint::PostSubDispatch,
                HookPoint::ExitCheck,
            ],
            events: Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all recorded events.
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl Default for LoggingHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Hook for LoggingHook {
    fn points(&self) -> &[HookPoint] {
        &self.points
    }

    async fn on_event(&self, ctx: &HookContext) -> Result<HookAction, HookError> {
        self.events.lock().unwrap().push(RecordedEvent {
            point: ctx.point,
            tokens_used: ctx.tokens_used,
            turns_completed: ctx.turns_completed,
        });
        Ok(HookAction::Continue)
    }
}
