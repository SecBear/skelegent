//! Push notification configuration storage for the A2A protocol.
//!
//! Defines the [`PushNotificationConfig`] type, the [`PushNotificationStore`]
//! trait for backend-agnostic CRUD, and [`InMemoryPushStore`] — a thread-safe
//! in-memory implementation suitable for dev/test.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned by [`PushNotificationStore`] operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PushStoreError {
    /// An internal / implementation-specific error.
    #[error("{0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for push notifications on a specific task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushNotificationConfig {
    /// The task ID this config applies to.
    pub task_id: String,
    /// Webhook URL to POST updates to.
    pub url: String,
    /// Optional authentication token included in webhook requests.
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// Store trait
// ---------------------------------------------------------------------------

/// Backend-agnostic storage for push notification registrations.
#[async_trait]
pub trait PushNotificationStore: Send + Sync {
    /// Register or update a push notification config for a task.
    ///
    /// If a config already exists for the given `task_id`, it is overwritten
    /// (upsert semantics).
    async fn set(&self, config: PushNotificationConfig) -> Result<(), PushStoreError>;

    /// Get the push notification config for a task.
    ///
    /// Returns `None` when no config has been registered for `task_id`.
    async fn get(&self, task_id: &str) -> Result<Option<PushNotificationConfig>, PushStoreError>;

    /// Delete the push notification config for a task.
    ///
    /// Returns `true` if a config existed and was removed, `false` otherwise.
    async fn delete(&self, task_id: &str) -> Result<bool, PushStoreError>;

    /// List all push notification configs.
    ///
    /// Returns an empty `Vec` when no configs exist.
    async fn list(&self) -> Result<Vec<PushNotificationConfig>, PushStoreError>;
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

/// Thread-safe, in-memory [`PushNotificationStore`].
///
/// Uses [`std::sync::RwLock`] internally — adequate for dev, test, and
/// single-node deployments. For distributed setups, implement
/// [`PushNotificationStore`] against a shared data store.
#[derive(Debug, Default)]
pub struct InMemoryPushStore {
    configs: RwLock<HashMap<String, PushNotificationConfig>>,
}

impl InMemoryPushStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PushNotificationStore for InMemoryPushStore {
    async fn set(&self, config: PushNotificationConfig) -> Result<(), PushStoreError> {
        let mut map = self
            .configs
            .write()
            .map_err(|e| PushStoreError::Internal(e.to_string()))?;
        map.insert(config.task_id.clone(), config);
        Ok(())
    }

    async fn get(&self, task_id: &str) -> Result<Option<PushNotificationConfig>, PushStoreError> {
        let map = self
            .configs
            .read()
            .map_err(|e| PushStoreError::Internal(e.to_string()))?;
        Ok(map.get(task_id).cloned())
    }

    async fn delete(&self, task_id: &str) -> Result<bool, PushStoreError> {
        let mut map = self
            .configs
            .write()
            .map_err(|e| PushStoreError::Internal(e.to_string()))?;
        Ok(map.remove(task_id).is_some())
    }

    async fn list(&self) -> Result<Vec<PushNotificationConfig>, PushStoreError> {
        let map = self
            .configs
            .read()
            .map_err(|e| PushStoreError::Internal(e.to_string()))?;
        Ok(map.values().cloned().collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> InMemoryPushStore {
        InMemoryPushStore::new()
    }

    fn config(task_id: &str, url: &str) -> PushNotificationConfig {
        PushNotificationConfig {
            task_id: task_id.to_string(),
            url: url.to_string(),
            token: None,
        }
    }

    #[tokio::test]
    async fn set_and_get() {
        let s = store();
        s.set(config("t1", "https://example.com/hook"))
            .await
            .unwrap();
        let got = s.get("t1").await.unwrap().expect("should exist");
        assert_eq!(got.task_id, "t1");
        assert_eq!(got.url, "https://example.com/hook");
        assert!(got.token.is_none());
    }

    #[tokio::test]
    async fn set_overwrites_existing() {
        let s = store();
        s.set(config("t1", "https://old.com")).await.unwrap();
        s.set(config("t1", "https://new.com")).await.unwrap();
        let got = s.get("t1").await.unwrap().expect("should exist");
        assert_eq!(got.url, "https://new.com");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let s = store();
        assert!(s.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let s = store();
        s.set(config("t1", "https://x.com")).await.unwrap();
        assert!(s.delete("t1").await.unwrap());
        assert!(s.get("t1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_false() {
        let s = store();
        assert!(!s.delete("nope").await.unwrap());
    }

    #[tokio::test]
    async fn list_empty() {
        let s = store();
        assert!(s.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_returns_all() {
        let s = store();
        s.set(config("t1", "https://a.com")).await.unwrap();
        s.set(config("t2", "https://b.com")).await.unwrap();
        let mut ids: Vec<String> = s
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.task_id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["t1", "t2"]);
    }
}
