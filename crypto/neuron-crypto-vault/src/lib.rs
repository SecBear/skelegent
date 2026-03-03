#![deny(missing_docs)]
//! Stub crypto provider for Vault Transit engine.
//!
//! This crate provides the correct trait impl shape for a Vault Transit crypto provider.
//! The actual Vault Transit integration is not implemented — all operations return
//! `CryptoError::UnsupportedOperation`.

use async_trait::async_trait;
use neuron_crypto::{CryptoError, CryptoProvider};

/// Stub for Vault Transit engine.
pub struct VaultTransitProvider {
    _addr: String,
}

impl VaultTransitProvider {
    /// Create a new Vault Transit provider (stub).
    pub fn new(addr: impl Into<String>) -> Self {
        Self { _addr: addr.into() }
    }
}

#[async_trait]
impl CryptoProvider for VaultTransitProvider {
    async fn sign(
        &self,
        _key_ref: &str,
        _algorithm: &str,
        _data: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "VaultTransitProvider is a stub".into(),
        ))
    }

    async fn verify(
        &self,
        _key_ref: &str,
        _algorithm: &str,
        _data: &[u8],
        _signature: &[u8],
    ) -> Result<bool, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "VaultTransitProvider is a stub".into(),
        ))
    }

    async fn encrypt(&self, _key_ref: &str, _plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "VaultTransitProvider is a stub".into(),
        ))
    }

    async fn decrypt(&self, _key_ref: &str, _ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "VaultTransitProvider is a stub".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn object_safety() {
        _assert_send_sync::<Box<dyn CryptoProvider>>();
        _assert_send_sync::<Arc<dyn CryptoProvider>>();
        let _: Arc<dyn CryptoProvider> = Arc::new(VaultTransitProvider::new("https://vault:8200"));
    }

    #[tokio::test]
    async fn sign_returns_stub_error() {
        let provider = VaultTransitProvider::new("https://vault:8200");
        let err = provider
            .sign("key-1", "ed25519", b"data")
            .await
            .unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn verify_returns_stub_error() {
        let provider = VaultTransitProvider::new("https://vault:8200");
        let err = provider
            .verify("key-1", "ed25519", b"data", b"sig")
            .await
            .unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }

    #[tokio::test]
    async fn encrypt_returns_stub_error() {
        let provider = VaultTransitProvider::new("https://vault:8200");
        let err = provider.encrypt("key-1", b"plaintext").await.unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }

    #[tokio::test]
    async fn decrypt_returns_stub_error() {
        let provider = VaultTransitProvider::new("https://vault:8200");
        let err = provider.decrypt("key-1", b"ciphertext").await.unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }
}
