#![deny(missing_docs)]
//! Retry middleware for skelegent — configurable backoff and deadline-aware dispatch retries.
//!
//! Wraps a [`DispatchMiddleware`] that automatically retries failed dispatches
//! using configurable backoff strategies, respecting [`DispatchContext`] deadlines.

use async_trait::async_trait;
use layer0::dispatch::DispatchHandle;
use layer0::dispatch_context::DispatchContext;
use layer0::error::{EnvError, OrchError};
use layer0::middleware::{DispatchMiddleware, DispatchNext};
use layer0::operator::OperatorInput;
use std::time::Duration;
use rand::Rng;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CONFIGURATION
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Configuration for retry behavior.
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial attempt).
    pub max_retries: u32,
    /// Base delay between retries.
    pub base_delay: Duration,
    /// Backoff strategy.
    pub backoff: BackoffStrategy,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            backoff: BackoffStrategy::Exponential,
        }
    }
}

/// Strategy for computing delay between retry attempts.
#[non_exhaustive]
pub enum BackoffStrategy {
    /// Fixed delay between retries.
    Fixed,
    /// Exponential backoff: delay * 2^attempt.
    Exponential,
    /// Full jitter: uniform random in [0, min(max_backoff, base * 2^attempt)).
    ///
    /// Eliminates thundering-herd problems by spreading retries randomly across
    /// the full computed window rather than clustering at its boundary.
    FullJitter {
        /// Upper bound on the computed backoff window before sampling.
        max_backoff: Duration,
    },
}

impl BackoffStrategy {
    /// Compute the delay for a given attempt (0-indexed).
    fn delay(&self, base: Duration, attempt: u32) -> Duration {
        match self {
            Self::Fixed => base,
            Self::Exponential => base.saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX)),
            Self::FullJitter { max_backoff } => {
                // Exponential window, capped at max_backoff.
                let window = base
                    .saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX))
                    .min(*max_backoff);
                // Sample uniformly from [0, window]. Nanosecond precision.
                let cap_nanos =
                    u64::try_from(window.as_nanos()).unwrap_or(u64::MAX);
                Duration::from_nanos(rand::thread_rng().gen_range(0..=cap_nanos))
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RETRYABILITY
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default classification: transient dispatch failures are retryable;
/// operator-not-found errors are not. Operator errors delegate to
/// [`OperatorError::is_retryable`] for fine-grained classification.
fn is_retryable_default(err: &OrchError) -> bool {
    match err {
        // Transient failures worth retrying.
        OrchError::DispatchFailed(_) | OrchError::SignalFailed(_) => true,
        // Permanent failures — retrying won't help.
        OrchError::OperatorNotFound(_) | OrchError::WorkflowNotFound(_) => false,
        // Environment errors: provisioning is transient; operator errors delegate;
        // isolation/credential/resource violations are permanent.
        OrchError::EnvironmentError(env_err) => match env_err {
            EnvError::ProvisionFailed(_) => true,
            EnvError::OperatorError(inner) => inner.is_retryable(),
            _ => false,
        },
        // Operator errors: delegate to inner retryability.
        OrchError::OperatorError(op_err) => op_err.is_retryable(),
        // Unknown variants (non_exhaustive): default to not retrying.
        _ => false,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MIDDLEWARE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Predicate that decides whether a failed dispatch should be retried.
pub type RetryPredicate = dyn Fn(&OrchError) -> bool + Send + Sync;

/// Dispatch middleware that retries failed calls with configurable backoff.
///
/// Respects [`DispatchContext::deadline`] — will not start a retry attempt
/// if the context has expired.
pub struct RetryMiddleware {
    config: RetryConfig,
    is_retryable: Box<RetryPredicate>,
}

impl RetryMiddleware {
    /// Create a new retry middleware with the given configuration.
    ///
    /// Uses the default retryability classifier: `DispatchFailed` and
    /// `SignalFailed` are retryable; all other errors are not.
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            is_retryable: Box::new(is_retryable_default),
        }
    }

    /// Override the retryability predicate.
    pub fn with_predicate<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&OrchError) -> bool + Send + Sync + 'static,
    {
        self.is_retryable = Box::new(predicate);
        self
    }
}

