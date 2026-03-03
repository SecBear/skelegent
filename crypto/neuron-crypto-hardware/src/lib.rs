#![deny(missing_docs)]
//! Stub crypto provider for PKCS#11 / YubiKey PIV hardware tokens.
//!
//! This crate provides the correct trait impl shape for a hardware crypto provider.
//! The actual PKCS#11/YubiKey integration is not implemented — all operations return
//! `CryptoError::UnsupportedOperation`.

use async_trait::async_trait;
use neuron_crypto::{CryptoError, CryptoProvider};

/// Stub for PKCS#11 / YubiKey PIV hardware tokens.
pub struct HardwareProvider {
    _slot: String,
}

impl HardwareProvider {
    /// Create a new hardware provider (stub).
    pub fn new(slot: impl Into<String>) -> Self {
        Self { _slot: slot.into() }
    }
}

#[async_trait]
impl CryptoProvider for HardwareProvider {
    async fn sign(
        &self,
        _key_ref: &str,
        _algorithm: &str,
        _data: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "HardwareProvider is a stub".into(),
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
            "HardwareProvider is a stub".into(),
        ))
    }

    async fn encrypt(&self, _key_ref: &str, _plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "HardwareProvider is a stub".into(),
        ))
    }

    async fn decrypt(&self, _key_ref: &str, _ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedOperation(
            "HardwareProvider is a stub".into(),
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
        let _: Arc<dyn CryptoProvider> = Arc::new(HardwareProvider::new("9a"));
    }

    #[tokio::test]
    async fn sign_returns_stub_error() {
        let provider = HardwareProvider::new("9a");
        let err = provider
            .sign("key-1", "ed25519", b"data")
            .await
            .unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
        assert!(err.to_string().contains("stub"));
    }

    #[tokio::test]
    async fn verify_returns_stub_error() {
        let provider = HardwareProvider::new("9a");
        let err = provider
            .verify("key-1", "ed25519", b"data", b"sig")
            .await
            .unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }

    #[tokio::test]
    async fn encrypt_returns_stub_error() {
        let provider = HardwareProvider::new("9a");
        let err = provider.encrypt("key-1", b"plaintext").await.unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }

    #[tokio::test]
    async fn decrypt_returns_stub_error() {
        let provider = HardwareProvider::new("9a");
        let err = provider.decrypt("key-1", b"ciphertext").await.unwrap_err();
        assert!(matches!(err, CryptoError::UnsupportedOperation(_)));
    }
}
