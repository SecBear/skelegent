#![deny(missing_docs)]
//! gRPC proxy for [`layer0::state::StateStore`].
//!
//! Provides two halves of a bridge for cross-process (typically cross-container)
//! state access:
//!
//! - [`StateStoreProxyServer`] — runs on the host, wraps an `Arc<dyn StateStore>`
//!   and exposes it via gRPC.
//! - [`RemoteStateStore`] — runs inside a container, implements `StateStore` by
//!   forwarding every call over gRPC to a `StateStoreProxyServer`.

use std::sync::Arc;

use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::state::{SearchResult, StateStore};
use tonic::transport::Channel;
use tonic::{Request, Response, Status};

/// Generated protobuf/gRPC types.
#[allow(missing_docs)]
pub mod proto {
    tonic::include_proto!("skg.state_proxy.v1");
}

use proto::state_store_proxy_client::StateStoreProxyClient;
use proto::state_store_proxy_server::{StateStoreProxy, StateStoreProxyServer as TonicServer};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Metadata key for session-based auth.
const SESSION_KEY_HEADER: &str = "x-session-key";

/// Serialize a [`Scope`] to its JSON string representation for the proto
/// `scope` field. This mirrors how `skg-state-memory` round-trips scopes.
fn scope_to_string(scope: &Scope) -> String {
    serde_json::to_string(scope).unwrap_or_else(|_| "\"unknown\"".to_string())
}

/// Deserialize a scope string from the proto back into a [`Scope`].
fn scope_from_string(s: &str) -> Result<Scope, Status> {
    serde_json::from_str(s)
        .map_err(|e| Status::invalid_argument(format!("invalid scope: {e}")))
}

/// Map a [`StateError`] to a [`tonic::Status`].
fn state_err_to_status(e: StateError) -> Status {
    match &e {
        StateError::NotFound { .. } => Status::not_found(e.to_string()),
        StateError::WriteFailed(_) => Status::internal(e.to_string()),
        StateError::Serialization(_) => Status::invalid_argument(e.to_string()),
        StateError::Other(_) => Status::internal(e.to_string()),
        // non_exhaustive — treat unknowns as internal errors
        _ => Status::internal(e.to_string()),
    }
}

/// Map a [`tonic::Status`] to a [`StateError`].
fn status_to_state_err(s: Status) -> StateError {
    match s.code() {
        tonic::Code::NotFound => StateError::NotFound {
            scope: String::new(),
            key: s.message().to_string(),
        },
        tonic::Code::InvalidArgument => StateError::Serialization(s.message().to_string()),
        _ => StateError::Other(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            s.message().to_string(),
        ))),
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// gRPC server that wraps an `Arc<dyn StateStore>` and serves the
/// `StateStoreProxy` proto service. Runs on the host side.
pub struct StateStoreProxyServer {
    store: Arc<dyn StateStore>,
    session_key: String,
}

impl StateStoreProxyServer {
    /// Create a new proxy server.
    pub fn new(store: Arc<dyn StateStore>, session_key: String) -> Self {
        Self { store, session_key }
    }

    /// Convert into a tonic gRPC service, ready for `Server::add_service`.
    pub fn into_service(self) -> TonicServer<Self> {
        TonicServer::new(self)
    }

    /// Validate the session key from request metadata.
    fn check_auth<T>(&self, req: &Request<T>) -> Result<(), Status> {
        let provided = req
            .metadata()
            .get(SESSION_KEY_HEADER)
            .and_then(|v| v.to_str().ok());
        match provided {
            Some(k) if k == self.session_key => Ok(()),
            _ => Err(Status::unauthenticated("invalid or missing session key")),
        }
    }
}