#[async_trait]
impl DispatchMiddleware for RetryMiddleware {
    /// Dispatch with retries on transient failure.
    async fn dispatch(
        &self,
        ctx: &DispatchContext,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<DispatchHandle, OrchError> {
        let mut last_err: Option<OrchError> = None;

        for attempt in 0..=self.config.max_retries {
            // Don't start a new attempt if the deadline has passed.
            if ctx.is_expired() {
                tracing::warn!(
                    attempt,
                    max_retries = self.config.max_retries,
                    "deadline expired, aborting retry loop"
                );
                break;
            }

            // Wait before retry (skip delay on the initial attempt).
            if attempt > 0 {
                let delay = self
                    .config
                    .backoff
                    .delay(self.config.base_delay, attempt - 1);

                // If the delay would push us past the deadline, don't bother.
                if let Some(remaining) = ctx.remaining()
                    && remaining <= delay
                {
                    tracing::warn!(
                        attempt,
                        ?delay,
                        ?remaining,
                        "remaining time less than backoff delay, aborting retry loop"
                    );
                    break;
                }

                tracing::info!(
                    attempt,
                    max_retries = self.config.max_retries,
                    ?delay,
                    "retrying dispatch"
                );
                tokio::time::sleep(delay).await;
            }

            match next.dispatch(ctx, input.clone()).await {
                Ok(handle) => return Ok(handle),
                Err(err) => {
                    if !(self.is_retryable)(&err) {
                        tracing::debug!(attempt, %err, "error is not retryable, giving up");
                        return Err(err);
                    }
                    tracing::warn!(attempt, %err, "dispatch failed, will retry");
                    last_err = Some(err);
                }
            }
        }

        // All retries exhausted or deadline expired — return the last error.
        Err(last_err.unwrap_or_else(|| {
            OrchError::DispatchFailed(
                "retry loop exited without an error (deadline expired before first attempt)".into(),
            )
        }))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use layer0::ExitReason;
    use layer0::content::Content;
    use layer0::dispatch::{DispatchEvent, DispatchHandle};
    use layer0::error::OperatorError;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{OperatorOutput, TriggerType};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Helper: create a DispatchHandle that immediately completes.
    fn immediate_handle(output: OperatorOutput) -> DispatchHandle {
        let (handle, sender) = DispatchHandle::channel(DispatchId::new("test"));
        tokio::spawn(async move {
            let _ = sender.send(DispatchEvent::Completed { output }).await;
        });
        handle
    }

    fn test_input() -> OperatorInput {
        OperatorInput::new(Content::text("hello"), TriggerType::User)
    }

    fn test_ctx() -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::from("op"))
    }

    // ── Fails N times then succeeds ────────────────────────

    /// A mock DispatchNext that fails `n` times then succeeds.
    struct FailNThenSucceed {
        remaining_failures: AtomicU32,
    }

    impl FailNThenSucceed {
        fn new(n: u32) -> Self {
            Self {
                remaining_failures: AtomicU32::new(n),
            }
        }
    }

