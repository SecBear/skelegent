#![deny(missing_docs)]
//! In-memory implementation of layer0's StateStore trait.
//!
//! Uses a `HashMap` behind a `RwLock` for concurrent access.
//! Scopes are serialized to strings for use as key prefixes,
//! providing full scope isolation. Search always returns empty
//! (no semantic search support in the in-memory backend).

use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::state::{SearchResult, StateStore};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// In-memory state store backed by a `HashMap` behind a `RwLock`.
///
/// Suitable for testing, prototyping, and single-process use cases
/// where persistence across restarts is not required.
pub struct MemoryStore {
    data: RwLock<HashMap<String, serde_json::Value>>,
}

impl MemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a composite key from scope + key to ensure isolation.
fn composite_key(scope: &Scope, key: &str) -> String {
    let scope_str = serde_json::to_string(scope).unwrap_or_else(|_| "unknown".to_string());
    format!("{scope_str}\0{key}")
}

/// Extract the user-facing key from a composite key, if it belongs to the given scope.
fn extract_key<'a>(composite: &'a str, scope_prefix: &str) -> Option<&'a str> {
    composite
        .strip_prefix(scope_prefix)
        .and_then(|rest| rest.strip_prefix('\0'))
}

#[async_trait]
impl StateStore for MemoryStore {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let ck = composite_key(scope, key);
        let data = self.data.read().await;
        Ok(data.get(&ck).cloned())
    }

    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        let ck = composite_key(scope, key);
        let mut data = self.data.write().await;
        data.insert(ck, value);
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let ck = composite_key(scope, key);
        let mut data = self.data.write().await;
        data.remove(&ck);
        Ok(())
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        let scope_prefix = serde_json::to_string(scope).unwrap_or_else(|_| "unknown".to_string());
        let data = self.data.read().await;
        let keys: Vec<String> = data
            .keys()
            .filter_map(|ck| {
                extract_key(ck, &scope_prefix).and_then(|k| {
                    if k.starts_with(prefix) {
                        Some(k.to_string())
                    } else {
                        None
                    }
                })
            })
            .collect();
        Ok(keys)
    }

    async fn search(
        &self,
        _scope: &Scope,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        // In-memory store does not support semantic search.
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn write_and_read() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "key1", json!("value1")).await.unwrap();
        let val = store.read(&scope, "key1").await.unwrap();
        assert_eq!(val, Some(json!("value1")));
    }

    #[tokio::test]
    async fn read_nonexistent_returns_none() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        let val = store.read(&scope, "missing").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn write_overwrites_existing() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "key1", json!("first")).await.unwrap();
        store.write(&scope, "key1", json!("second")).await.unwrap();
        let val = store.read(&scope, "key1").await.unwrap();
        assert_eq!(val, Some(json!("second")));
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "key1", json!("value1")).await.unwrap();
        store.delete(&scope, "key1").await.unwrap();
        let val = store.read(&scope, "key1").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        let result = store.delete(&scope, "missing").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_keys_with_prefix() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store
            .write(&scope, "user:name", json!("Alice"))
            .await
            .unwrap();
        store.write(&scope, "user:age", json!(30)).await.unwrap();
        store
            .write(&scope, "system:version", json!("1.0"))
            .await
            .unwrap();

        let mut keys = store.list(&scope, "user:").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["user:age", "user:name"]);
    }

    #[tokio::test]
    async fn list_empty_prefix_returns_all() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "a", json!(1)).await.unwrap();
        store.write(&scope, "b", json!(2)).await.unwrap();

        let keys = store.list(&scope, "").await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn scopes_are_isolated() {
        let store = MemoryStore::new();
        let global = Scope::Global;
        let session = Scope::Session(layer0::SessionId::new("s1"));

        store
            .write(&global, "key", json!("global_val"))
            .await
            .unwrap();
        store
            .write(&session, "key", json!("session_val"))
            .await
            .unwrap();

        let global_val = store.read(&global, "key").await.unwrap();
        let session_val = store.read(&session, "key").await.unwrap();

        assert_eq!(global_val, Some(json!("global_val")));
        assert_eq!(session_val, Some(json!("session_val")));
    }

    #[tokio::test]
    async fn search_returns_empty() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        let results = store.search(&scope, "query", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn default_store_is_empty() {
        let store = MemoryStore::default();
        let _ = store; // Just verify it constructs
    }

    #[test]
    fn memory_store_implements_state_store() {
        fn _assert_state_store<T: StateStore>() {}
        _assert_state_store::<MemoryStore>();
    }
}
