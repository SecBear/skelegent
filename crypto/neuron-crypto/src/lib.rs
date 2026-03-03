#![deny(missing_docs)]
//! Cryptographic operations for neuron.
//!
//! This crate defines the [`CryptoProvider`] trait for cryptographic operations
//! where private keys never leave the provider boundary (Vault Transit, PKCS#11,
//! HSM, YubiKey, KMS).
//!
//! The consumer sends data in and gets results out. Private keys are never
//! exposed — this is the fundamental security property of hardware security
//! modules and transit encryption engines.

use async_trait::async_trait;
use thiserror::Error;

/// Errors from cryptographic operations (crate-local, not in layer0).
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CryptoError {
    /// The referenced key was not found.
    #[error("key not found: {0}")]
    KeyNotFound(String),

    /// The operation is not supported for this key type or algorithm.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// The cryptographic operation failed.
    #[error("crypto operation failed: {0}")]
    OperationFailed(String),

    /// Catch-all.
    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Cryptographic operations where private keys never leave the provider boundary.
///
/// Implementations:
/// - `VaultTransitProvider`: Vault Transit engine (encrypt, decrypt, sign, verify)
/// - `HardwareProvider`: PKCS#11 / YubiKey PIV (sign, decrypt with on-device keys)
/// - `KmsProvider`: AWS KMS / GCP KMS / Azure Key Vault (envelope encryption)
///
/// The `key_ref` parameter is an opaque identifier meaningful to the implementation:
/// - Vault Transit: the key name in the transit engine
/// - PKCS#11: slot + key label
/// - YubiKey: PIV slot (e.g., "9a", "9c")
/// - KMS: key ARN or resource ID
#[async_trait]
pub trait CryptoProvider: Send + Sync {
    /// Sign data with the referenced key.
    async fn sign(
        &self,
        key_ref: &str,
        algorithm: &str,
        data: &[u8],
    ) -> Result<Vec<u8>, CryptoError>;

    /// Verify a signature against the referenced key.
    async fn verify(
        &self,
        key_ref: &str,
        algorithm: &str,
        data: &[u8],
        signature: &[u8],
    ) -> Result<bool, CryptoError>;

    /// Encrypt data with the referenced key.
    async fn encrypt(&self, key_ref: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError>;

    /// Decrypt data with the referenced key.
    async fn decrypt(&self, key_ref: &str, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn crypto_provider_is_object_safe_send_sync() {
        _assert_send_sync::<Box<dyn CryptoProvider>>();
        _assert_send_sync::<Arc<dyn CryptoProvider>>();
    }

    struct NoopCryptoProvider;

    #[async_trait]
    impl CryptoProvider for NoopCryptoProvider {
        async fn sign(
            &self,
            _key_ref: &str,
            _algorithm: &str,
            data: &[u8],
        ) -> Result<Vec<u8>, CryptoError> {
            // Stub: return data as "signature"
            Ok(data.to_vec())
        }

        async fn verify(
            &self,
            _key_ref: &str,
            _algorithm: &str,
            data: &[u8],
            signature: &[u8],
        ) -> Result<bool, CryptoError> {
            Ok(data == signature)
        }

        async fn encrypt(&self, _key_ref: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
            // Stub: return plaintext as "ciphertext"
            Ok(plaintext.to_vec())
        }

        async fn decrypt(&self, _key_ref: &str, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
            Ok(ciphertext.to_vec())
        }
    }

    #[tokio::test]
    async fn noop_provider_sign_verify_roundtrip() {
        let provider = NoopCryptoProvider;
        let data = b"hello world";
        let sig = provider.sign("key-1", "ed25519", data).await.unwrap();
        let valid = provider
            .verify("key-1", "ed25519", data, &sig)
            .await
            .unwrap();
        assert!(valid);
    }

    #[tokio::test]
    async fn noop_provider_encrypt_decrypt_roundtrip() {
        let provider = NoopCryptoProvider;
        let plaintext = b"secret message";
        let ciphertext = provider.encrypt("key-1", plaintext).await.unwrap();
        let decrypted = provider.decrypt("key-1", &ciphertext).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn crypto_error_display_all_variants() {
        assert_eq!(
            CryptoError::KeyNotFound("transit/my-key".into()).to_string(),
            "key not found: transit/my-key"
        );
        assert_eq!(
            CryptoError::UnsupportedOperation("rsa-4096".into()).to_string(),
            "unsupported operation: rsa-4096"
        );
        assert_eq!(
            CryptoError::OperationFailed("invalid ciphertext".into()).to_string(),
            "crypto operation failed: invalid ciphertext"
        );
    }
}
