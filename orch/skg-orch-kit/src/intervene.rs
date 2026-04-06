use skg_context_engine::{ErasedMiddleware, Middleware};
use thiserror::Error;
use tokio::sync::mpsc;

/// Stable intervention adapter for sending middleware to a running worker.
///
/// The intervention vocabulary stays open: callers can send any
/// `Middleware` or an already-erased middleware.
#[derive(Clone)]
pub struct ContextIntervenor {
    tx: mpsc::Sender<Box<dyn ErasedMiddleware>>,
}

impl ContextIntervenor {
    /// Wrap an existing intervention sender.
    pub fn new(tx: mpsc::Sender<Box<dyn ErasedMiddleware>>) -> Self {
        Self { tx }
    }

    /// Send a typed middleware as an intervention.
    pub async fn send<M>(&self, mw: M) -> Result<(), InterventionSendError>
    where
        M: Middleware + 'static,
    {
        self.send_erased(Box::new(mw)).await
    }

    /// Send an already-erased intervention.
    pub async fn send_erased(
        &self,
        mw: Box<dyn ErasedMiddleware>,
    ) -> Result<(), InterventionSendError> {
        self.tx
            .send(mw)
            .await
            .map_err(|_| InterventionSendError::Closed)
    }

    /// Borrow the wrapped intervention sender.
    pub fn sender(&self) -> &mpsc::Sender<Box<dyn ErasedMiddleware>> {
        &self.tx
    }

    /// Recover the wrapped intervention sender.
    pub fn into_inner(self) -> mpsc::Sender<Box<dyn ErasedMiddleware>> {
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