#[tonic::async_trait]
impl StateStoreProxy for StateStoreProxyServer {
    async fn read(
        &self,
        request: Request<proto::ReadRequest>,
    ) -> Result<Response<proto::ReadResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let scope = scope_from_string(&req.scope)?;
        let val = self.store.read(&scope, &req.key).await.map_err(state_err_to_status)?;
        let value = val.map(|v| serde_json::to_vec(&v).unwrap_or_default());
        Ok(Response::new(proto::ReadResponse { value }))
    }

    async fn write(
        &self,
        request: Request<proto::WriteRequest>,
    ) -> Result<Response<proto::WriteResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let scope = scope_from_string(&req.scope)?;
        let value: serde_json::Value = serde_json::from_slice(&req.value)
            .map_err(|e| Status::invalid_argument(format!("invalid value bytes: {e}")))?;
        self.store
            .write(&scope, &req.key, value)
            .await
            .map_err(state_err_to_status)?;
        Ok(Response::new(proto::WriteResponse {}))
    }

    async fn delete(
        &self,
        request: Request<proto::DeleteRequest>,
    ) -> Result<Response<proto::DeleteResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let scope = scope_from_string(&req.scope)?;
        // The StateStore::delete trait returns () and is a no-op for missing keys.
        // We probe existence first so we can populate the proto `existed` field.
        let existed = self
            .store
            .read(&scope, &req.key)
            .await
            .map_err(state_err_to_status)?
            .is_some();
        self.store
            .delete(&scope, &req.key)
            .await
            .map_err(state_err_to_status)?;
        Ok(Response::new(proto::DeleteResponse { existed }))
    }

    async fn list(
        &self,
        request: Request<proto::ListRequest>,
    ) -> Result<Response<proto::ListResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let scope = scope_from_string(&req.scope)?;
        let prefix = req.prefix.as_deref().unwrap_or("");
        let keys = self
            .store
            .list(&scope, prefix)
            .await
            .map_err(state_err_to_status)?;
        Ok(Response::new(proto::ListResponse { keys }))
    }

    async fn search(
        &self,
        request: Request<proto::SearchRequest>,
    ) -> Result<Response<proto::SearchResponse>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let scope = scope_from_string(&req.scope)?;
        let results = self
            .store
            .search(&scope, &req.query, req.limit as usize)
            .await
            .map_err(state_err_to_status)?;
        let hits = results
            .into_iter()
            .map(|r| {
                let value = serde_json::to_vec(
                    &serde_json::Value::String(r.snippet.unwrap_or_default()),
                )
                .unwrap_or_default();
                proto::SearchHit {
                    key: r.key,
                    value,
                    score: r.score as f32,
                }
            })
            .collect();
        Ok(Response::new(proto::SearchResponse { hits }))
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Client-side `StateStore` implementation that forwards all calls over gRPC
/// to a [`StateStoreProxyServer`]. Typically runs inside a Docker container.
pub struct RemoteStateStore {
    client: StateStoreProxyClient<Channel>,
    session_key: String,
}

impl RemoteStateStore {
    /// Connect to a remote StateStoreProxy gRPC server.
    pub async fn connect(endpoint: &str, session_key: String) -> Result<Self, StateError> {
        let client = StateStoreProxyClient::connect(endpoint.to_string())
            .await
            .map_err(|e| {
                StateError::Other(Box::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("failed to connect to state proxy: {e}"),
                )))
            })?;
        Ok(Self {
            client,
            session_key,
        })
    }

    /// Attach the session key as gRPC metadata to a request.
    fn authed<T>(&self, msg: T) -> Request<T> {
        let mut req = Request::new(msg);
        req.metadata_mut().insert(
            SESSION_KEY_HEADER,
            self.session_key.parse().expect("session key must be ASCII"),
        );
        req
    }
}

