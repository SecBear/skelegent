#![deny(missing_docs)]
//! Filesystem-backed implementation of layer0's StateStore trait.
//!
//! Each scope maps to a subdirectory under the root. Keys are
//! URL-encoded and stored as `.json` files within the scope directory.
//! Provides true persistence across process restarts.

use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::state::{SearchResult, StateStore};
use std::path::{Path, PathBuf};

/// Filesystem-backed state store.
///
/// Directory layout:
/// ```text
/// root/
///   <scope-hash>/
///     <url-encoded-key>.json
/// ```
///
/// Suitable for development, single-machine deployments, and cases
/// where data must survive process restarts without a database.
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    /// Create a new filesystem store rooted at the given directory.
    ///
    /// The directory is created lazily on first write.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

/// Derive a safe directory name from a scope.
fn scope_dir_name(scope: &Scope) -> String {
    // Use a deterministic, filesystem-safe representation.
    // We hash the JSON serialization of the scope.
    let json = serde_json::to_string(scope).unwrap_or_else(|_| "unknown".into());
    // Simple hash to avoid overly long directory names
    let mut hash: u64 = 5381;
    for byte in json.as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(*byte as u64);
    }
    format!("scope-{hash:016x}")
}

/// Encode a key into a safe filename.
fn key_to_filename(key: &str) -> String {
    let mut encoded = String::new();
    for ch in key.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => encoded.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    format!("{encoded}.json")
}

/// Decode a filename back to a key.
fn filename_to_key(filename: &str) -> Option<String> {
    let name = filename.strip_suffix(".json")?;
    let mut result = Vec::new();
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            result.push(byte);
            i += 3;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(result).ok()
}

#[async_trait]
impl StateStore for FsStore {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let path = self
            .root
            .join(scope_dir_name(scope))
            .join(key_to_filename(key));
        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => {
                let value: serde_json::Value = serde_json::from_str(&contents)
                    .map_err(|e| StateError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StateError::WriteFailed(e.to_string())),
        }
    }

    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        let dir = self.root.join(scope_dir_name(scope));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;

        let path = dir.join(key_to_filename(key));
        let contents = serde_json::to_string_pretty(&value)
            .map_err(|e| StateError::Serialization(e.to_string()))?;
        tokio::fs::write(&path, contents)
            .await
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let path = self
            .root
            .join(scope_dir_name(scope))
            .join(key_to_filename(key));
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StateError::WriteFailed(e.to_string())),
        }
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        let dir = self.root.join(scope_dir_name(scope));
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(StateError::WriteFailed(e.to_string())),
        };

        let mut keys = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| StateError::WriteFailed(e.to_string()))?
        {
            if let Some(filename) = entry.file_name().to_str()
                && let Some(key) = filename_to_key(filename)
                && key.starts_with(prefix)
            {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    async fn search(
        &self,
        _scope: &Scope,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        // Filesystem store does not support semantic search.
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_encoding_roundtrip() {
        let keys = [
            "simple",
            "user:name",
            "path/to/key",
            "has spaces",
            "emoji🎉",
        ];
        for key in &keys {
            let filename = key_to_filename(key);
            let decoded = filename_to_key(&filename).unwrap();
            assert_eq!(*key, decoded, "roundtrip failed for {key}");
        }
    }

    #[test]
    fn scope_dir_name_is_deterministic() {
        let scope = Scope::Global;
        let dir1 = scope_dir_name(&scope);
        let dir2 = scope_dir_name(&scope);
        assert_eq!(dir1, dir2);
    }

    #[test]
    fn different_scopes_get_different_dirs() {
        let global = scope_dir_name(&Scope::Global);
        let session = scope_dir_name(&Scope::Session(layer0::SessionId::new("s1")));
        assert_ne!(global, session);
    }

    #[test]
    fn key_to_filename_produces_json_extension() {
        let filename = key_to_filename("test");
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn filename_to_key_rejects_non_json() {
        let result = filename_to_key("test.txt");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        store.write(&scope, "key1", json!("hello")).await.unwrap();
        let val = store.read(&scope, "key1").await.unwrap();
        assert_eq!(val, Some(json!("hello")));
    }

    #[tokio::test]
    async fn read_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        let val = store.read(&scope, "missing").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        store.write(&scope, "key1", json!("hello")).await.unwrap();
        store.delete(&scope, "key1").await.unwrap();
        let val = store.read(&scope, "key1").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        let result = store.delete(&scope, "missing").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_keys_with_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
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
    async fn list_nonexistent_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        let keys = store.list(&scope, "").await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn scopes_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
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
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        let results = store.search(&scope, "query", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fs_store_implements_state_store() {
        fn _assert_state_store<T: StateStore>() {}
        _assert_state_store::<FsStore>();
    }
}
