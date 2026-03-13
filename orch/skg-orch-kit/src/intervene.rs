use skg_context_engine::{ContextOp, ErasedOp};
use thiserror::Error;
use tokio::sync::mpsc;

/// Stable intervention adapter for sending context operations to a running worker.
///
/// The intervention vocabulary stays open: callers can send any
/// `ContextOp<Output = ()>` or an already-erased op.
#[derive(Clone)]
pub struct ContextIntervenor {
    tx: mpsc::Sender<Box<dyn ErasedOp>>,
}

impl ContextIntervenor {
    /// Wrap an existing intervention sender.
    pub fn new(tx: mpsc::Sender<Box<dyn ErasedOp>>) -> Self {
        Self { tx }
    }

    /// Send a typed context op as an intervention.
    pub async fn send<O>(&self, op: O) -> Result<(), InterventionSendError>
    where
        O: ContextOp<Output = ()> + 'static,
    {
        self.send_erased(Box::new(op)).await
    }

    /// Send an already-erased intervention.
    pub async fn send_erased(&self, op: Box<dyn ErasedOp>) -> Result<(), InterventionSendError> {
        self.tx
            .send(op)
            .await
            .map_err(|_| InterventionSendError::Closed)
    }

    /// Borrow the wrapped intervention sender.
    pub fn sender(&self) -> &mpsc::Sender<Box<dyn ErasedOp>> {
        &self.tx
    }

    /// Recover the wrapped intervention sender.
    pub fn into_inner(self) -> mpsc::Sender<Box<dyn ErasedOp>> {
        self.tx
    }
}

/// Sending an intervention failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InterventionSendError {
    /// The receiving worker is gone, so the intervention was not delivered.
    #[error("intervention channel is closed")]
    Closed,
}
