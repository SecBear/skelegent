#![deny(missing_docs)]
//! In-memory implementation of layer0's StateStore trait.
//!
//! Uses a `HashMap` behind a `RwLock` for concurrent access.
//! Scopes are serialized to strings for use as key prefixes,
//! providing full scope isolation. Supports optional LRU eviction
//! via [`MemoryStore::bounded`] and basic case-insensitive substring search.

use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::state::{SearchResult, StateStore, StoreOptions};
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

/// In-memory state store backed by a `HashMap` behind a `RwLock`.
///
/// Suitable for testing, prototyping, and single-process use cases
/// where persistence across restarts is not required.
///
/// Create an unbounded store with [`MemoryStore::new`] or a capacity-limited
/// LRU store with [`MemoryStore::bounded`].
pub struct MemoryStore {
    data: RwLock<HashMap<String, serde_json::Value>>,
    transient: RwLock<HashMap<String, serde_json::Value>>,
    capacity: Option<usize>,
    /// Composite keys ordered by last access, least-recently used at front.
    access_order: RwLock<Vec<String>>,
    /// Composite keys marked durable — never evicted by LRU.
    durable_keys: RwLock<HashSet<String>>,
}

impl MemoryStore {
    /// Create a new empty in-memory store with no eviction limit.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            transient: RwLock::new(HashMap::new()),
            capacity: None,
            access_order: RwLock::new(Vec::new()),
            durable_keys: RwLock::new(HashSet::new()),
        }
    }

    /// Create a bounded in-memory store that evicts least-recently-used entries
    /// when the entry count exceeds `capacity`.
    ///
    /// Reads and writes both count as "use" for LRU ordering.
    /// Pinned scope entries (written via `write_hinted` with `Lifetime::Durable`)
    /// are never evicted.
    pub fn bounded(capacity: usize) -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            transient: RwLock::new(HashMap::new()),
            capacity: Some(capacity),
            access_order: RwLock::new(Vec::new()),
            durable_keys: RwLock::new(HashSet::new()),
        }
    }

    /// Insert or update `ck` in the data map, updating LRU tracking and evicting
    /// if the store is bounded and over capacity.
    ///
    /// `is_durable` marks the key as non-evictable. Transient entries bypass this
    /// path entirely (they go to the separate `transient` map).
    ///
    /// Lock order: `data` → `access_order` → `durable_keys`.
    async fn write_inner(&self, ck: String, value: serde_json::Value, is_durable: bool) {
        let mut data = self.data.write().await;
        let mut order = self.access_order.write().await;
        let mut durable = self.durable_keys.write().await;

        if is_durable {
            durable.insert(ck.clone());
        }

        // Remove any existing position, then push to back (most-recently used).
        order.retain(|k| k != &ck);
        order.push(ck.clone());
        data.insert(ck, value);

        // Evict least-recently-used non-durable entries until within capacity.
        if let Some(cap) = self.capacity {
            while data.len() > cap {
                // Find the front-most key that is not durable.
                let evict_idx = order.iter().position(|k| !durable.contains(k));
                match evict_idx {
                    Some(idx) => {
                        let evict_ck = order.remove(idx);
                        data.remove(&evict_ck);
                    }
                    // All remaining keys are durable — cannot evict further.
                    None => break,
                }
            }
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
        // Drop the read lock before acquiring the write lock on access_order
        // to avoid holding two locks simultaneously (data.read + order.write).
        let value = self.data.read().await.get(&ck).cloned();
        if value.is_some() {
            let mut order = self.access_order.write().await;
            order.retain(|k| k != &ck);
            order.push(ck);
        }
        Ok(value)
    }

    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        let ck = composite_key(scope, key);
        self.write_inner(ck, value, false).await;
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let ck = composite_key(scope, key);
        self.data.write().await.remove(&ck);
        self.access_order.write().await.retain(|k| k != &ck);
        self.durable_keys.write().await.remove(&ck);
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
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        if query.is_empty() || limit == 0 {
            return Ok(vec![]);
        }

        let scope_prefix =
            serde_json::to_string(scope).unwrap_or_else(|_| "unknown".to_string());
        let query_lower = query.to_lowercase();

        let data = self.data.read().await;
        let mut results: Vec<SearchResult> = data
            .iter()
            .filter_map(|(ck, value)| {
                let key = extract_key(ck, &scope_prefix)?;
                let text = value.to_string();
                let text_lower = text.to_lowercase();

                let count = text_lower.matches(query_lower.as_str()).count();
                if count == 0 {
                    return None;
                }

                // Score: occurrence density — more occurrences in shorter text ranks higher.
                let score = count as f64 / text_lower.len().max(1) as f64;
                let mut result = SearchResult::new(key, score);
                result.snippet = Some(if text.len() > 200 {
                    format!("{}...", &text[..200])
                } else {
                    text
                });
                Some(result)
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    async fn write_hinted(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        options: &StoreOptions,
    ) -> Result<(), StateError> {
        use layer0::state::Lifetime;
        match options.lifetime {
            Some(Lifetime::Transient) => {
                let ck = composite_key(scope, key);
                self.transient.write().await.insert(ck, value);
            }
            Some(Lifetime::Durable) => {
                let ck = composite_key(scope, key);
                self.write_inner(ck, value, true).await;
            }
            _ => {
                self.write(scope, key, value).await?;
            }
        }
        Ok(())
    }

    fn clear_transient(&self) {
        // Use try_write; if the lock is contended, skip — best-effort clearing.
        if let Ok(mut t) = self.transient.try_write() {
            t.clear();
        }
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
    async fn search_returns_empty_on_no_match() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "k1", json!("hello world")).await.unwrap();
        let results = store.search(&scope, "xyzzy", 10).await.unwrap();
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

    #[tokio::test]
    async fn test_transient_write_not_durable() {
        use layer0::state::{Lifetime, StoreOptions};

        let store = MemoryStore::new();
        let scope = Scope::Global;

        // Write transient entry
        let opts = StoreOptions {
            lifetime: Some(Lifetime::Transient),
            ..Default::default()
        };
        store
            .write_hinted(&scope, "scratch", serde_json::json!("temp"), &opts)
            .await
            .unwrap();

        // Transient entries are not visible via read()
        let val = store.read(&scope, "scratch").await.unwrap();
        assert_eq!(val, None, "transient entry must not be visible via read()");

        // clear_transient is idempotent
        store.clear_transient();
        store.clear_transient();

        // Write a durable entry
        store
            .write(&scope, "durable", serde_json::json!("persisted"))
            .await
            .unwrap();

        // clear_transient does not touch durable storage
        store.clear_transient();

        let durable_val = store.read(&scope, "durable").await.unwrap();
        assert_eq!(
            durable_val,
            Some(serde_json::json!("persisted")),
            "durable entry must survive clear_transient()"
        );
    }

    // ── LRU / bounded tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn bounded_evicts_oldest() {
        let store = MemoryStore::bounded(3);
        let scope = Scope::Global;

        for k in ["a", "b", "c", "d", "e"] {
            store.write(&scope, k, json!(k)).await.unwrap();
        }

        assert_eq!(store.read(&scope, "a").await.unwrap(), None, "a should be evicted");
        assert_eq!(store.read(&scope, "b").await.unwrap(), None, "b should be evicted");
        assert_eq!(store.read(&scope, "c").await.unwrap(), Some(json!("c")));
        assert_eq!(store.read(&scope, "d").await.unwrap(), Some(json!("d")));
        assert_eq!(store.read(&scope, "e").await.unwrap(), Some(json!("e")));
    }

    #[tokio::test]
    async fn bounded_read_refreshes_lru() {
        let store = MemoryStore::bounded(3);
        let scope = Scope::Global;

        store.write(&scope, "a", json!("a")).await.unwrap();
        store.write(&scope, "b", json!("b")).await.unwrap();
        store.write(&scope, "c", json!("c")).await.unwrap();

        // Touch "a" — it becomes most-recently used; order becomes [b, c, a].
        let _ = store.read(&scope, "a").await.unwrap();

        // Write "d" — should evict "b" (now at front), not "a".
        store.write(&scope, "d", json!("d")).await.unwrap();

        assert_eq!(store.read(&scope, "b").await.unwrap(), None, "b should be evicted");
        assert!(store.read(&scope, "a").await.unwrap().is_some(), "a should survive");
        assert!(store.read(&scope, "c").await.unwrap().is_some(), "c should survive");
        assert!(store.read(&scope, "d").await.unwrap().is_some(), "d should survive");
    }

    #[tokio::test]
    async fn bounded_unlimited_default() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        for i in 0..100u32 {
            store.write(&scope, &i.to_string(), json!(i)).await.unwrap();
        }

        for i in 0..100u32 {
            assert!(
                store.read(&scope, &i.to_string()).await.unwrap().is_some(),
                "key {i} should not be evicted from unbounded store",
            );
        }
    }

    // ── Search tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_finds_substring() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "k1", json!("hello world")).await.unwrap();
        store.write(&scope, "k2", json!("goodbye world")).await.unwrap();
        store.write(&scope, "k3", json!(42)).await.unwrap();

        let results = store.search(&scope, "world", 10).await.unwrap();
        let keys: Vec<&str> = results.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"k1"), "k1 should match");
        assert!(keys.contains(&"k2"), "k2 should match");
        assert!(!keys.contains(&"k3"), "k3 should not match");
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        store.write(&scope, "k1", json!("Hello World")).await.unwrap();
        store.write(&scope, "k2", json!("HELLO")).await.unwrap();
        store.write(&scope, "k3", json!("unrelated")).await.unwrap();

        let results = store.search(&scope, "hello", 10).await.unwrap();
        let keys: Vec<&str> = results.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"k1"), "k1 should match case-insensitively");
        assert!(keys.contains(&"k2"), "k2 should match case-insensitively");
        assert!(!keys.contains(&"k3"), "k3 should not match");
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let store = MemoryStore::new();
        let scope = Scope::Global;

        for i in 0..10u32 {
            store
                .write(&scope, &format!("k{i}"), json!("needle in haystack"))
                .await
                .unwrap();
        }

        let results = store.search(&scope, "needle", 3).await.unwrap();
        assert_eq!(results.len(), 3, "results must be capped at the limit");
    }
}
