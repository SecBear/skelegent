//! `skg-runner` — Universal operator runner binary.
//!
//! Runs inside a Docker container and exposes:
//! - A gRPC server (tonic) implementing the Runner service on port 50051
//! - HTTP/JSON endpoints (axum) on port 8080 for healthcheck and convenience API
//!
//! The runner authenticates requests via `session_key`, validates the
//! `EnvironmentSpec`, loads the requested operator from a compiled-in
//! registry, executes it, and returns the result.

mod http_adapter;
mod registry;

use std::sync::Arc;

use layer0::dispatch::Capabilities;
use tokio::signal;
use tonic::transport::Server as TonicServer;
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

mod proto {
    tonic::include_proto!("skg.runner.v1");
}

use proto::runner_server::{Runner, RunnerServer};
use proto::{
    execute_event, ExecuteEvent, ExecuteRequest, ExecuteResponse, HealthRequest, HealthResponse,
};

use registry::OperatorRegistry;

/// gRPC port the runner listens on inside the container.
const GRPC_PORT: u16 = 50051;
/// HTTP port for healthcheck and JSON API.
const HTTP_PORT: u16 = 8080;

// ---------------------------------------------------------------------------
// Transport-agnostic error type
// ---------------------------------------------------------------------------

/// Errors from core runner logic, independent of transport (gRPC / HTTP).
pub enum CoreError {
    Unauthenticated(String),
    NotFound(String),
    InvalidArgument(String),
    Internal(String),
}

impl CoreError {
    pub fn message(&self) -> &str {
        match self {
            CoreError::Unauthenticated(m)
            | CoreError::NotFound(m)
            | CoreError::InvalidArgument(m)
            | CoreError::Internal(m) => m,
        }
    }
}

impl From<CoreError> for Status {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Unauthenticated(m) => Status::unauthenticated(m),
            CoreError::NotFound(m) => Status::not_found(m),
            CoreError::InvalidArgument(m) => Status::invalid_argument(m),
            CoreError::Internal(m) => Status::internal(m),
        }
    }
}

// ---------------------------------------------------------------------------
// Core runner service
// ---------------------------------------------------------------------------

/// Core runner service implementation.
///
/// Holds a compiled-in operator registry and validates requests
/// against an expected session key set at startup.
pub struct RunnerServiceImpl {
    registry: Arc<OperatorRegistry>,
    expected_session_key: String,
}

impl RunnerServiceImpl {
    fn new(registry: Arc<OperatorRegistry>, expected_session_key: String) -> Self {
        Self {
            registry,
            expected_session_key,
        }
    }

    /// Validate session key.
    pub fn validate_session_key(&self, provided: &str) -> Result<(), CoreError> {
        if provided.is_empty() || provided != self.expected_session_key {
            return Err(CoreError::Unauthenticated("invalid session key".into()));
        }
        Ok(())
    }

    /// Deserialize `OperatorInput` from JSON bytes.
    fn deserialize_input(&self, bytes: &[u8]) -> Result<layer0::OperatorInput, CoreError> {
        serde_json::from_slice(bytes).map_err(|e| {
            warn!("failed to deserialize OperatorInput: {e}");
            CoreError::InvalidArgument("failed to deserialize OperatorInput".into())
        })
    }

    /// Look up an operator by id.
    fn resolve_operator(
        &self,
        operator_id: &str,
    ) -> Result<Arc<dyn layer0::Operator>, CoreError> {
        self.registry
            .get(operator_id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("operator not found: {operator_id}")))
    }

    /// Shared execute pipeline used by both gRPC and HTTP transports.
    ///
    /// Takes raw `input_bytes` (JSON-encoded `OperatorInput`), dispatches
    /// to the named operator, and returns serialized `OperatorOutput`.
    pub async fn execute_core(
        &self,
        operator_id: &str,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        let input = self.deserialize_input(input_bytes)?;
        let operator = self.resolve_operator(operator_id)?;

        // Spawn in a task to catch panics from operator implementations.
        let handle = tokio::task::spawn(async move { operator.execute(input, &Capabilities::none()).await });

        let result = handle.await.map_err(|join_err| {
            error!("operator panicked: {join_err}");
            CoreError::Internal("operator execution failed".into())
        })?;

        let output = result.map_err(|op_err| {
            error!("operator error: {op_err}");
            CoreError::Internal("operator execution failed".into())
        })?;

        serde_json::to_vec(&output).map_err(|e| {
            error!("failed to serialize OperatorOutput: {e}");
            CoreError::Internal("failed to serialize operator output".into())
        })
    }
}

