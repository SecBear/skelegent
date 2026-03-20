//! [`SpeakerSelector`] trait and built-in implementations for supervisor routing.
//!
//! A supervisor pattern needs to route between multiple sub-agents. This module
//! defines the selection interface and two simple built-in policies:
//! [`RoundRobinSelector`] and [`RandomSelector`].

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use layer0::DispatchContext;
use layer0::Message;
use layer0::id::OperatorId;

/// Errors from [`SpeakerSelector::select`].
#[non_exhaustive]
#[derive(Debug)]
pub enum SelectorError {
    /// No candidate operators were provided.
    NoCandidates,
    /// Selection failed for another reason.
    Other(String),
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCandidates => write!(f, "no candidates provided"),
            Self::Other(msg) => write!(f, "selector error: {msg}"),
        }
    }
}

impl std::error::Error for SelectorError {}

/// Pluggable routing strategy for supervisor-style multi-agent orchestration.
///
/// Given the current candidates and conversation history, returns the
/// [`OperatorId`] that should speak next.
#[async_trait]
pub trait SpeakerSelector: Send + Sync {
    /// Choose the next speaker from `candidates`.
    ///
    /// `history` carries the conversation messages seen so far (role + content +
    /// attribution); `ctx` provides execution context (auth, trace, extensions).
    async fn select(
        &self,
        candidates: &[OperatorId],
        history: &[Message],
        ctx: &DispatchContext,
    ) -> Result<OperatorId, SelectorError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// RoundRobinSelector
// ─────────────────────────────────────────────────────────────────────────────

/// Cycles through candidates in order, wrapping at the end.
///
/// Thread-safe: uses an atomic counter so concurrent supervisor calls see
/// a consistent (monotonically advancing) position.
pub struct RoundRobinSelector {
    position: AtomicUsize,
}

impl RoundRobinSelector {
    /// Create a new round-robin selector starting at the first candidate.
    pub fn new() -> Self {
        Self {
            position: AtomicUsize::new(0),
        }
    }
}

impl Default for RoundRobinSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SpeakerSelector for RoundRobinSelector {
    async fn select(
        &self,
        candidates: &[OperatorId],
        _history: &[Message],
        _ctx: &DispatchContext,
    ) -> Result<OperatorId, SelectorError> {
        if candidates.is_empty() {
            return Err(SelectorError::NoCandidates);
        }
        // Atomically advance; the modulo below handles wrap-around.
        let idx = self.position.fetch_add(1, Ordering::Relaxed) % candidates.len();
        Ok(candidates[idx].clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RandomSelector
// ─────────────────────────────────────────────────────────────────────────────

/// Selects a candidate using a deterministic pseudo-random sequence.
///
/// Uses a splitmix64-style bit mixer seeded from the call count. Not
/// cryptographically secure, but sufficient for load-balancing and testing.
/// No external `rand` crate is required.
pub struct RandomSelector {
    call_count: AtomicUsize,
}

impl RandomSelector {
    /// Create a new random selector.
    pub fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
        }
    }

    /// Splitmix64-style bit mixer.
    fn mix(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }
}

impl Default for RandomSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SpeakerSelector for RandomSelector {
    async fn select(
        &self,
        candidates: &[OperatorId],
        _history: &[Message],
        _ctx: &DispatchContext,
    ) -> Result<OperatorId, SelectorError> {
        if candidates.is_empty() {
            return Err(SelectorError::NoCandidates);
        }
        let n = self.call_count.fetch_add(1, Ordering::Relaxed) as u64;
        let idx = Self::mix(n) as usize % candidates.len();
        Ok(candidates[idx].clone())
    }
}
