#![deny(missing_docs)]
//! File-based auth provider that reads a bearer token from disk.
//!
//! Reads the entire file contents as the token bytes. Useful for Kubernetes
//! projected service account tokens or other file-mounted credentials.

use async_trait::async_trait;
use neuron_auth::{AuthError, AuthProvider, AuthRequest, AuthToken};
use std::path::PathBuf;

/// Reads a bearer token from a file (e.g., K8s projected service account token).
pub struct FileTokenProvider {
    path: PathBuf,
}

impl FileTokenProvider {
    /// Create with the path to the token file.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl AuthProvider for FileTokenProvider {
    async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
        let bytes = tokio::fs::read(&self.path)
            .await
            .map_err(|e| AuthError::BackendError(format!("failed to read token file: {e}")))?;
        Ok(AuthToken::permanent(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn object_safety() {
        _assert_send_sync::<Box<dyn AuthProvider>>();
        _assert_send_sync::<Arc<dyn AuthProvider>>();
        let _: Arc<dyn AuthProvider> = Arc::new(FileTokenProvider::new("/tmp/token"));
    }

    #[tokio::test]
    async fn reads_token_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, b"file-based-token").unwrap();
        let provider = FileTokenProvider::new(&path);
        let token = provider.provide(&AuthRequest::new()).await.unwrap();
        token.with_bytes(|b| assert_eq!(b, b"file-based-token"));
    }

    #[tokio::test]
    async fn returns_error_for_missing_file() {
        let provider = FileTokenProvider::new("/tmp/nonexistent-neuron-test-token-file");
        let err = provider.provide(&AuthRequest::new()).await.unwrap_err();
        assert!(matches!(err, AuthError::BackendError(_)));
        assert!(err.to_string().contains("failed to read token file"));
    }
}