// ---------------------------------------------------------------------------
// gRPC transport (tonic)
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl Runner for RunnerServiceImpl {
    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<ExecuteResponse>, Status> {
        let req = request.into_inner();

        self.validate_session_key(&req.session_key)?;
        let output_bytes = self.execute_core(&req.operator, &req.input).await?;

        Ok(Response::new(ExecuteResponse {
            output: output_bytes,
        }))
    }

    type ExecuteStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecuteEvent, Status>>;

    async fn execute_stream(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStreamStream>, Status> {
        let req = request.into_inner();

        self.validate_session_key(&req.session_key)?;
        let input = self.deserialize_input(&req.input)?;
        let operator = self.resolve_operator(&req.operator)?;

        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::task::spawn(async move {
            // Log that execution has started.
            let started_event = ExecuteEvent {
                event: Some(execute_event::Event::LogLine(
                    b"operator started".to_vec(),
                )),
            };
            if tx.send(Ok(started_event)).await.is_err() {
                return; // receiver dropped
            }

            // Execute the operator, catching panics via the spawned task boundary.
            let result = tokio::task::spawn(async move { operator.execute(input, &Capabilities::none()).await }).await;

            match result {
                Ok(Ok(output)) => {
                    let output_bytes = match serde_json::to_vec(&output) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            let _ = tx
                                .send(Err(Status::internal(format!(
                                    "failed to serialize operator output: {e}"
                                ))))
                                .await;
                            return;
                        }
                    };
                    let final_event = ExecuteEvent {
                        event: Some(execute_event::Event::FinalOutput(ExecuteResponse {
                            output: output_bytes,
                        })),
                    };
                    let _ = tx.send(Ok(final_event)).await;
                }
                Ok(Err(op_err)) => {
                    error!("operator error during stream: {op_err}");
                    let _ = tx
                        .send(Err(Status::internal("operator execution failed")))
                        .await;
                }
                Err(join_err) => {
                    error!("operator panicked during stream: {join_err}");
                    let _ = tx
                        .send(Err(Status::internal("operator execution failed")))
                        .await;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            ready: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skg_runner=info".into()),
        )
        .init();

    // Session key is required — container must be started with SKG_SESSION_KEY set.
    let session_key = std::env::var("SKG_SESSION_KEY").unwrap_or_else(|_| {
        eprintln!("FATAL: SKG_SESSION_KEY environment variable is required");
        std::process::exit(1);
    });

    // Build operator registry. Empty by default — downstream image builders
    // will compile their operators into the binary.
    let registry = Arc::new(OperatorRegistry::builder().build());

    let grpc_addr: std::net::SocketAddr = format!("0.0.0.0:{GRPC_PORT}").parse()?;
    let http_addr: std::net::SocketAddr = format!("0.0.0.0:{HTTP_PORT}").parse()?;

    info!("starting skg-runner grpc={grpc_addr} http={http_addr}");

    let runner = Arc::new(RunnerServiceImpl::new(registry, session_key));

    // Spawn HTTP server (healthcheck + JSON API).
    let http_runner = Arc::clone(&runner);
    let http_server = tokio::spawn(async move {
        let app = http_adapter::router(http_runner);
        let listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
    });

    // Run gRPC server.
    TonicServer::builder()
        .add_service(RunnerServer::from_arc(runner))
        .serve_with_shutdown(grpc_addr, shutdown_signal())
        .await?;

    http_server.await?;

    info!("skg-runner shut down");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { info!("received CTRL+C, shutting down"); }
        _ = terminate => { info!("received SIGTERM, shutting down"); }
    }
}