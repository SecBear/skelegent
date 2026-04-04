//! Global gate for real LLM API calls.
//!
//! Tests that forget to inject a [`TestProvider`] will silently call live APIs
//! and incur charges. This module provides a process-wide toggle so a test
//! harness can opt-in to failure instead.
//!
//! [`TestProvider`]: crate::test_utils

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide flag; `true` = real requests permitted (default).
static REAL_REQUESTS_ALLOWED: AtomicBool = AtomicBool::new(true);

/// Disallow real LLM API requests for the lifetime of the current process.
///
/// Call this in test setup (e.g., `#[ctor]` or at the top of each test) to
/// ensure any test that forgets to inject a test provider panics immediately
/// rather than hitting a live API.
///
/// Real providers call [`assert_real_requests_allowed`] at the start of every
/// `infer()`. Undo with [`allow_real_requests`].
pub fn deny_real_requests() {
    REAL_REQUESTS_ALLOWED.store(false, Ordering::SeqCst);
}

/// Re-allow real LLM API requests.
///
/// Use in integration tests that intentionally call live APIs, or to restore
/// state after a [`deny_real_requests`] call in a test teardown.
pub fn allow_real_requests() {
    REAL_REQUESTS_ALLOWED.store(true, Ordering::SeqCst);
}

/// Panic if real API requests are currently denied.
///
/// Real provider implementations call this as the first line of `infer()`.
/// Test and function providers **must not** call this — they are always
/// permitted regardless of the global flag.
///
/// # Panics
///
/// Panics with a descriptive message when [`deny_real_requests`] has been
/// called and [`allow_real_requests`] has not been called since.
pub fn assert_real_requests_allowed() {
    if !REAL_REQUESTS_ALLOWED.load(Ordering::SeqCst) {
        panic!(
            "Real LLM API request attempted but denied. \
             Call allow_real_requests() or inject a test provider."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize all tests that touch the global flag so they cannot race.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn deny_blocks_real_requests() {
        let _guard = TEST_LOCK.lock().unwrap();
        deny_real_requests();
        let result = std::panic::catch_unwind(assert_real_requests_allowed);
        allow_real_requests(); // always restore before assertions so later tests aren't poisoned
        assert!(
            result.is_err(),
            "expected panic when real requests are denied"
        );
    }

    #[test]
    fn allow_permits_real_requests() {
        let _guard = TEST_LOCK.lock().unwrap();
        deny_real_requests();
        allow_real_requests();
        // Must not panic.
        assert_real_requests_allowed();
    }
}
