//! `skg-runner` — Universal operator runner binary.
//!
//! Runs inside a Docker container and exposes:
//! - A gRPC server (tonic) implementing the Runner service on port 50051
//! - An HTTP healthcheck endpoint (axum) on port 8080 for Docker HEALTHCHECK
//!
//! The runner authenticates requests via `session_key`, validates the
//! `EnvironmentSpec`, loads the requested operator from a compiled-in
//! registry, executes it, and returns the result.

use axum::{routing::get, Router};
use tokio::signal;
use tonic::transport::Server as TonicServer;
use tonic::{Request, Response, Status};
use tracing::info;

mod proto {
    tonic::include_proto!("skg.runner.v1");
}

use proto::runner_server::{Runner, RunnerServer};
use proto::{
    ExecuteEvent, ExecuteRequest, ExecuteResponse, HealthRequest, HealthResponse,
};

/// gRPC port the runner listens on inside the container.
const GRPC_PORT: u16 = 50051;
/// HTTP port for Docker healthcheck.
const HTTP_PORT: u16 = 8080;

/// Core runner service implementation.
///
/// In v1, operators are compiled into the binary and selected by id
/// from the `ExecuteRequest.operator` field.
struct RunnerServiceImpl;

#[tonic::async_trait]
impl Runner for RunnerServiceImpl {
    async fn execute(
        &self,
        _request: Request<ExecuteRequest>,
    ) -> Result<Response<ExecuteResponse>, Status> {
        // 1) Validate session_key
        // 2) Deserialize OperatorInput from request.input
        // 3) Validate spec compatibility
        // 4) Look up operator by id in compiled-in registry
        // 5) Execute operator
        // 6) Serialize OperatorOutput into response.output
        todo!("RunnerServiceImpl::execute — implement operator dispatch")
    }

    type ExecuteStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecuteEvent, Status>>;

    async fn execute_stream(
        &self,
        _request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStreamStream>, Status> {
        todo!("RunnerServiceImpl::execute_stream — implement streaming execution")
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

/// HTTP healthcheck handler for Docker HEALTHCHECK.
async fn health_handler() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skg_runner=info".into()),
        )
        .init();

    let grpc_addr: std::net::SocketAddr = format!("0.0.0.0:{GRPC_PORT}").parse()?;
    let http_addr: std::net::SocketAddr = format!("0.0.0.0:{HTTP_PORT}").parse()?;

    info!("starting skg-runner grpc={grpc_addr} http={http_addr}");

    let runner = RunnerServiceImpl;

    // Spawn HTTP healthcheck server
    let http_server = tokio::spawn(async move {
        let app = Router::new().route("/health", get(health_handler));
        let listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
    });

    // Run gRPC server
    TonicServer::builder()
        .add_service(RunnerServer::new(runner))
        .serve_with_shutdown(grpc_addr, shutdown_signal())
        .await?;

    http_server.await?;

    info!("skg-runner shut down");
    Ok(())
}

async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
}