    #[async_trait]
    impl DispatchNext for FailNThenSucceed {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            _input: OperatorInput,
        ) -> Result<DispatchHandle, OrchError> {
            let prev = self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            if prev > 0 {
                Err(OrchError::DispatchFailed("transient".into()))
            } else {
                Ok(immediate_handle(OperatorOutput::new(
                    Content::text("ok"),
                    ExitReason::Complete,
                )))
            }
        }
    }

    #[tokio::test]
    async fn retry_on_transient_error() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            backoff: BackoffStrategy::Fixed,
        });

        // Fails 2 times, then succeeds on the 3rd attempt.
        let mock = FailNThenSucceed::new(2);
        let result = mw.dispatch(&test_ctx(), test_input(), &mock).await;
        assert!(result.is_ok(), "should succeed after retries");
    }

    #[tokio::test]
    async fn no_retry_on_non_retryable_error() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct NonRetryableError(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchNext for NonRetryableError {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
            ) -> Result<DispatchHandle, OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(OrchError::OperatorNotFound("missing".into()))
            }
        }

        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            backoff: BackoffStrategy::Fixed,
        });

        let mock = NonRetryableError(count);
        let result = mw.dispatch(&test_ctx(), test_input(), &mock).await;
        assert!(result.is_err());
        // Should have been called exactly once — no retries.
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn respects_max_retries() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct AlwaysFail(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchNext for AlwaysFail {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
            ) -> Result<DispatchHandle, OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(OrchError::DispatchFailed("always fails".into()))
            }
        }

        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            backoff: BackoffStrategy::Fixed,
        });

        let mock = AlwaysFail(count);
        let result = mw.dispatch(&test_ctx(), test_input(), &mock).await;
        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total calls.
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn respects_deadline() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct AlwaysFail(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchNext for AlwaysFail {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
            ) -> Result<DispatchHandle, OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(OrchError::DispatchFailed("always fails".into()))
            }
        }

        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 10,
            base_delay: Duration::from_millis(50),
            backoff: BackoffStrategy::Fixed,
        });

        // Set a deadline that expires quickly — won't survive many retries.
        let ctx = test_ctx().with_timeout(Duration::from_millis(80));
        let mock = AlwaysFail(count);
        let result = mw.dispatch(&ctx, test_input(), &mock).await;
        assert!(result.is_err());
        // Should have stopped well before 10 retries due to deadline.
        let calls = call_count.load(Ordering::SeqCst);
        assert!(
            calls < 10,
            "should have stopped retrying before max_retries due to deadline, but made {calls} calls"
        );
    }

    // ── Operator-error retryability propagation ─────────────

    /// Mock that fails once with a retryable OperatorError, then succeeds.
    struct RetryableOperatorErrorThenSucceed {
        remaining: AtomicU32,
    }

    impl RetryableOperatorErrorThenSucceed {
        fn new() -> Self {
            Self {
                remaining: AtomicU32::new(1),
            }
        }
    }

    #[async_trait]
    impl DispatchNext for RetryableOperatorErrorThenSucceed {
        async fn dispatch(
            &self,
            _ctx: &DispatchContext,
            _input: OperatorInput,
        ) -> Result<DispatchHandle, OrchError> {
            let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
            if prev > 0 {
                Err(OrchError::OperatorError(OperatorError::model_retryable(
                    std::io::Error::other("rate limited"),
                )))
            } else {
                Ok(immediate_handle(OperatorOutput::new(
                    Content::text("ok"),
                    ExitReason::Complete,
                )))
            }
        }
    }

    #[tokio::test]
    async fn retries_retryable_operator_error() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            backoff: BackoffStrategy::Fixed,
        });

        let mock = RetryableOperatorErrorThenSucceed::new();
        let result = mw.dispatch(&test_ctx(), test_input(), &mock).await;
        assert!(
            result.is_ok(),
            "should succeed after retrying retryable operator error"
        );
    }

    #[tokio::test]
    async fn no_retry_on_permanent_operator_error() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct PermanentOperatorError(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchNext for PermanentOperatorError {
            async fn dispatch(
                &self,
                _ctx: &DispatchContext,
                _input: OperatorInput,
            ) -> Result<DispatchHandle, OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(OrchError::OperatorError(OperatorError::model("permanent")))
            }
        }

        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            backoff: BackoffStrategy::Fixed,
        });

        let mock = PermanentOperatorError(count);
        let result = mw.dispatch(&test_ctx(), test_input(), &mock).await;
        assert!(result.is_err());
        // Should have been called exactly once — no retries for permanent errors.
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    // ── BackoffStrategy::FullJitter ────────────────────────────

    #[test]
    fn full_jitter_stays_within_window() {
        let strategy = BackoffStrategy::FullJitter {
            max_backoff: Duration::from_millis(500),
        };
        let base = Duration::from_millis(100);
        // 100 samples across several attempts; none must exceed the cap.
        for attempt in 0..6u32 {
            let cap = base
                .saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX))
                .min(Duration::from_millis(500));
            for _ in 0..100 {
                let d = strategy.delay(base, attempt);
                assert!(
                    d <= cap,
                    "attempt {attempt}: jitter {d:?} exceeded cap {cap:?}"
                );
            }
        }
    }

    #[test]
    fn full_jitter_respects_max_backoff_cap() {
        let strategy = BackoffStrategy::FullJitter {
            max_backoff: Duration::from_millis(200),
        };
        // At attempt 10, uncapped window is 100ms * 2^10 = 102.4s >> 200ms cap.
        let base = Duration::from_millis(100);
        for _ in 0..200 {
            let d = strategy.delay(base, 10);
            assert!(
                d <= Duration::from_millis(200),
                "jitter {d:?} exceeded max_backoff 200ms"
            );
        }
    }

    // ── EnvironmentError retryability ────────────────────────────

    #[test]
    fn provision_failed_is_retryable() {
        use layer0::error::EnvError;
        assert!(is_retryable_default(&OrchError::EnvironmentError(
            EnvError::ProvisionFailed("spawn timeout".into()),
        )));
    }

    #[test]
    fn isolation_violation_is_not_retryable() {
        use layer0::error::EnvError;
        assert!(!is_retryable_default(&OrchError::EnvironmentError(
            EnvError::IsolationViolation("sandbox breach".into()),
        )));
    }

    #[test]
    fn credential_failed_is_not_retryable() {
        use layer0::error::EnvError;
        assert!(!is_retryable_default(&OrchError::EnvironmentError(
            EnvError::CredentialFailed("missing token".into()),
        )));
    }

    #[test]
    fn env_operator_error_delegates_to_inner() {
        use layer0::error::EnvError;
        // Retryable inner.
        assert!(is_retryable_default(&OrchError::EnvironmentError(
            EnvError::OperatorError(OperatorError::retryable("transient")),
        )));
        // Non-retryable inner.
        assert!(!is_retryable_default(&OrchError::EnvironmentError(
            EnvError::OperatorError(OperatorError::non_retryable("permanent")),
        )));
    }
}