#[async_trait]
impl StateStore for RemoteStateStore {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let req = self.authed(proto::ReadRequest {
            scope: scope_to_string(scope),
            key: key.to_string(),
        });
        let resp = self
            .client
            .clone()
            .read(req)
            .await
            .map_err(status_to_state_err)?
            .into_inner();
        match resp.value {
            Some(bytes) => {
                let val = serde_json::from_slice(&bytes)
                    .map_err(|e| StateError::Serialization(e.to_string()))?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        let value_bytes =
            serde_json::to_vec(&value).map_err(|e| StateError::Serialization(e.to_string()))?;
        let req = self.authed(proto::WriteRequest {
            scope: scope_to_string(scope),
            key: key.to_string(),
            value: value_bytes,
        });
        self.client
            .clone()
            .write(req)
            .await
            .map_err(status_to_state_err)?;
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let req = self.authed(proto::DeleteRequest {
            scope: scope_to_string(scope),
            key: key.to_string(),
        });
        self.client
            .clone()
            .delete(req)
            .await
            .map_err(status_to_state_err)?;
        Ok(())
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        let req = self.authed(proto::ListRequest {
            scope: scope_to_string(scope),
            prefix: if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            },
        });
        let resp = self
            .client
            .clone()
            .list(req)
            .await
            .map_err(status_to_state_err)?
            .into_inner();
        Ok(resp.keys)
    }

    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        let req = self.authed(proto::SearchRequest {
            scope: scope_to_string(scope),
            query: query.to_string(),
            limit: limit as u32,
        });
        let resp = self
            .client
            .clone()
            .search(req)
            .await
            .map_err(status_to_state_err)?
            .into_inner();
        let results = resp
            .hits
            .into_iter()
            .map(|h| {
                let snippet: Option<String> = serde_json::from_slice(&h.value)
                    .ok()
                    .and_then(|v: serde_json::Value| v.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty());
                let mut result = SearchResult::new(h.key, h.score as f64);
                result.snippet = snippet;
                result
            })
            .collect();
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Start a proxy server on a random port and return the endpoint URL.
    async fn start_server(store: Arc<dyn StateStore>, session_key: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = StateStoreProxyServer::new(store, session_key.to_string());
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(server.into_service())
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn round_trip_write_read() {
        let store = Arc::new(skg_state_memory::MemoryStore::new());
        let key = "test-session-key";
        let endpoint = start_server(store.clone(), key).await;
        let client = RemoteStateStore::connect(&endpoint, key.to_string())
            .await
            .unwrap();

        let scope = Scope::Global;
        let value = json!({"hello": "world"});

        // Write through proxy
        client.write(&scope, "k1", value.clone()).await.unwrap();

        // Read back through proxy
        let got = client.read(&scope, "k1").await.unwrap();
        assert_eq!(got, Some(value));

        // Read missing key
        let missing = client.read(&scope, "nope").await.unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let store = Arc::new(skg_state_memory::MemoryStore::new());
        let key = "sess";
        let endpoint = start_server(store.clone(), key).await;
        let client = RemoteStateStore::connect(&endpoint, key.to_string())
            .await
            .unwrap();

        let scope = Scope::Global;
        client
            .write(&scope, "del-me", json!("value"))
            .await
            .unwrap();
        assert!(client.read(&scope, "del-me").await.unwrap().is_some());

        client.delete(&scope, "del-me").await.unwrap();
        assert!(client.read(&scope, "del-me").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_with_prefix() {
        let store = Arc::new(skg_state_memory::MemoryStore::new());
        let key = "sess";
        let endpoint = start_server(store.clone(), key).await;
        let client = RemoteStateStore::connect(&endpoint, key.to_string())
            .await
            .unwrap();

        let scope = Scope::Global;
        client.write(&scope, "pfx/a", json!(1)).await.unwrap();
        client.write(&scope, "pfx/b", json!(2)).await.unwrap();
        client.write(&scope, "other", json!(3)).await.unwrap();

        let mut keys = client.list(&scope, "pfx/").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["pfx/a", "pfx/b"]);
    }

    #[tokio::test]
    async fn search_returns_no_error() {
        let store = Arc::new(skg_state_memory::MemoryStore::new());
        let key = "sess";
        let endpoint = start_server(store.clone(), key).await;
        let client = RemoteStateStore::connect(&endpoint, key.to_string())
            .await
            .unwrap();

        let scope = Scope::Global;
        // MemoryStore does support basic substring search; just verify no error.
        let results = client.search(&scope, "anything", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn auth_rejected_on_bad_key() {
        let store = Arc::new(skg_state_memory::MemoryStore::new());
        let endpoint = start_server(store.clone(), "correct-key").await;
        let client = RemoteStateStore::connect(&endpoint, "wrong-key".to_string())
            .await
            .unwrap();

        let result = client.read(&Scope::Global, "k").await;
        assert!(result.is_err());
    }
}
