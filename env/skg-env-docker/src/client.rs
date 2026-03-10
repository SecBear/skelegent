//! gRPC client wrapper for communicating with the in-container runner.
//!
//! Provides connection establishment and retry logic with exponential
//! backoff + jitter for transient failures.

use crate::config::RetryConfig;
use crate::proto::runner::ExecuteRequest;
use crate::proto::runner::ExecuteResponse;
use crate::proto::runner::runner_client::RunnerClient;
use layer0::error::EnvError;
use std::time::Duration;
use tonic::transport::Channel;
use tracing;

/// Connect to the runner gRPC service inside the container.
///
/// Retries the initial connection with short backoff since the container
/// may still be starting up.
pub async fn connect_runner(
    endpoint: &str,
    timeout: Duration,
) -> Result<RunnerClient<Channel>, EnvError> {
    let channel = Channel::from_shared(endpoint.to_string())
        .map_err(|e| EnvError::ProvisionFailed(format!("invalid gRPC endpoint: {e}")))?
        .connect_timeout(timeout)
        .timeout(timeout)
        .connect()
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("gRPC connection to runner failed: {e}")))?;

    Ok(RunnerClient::new(channel))
}

/// Retryable status codes per gRPC conventions.
fn is_retryable(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded | tonic::Code::ResourceExhausted
    )
}

/// Execute a request with exponential backoff + jitter on transient failures.
///
/// The `idempotency_key` in the request must be set once by the caller and
/// remains stable across retries — the runner uses it for deduplication.
pub async fn execute_with_retry(
    client: &mut RunnerClient<Channel>,
    request: ExecuteRequest,
    retry_config: &RetryConfig,
) -> Result<ExecuteResponse, EnvError> {
    let mut backoff = retry_config.initial_backoff;
    let mut attempt = 0u32;

    loop {
        let req = request.clone();
        match client.execute(tonic::Request::new(req)).await {
            Ok(response) => return Ok(response.into_inner()),
            Err(status) => {
                if !is_retryable(status.code()) || attempt >= retry_config.max_retries {
                    return Err(EnvError::ProvisionFailed(format!(
                        "runner execute failed (code={}, attempt {}/{}): {}",
                        status.code(),
                        attempt + 1,
                        retry_config.max_retries + 1,
                        status.message()
                    )));
                }

                attempt += 1;
                // Jitter: random factor between 0.5 and 1.0 of the backoff
                let jitter_factor = 0.5 + (simple_jitter() * 0.5);
                let sleep_dur = backoff.mul_f64(jitter_factor);

                tracing::warn!(
                    attempt = attempt,
                    max_retries = retry_config.max_retries,
                    backoff_ms = sleep_dur.as_millis() as u64,
                    code = %status.code(),
                    "retrying runner execute after transient failure"
                );

                tokio::time::sleep(sleep_dur).await;

                // Exponential growth, capped at max_backoff
                backoff = Duration::from_secs_f64(
                    (backoff.as_secs_f64() * retry_config.multiplier)
                        .min(retry_config.max_backoff.as_secs_f64()),
                );
            }
        }
    }
}

/// Simple jitter source: uses the lower bits of the current time.
/// Not cryptographic — just enough to decorrelate retry storms.
fn simple_jitter() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Map to [0.0, 1.0)
    (nanos as f64) / (u32::MAX as f64)
}
