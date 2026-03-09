#![deny(missing_docs)]
//! Filesystem-backed implementation of layer0's StateStore trait.
//!
//! Each scope maps to a subdirectory under the root. Keys are
//! URL-encoded and stored as `.json` or `.md` files within the scope directory.
//! Provides true persistence across process restarts.

use async_trait::async_trait;
use layer0::effect::Scope;
use layer0::error::StateError;
use layer0::state::{SearchResult, StateStore, StoreOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Serialization format for stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// JSON (default). Values stored as `.json` files.
    #[default]
    Json,
    /// Markdown. Values stored as `.md` files.
    ///
    /// String values are written as raw text. All other JSON types are
    /// serialized as a fenced JSON code block (` ```json … ``` `).
    /// On read, content that parses as valid JSON is returned as-is;
    /// content that does not is wrapped as a JSON string value.
    Markdown,
}

impl Format {
    /// Returns the file extension for data files in this format, without the leading dot.
    fn ext(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Markdown => "md",
        }
    }
}

/// Filesystem-backed state store.
///
/// Directory layout:
/// ```text
/// root/
///   <scope-hash>/
///     <url-encoded-key>.json        (or .md when using Format::Markdown)
///     <url-encoded-key>_meta.json   (optional TTL sidecar — always JSON)
/// ```
///
/// Suitable for development, single-machine deployments, and cases
/// where data must survive process restarts without a database.
pub struct FsStore {
    root: PathBuf,
    format: Format,
}

impl FsStore {
    /// Create a new filesystem store rooted at the given directory.
    ///
    /// Defaults to [`Format::Json`]. The directory is created lazily on first write.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            format: Format::default(),
        }
    }

    /// Create a new filesystem store with the given serialization format.
    ///
    /// The directory is created lazily on first write.
    pub fn with_format(root: &Path, format: Format) -> Self {
        Self {
            root: root.to_path_buf(),
            format,
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

/// Encode a key into a percent-encoded filename stem (without extension).
///
/// The data file for a key is `{stem}.{ext}`; its TTL sidecar is `{stem}_meta.json`.
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
    encoded
}

/// Decode a filename back to a key, stripping the given extension (without leading dot).
///
/// Returns `None` if the filename does not end with `.{ext}`.
fn filename_to_key(filename: &str, ext: &str) -> Option<String> {
    let suffix = format!(".{ext}");
    let name = filename.strip_suffix(&suffix)?;
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

/// Returns `true` if the entry has expired and should be treated as absent.
fn is_expired(meta_path: &Path) -> bool {
    let Ok(data) = std::fs::read(meta_path) else {
        return false;
    };
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return false;
    };
    let Some(expires_at) = val.get("expires_at").and_then(|v| v.as_u64()) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    now >= expires_at
}

/// Read a JSON data file, returning `None` if absent.
async fn read_json(path: &Path) -> Result<Option<serde_json::Value>, StateError> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            let value: serde_json::Value = serde_json::from_str(&contents)
                .map_err(|e| StateError::Serialization(e.to_string()))?;
            Ok(Some(value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StateError::WriteFailed(e.to_string())),
    }
}

/// Read a Markdown data file, returning `None` if absent.
///
/// The read order is:
///   1. Try to parse the raw content as JSON.
///   2. Try to strip a fenced JSON code block (` ```json…``` `) and parse the inner text.
///   3. Fall back to wrapping the raw content as [`serde_json::Value::String`].
async fn read_md(path: &Path) -> Result<Option<serde_json::Value>, StateError> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            // 1. Try raw JSON parse (handles bare strings stored as quoted JSON, etc.)
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                return Ok(Some(value));
            }
            // 2. Try fenced JSON code block produced by value_to_markdown.
            if let Some(inner) = strip_json_fence(&contents)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(inner)
            {
                return Ok(Some(value));
            }
            // 3. Treat the raw content as a plain string value.
            Ok(Some(serde_json::Value::String(contents)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StateError::WriteFailed(e.to_string())),
    }
}

/// Strip the ` ```json\n…\n``` ` fence from Markdown content and return the inner text.
fn strip_json_fence(s: &str) -> Option<&str> {
    let inner = s.strip_prefix("```json\n")?;
    inner.strip_suffix("\n```")
}

/// Serialize a JSON value to Markdown text.
///
/// String values are written as raw text. All other types are wrapped in a
/// fenced JSON code block.
fn value_to_markdown(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_default();
            format!("```json\n{pretty}\n```")
        }
    }
}

/// Count non-overlapping occurrences of `needle` within `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

#[async_trait]
impl StateStore for FsStore {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let scope_path = self.root.join(scope_dir_name(scope));
        let stem = key_to_filename(key);
        let ext = self.format.ext();
        let data_path = scope_path.join(format!("{stem}.{ext}"));
        let meta_path = scope_path.join(format!("{stem}_meta.json"));

        // Check expiry lazily: if expired, delete both files and return None.
        if meta_path.exists() && is_expired(&meta_path) {
            let _ = std::fs::remove_file(&data_path);
            let _ = std::fs::remove_file(&meta_path);
            return Ok(None);
        }

        match self.format {
            Format::Json => read_json(&data_path).await,
            Format::Markdown => read_md(&data_path).await,
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

        let stem = key_to_filename(key);
        let ext = self.format.ext();
        let path = dir.join(format!("{stem}.{ext}"));

        let contents = match self.format {
            Format::Json => serde_json::to_string_pretty(&value)
                .map_err(|e| StateError::Serialization(e.to_string()))?,
            Format::Markdown => value_to_markdown(&value),
        };

        tokio::fs::write(&path, contents)
            .await
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let dir = self.root.join(scope_dir_name(scope));
        let stem = key_to_filename(key);
        let ext = self.format.ext();
        let path = dir.join(format!("{stem}.{ext}"));
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StateError::WriteFailed(e.to_string())),
        }
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        let dir = self.root.join(scope_dir_name(scope));
        let ext = self.format.ext();
        let data_suffix = format!(".{ext}");

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
                // Explicitly skip TTL sidecar files — they must not appear as keys.
                && !filename.ends_with("_meta.json")
                && filename.ends_with(&data_suffix)
                && let Some(key) = filename_to_key(filename, ext)
                && key.starts_with(prefix)
            {
                // Skip expired entries without deleting them (lazy cleanup on read).
                let stem = filename.strip_suffix(&data_suffix).unwrap_or(filename);
                let meta_path = dir.join(format!("{stem}_meta.json"));
                if meta_path.exists() && is_expired(&meta_path) {
                    continue;
                }
                keys.push(key);
            }
        }
        Ok(keys)
    }

    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        if query.is_empty() {
            return Ok(vec![]);
        }

        let keys = self.list(scope, "").await?;
        let query_lower = query.to_lowercase();

        // When the `regex` feature is enabled, attempt to compile the query as a
        // regex pattern and fall back to substring matching if compilation fails.
        #[cfg(feature = "regex")]
        let compiled_regex = regex::Regex::new(query).ok();

        let mut results = Vec::new();

        for key in keys {
            let Some(value) = self.read(scope, &key).await? else {
                continue;
            };

            let text = match &value {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };

            let match_count = {
                #[cfg(feature = "regex")]
                {
                    if let Some(ref re) = compiled_regex {
                        re.find_iter(&text).count()
                    } else {
                        count_occurrences(&text.to_lowercase(), &query_lower)
                    }
                }
                #[cfg(not(feature = "regex"))]
                {
                    count_occurrences(&text.to_lowercase(), &query_lower)
                }
            };

            if match_count > 0 {
                let score = if text.is_empty() {
                    0.0
                } else {
                    match_count as f64 / text.len() as f64
                };
                let mut result = SearchResult::new(key, score);
                result.snippet = Some(text.chars().take(200).collect());
                results.push(result);
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
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
        // Write the data file first (also ensures the scope directory exists).
        self.write(scope, key, value).await?;

        // If a TTL was specified, write a sidecar recording the expiry timestamp.
        // TTL sidecars always use .json regardless of the store's data format.
        if let Some(ttl) = options.ttl {
            let dir = self.root.join(scope_dir_name(scope));
            let stem = key_to_filename(key);
            let meta_path = dir.join(format!("{stem}_meta.json"));

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let expires_at = now_ms.saturating_add(ttl.as_millis());

            let meta = serde_json::json!({ "expires_at": expires_at });
            let contents = serde_json::to_string(&meta)
                .map_err(|e| StateError::Serialization(e.to_string()))?;
            tokio::fs::write(&meta_path, contents)
                .await
                .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        }

        Ok(())
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
            let stem = key_to_filename(key);
            let filename = format!("{stem}.json");
            let decoded = filename_to_key(&filename, "json").unwrap();
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
    fn key_to_filename_returns_stem_without_extension() {
        let stem = key_to_filename("test");
        assert!(
            !stem.ends_with(".json"),
            "key_to_filename should return a stem without the .json extension"
        );
    }

    #[test]
    fn filename_to_key_rejects_non_json() {
        let result = filename_to_key("test.txt", "json");
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

    #[tokio::test]
    async fn test_fsstore_ttl_expiration() {
        use layer0::DurationMs;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        let opts = StoreOptions {
            ttl: Some(DurationMs::from_millis(1)),
            ..Default::default()
        };
        store
            .write_hinted(&scope, "expiring", serde_json::json!("value"), &opts)
            .await
            .unwrap();

        // Wait long enough for the 1ms TTL to elapse.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Expired entry must return None.
        let result = store.read(&scope, "expiring").await.unwrap();
        assert!(result.is_none(), "expired entry should return None");

        // List must not surface the expired key.
        let keys = store.list(&scope, "").await.unwrap();
        assert!(
            !keys.contains(&"expiring".to_string()),
            "expired entry must not appear in list"
        );
    }

    #[tokio::test]
    async fn test_fsstore_no_ttl_reads_normally() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        // Write with no TTL.
        let opts = StoreOptions::default();
        store
            .write_hinted(&scope, "durable", serde_json::json!("keep"), &opts)
            .await
            .unwrap();

        // Should still be present.
        let result = store.read(&scope, "durable").await.unwrap();
        assert_eq!(result, Some(serde_json::json!("keep")));

        // Should appear in list.
        let keys = store.list(&scope, "").await.unwrap();
        assert!(keys.contains(&"durable".to_string()));
    }

    #[tokio::test]
    async fn test_fsstore_durable_and_expiring_coexist() {
        use layer0::DurationMs;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        // Write a durable entry.
        store
            .write(&scope, "durable_a", serde_json::json!("stays"))
            .await
            .unwrap();

        // Write a short-TTL entry.
        let opts = StoreOptions {
            ttl: Some(DurationMs::from_millis(1)),
            ..Default::default()
        };
        store
            .write_hinted(&scope, "expiring_b", serde_json::json!("gone"), &opts)
            .await
            .unwrap();

        // Let the TTL expire.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Trigger expiry cleanup by reading the expired key.
        let expired = store.read(&scope, "expiring_b").await.unwrap();
        assert!(expired.is_none(), "expiring_b should have expired");

        // List must show only the durable entry.
        let keys = store.list(&scope, "").await.unwrap();
        assert!(
            keys.contains(&"durable_a".to_string()),
            "durable_a should still be listed"
        );
        assert!(
            !keys.contains(&"expiring_b".to_string()),
            "expiring_b must not appear after expiry"
        );
    }

    // --- Format::Markdown tests ---

    #[tokio::test]
    async fn markdown_write_and_read_string() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::with_format(dir.path(), Format::Markdown);
        let scope = Scope::Global;

        store
            .write(&scope, "note", json!("Hello, world!"))
            .await
            .unwrap();

        // Verify raw file contains the string without JSON quoting.
        let stem = key_to_filename("note");
        let raw = std::fs::read_to_string(
            dir.path()
                .join(scope_dir_name(&scope))
                .join(format!("{stem}.md")),
        )
        .unwrap();
        assert_eq!(raw, "Hello, world!");

        // Round-trip through the store returns the original value.
        let val = store.read(&scope, "note").await.unwrap();
        assert_eq!(val, Some(json!("Hello, world!")));
    }

    #[tokio::test]
    async fn markdown_write_and_read_object() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::with_format(dir.path(), Format::Markdown);
        let scope = Scope::Global;

        let obj = json!({"name": "Alice", "age": 30});
        store.write(&scope, "profile", obj.clone()).await.unwrap();

        // Verify raw file is a fenced JSON code block.
        let stem = key_to_filename("profile");
        let raw = std::fs::read_to_string(
            dir.path()
                .join(scope_dir_name(&scope))
                .join(format!("{stem}.md")),
        )
        .unwrap();
        assert!(raw.starts_with("```json\n"), "expected fenced code block");
        assert!(raw.ends_with("\n```"), "expected closing fence");

        // Round-trip must reconstruct the original JSON object.
        let val = store.read(&scope, "profile").await.unwrap();
        assert_eq!(val, Some(obj));
    }

    #[tokio::test]
    async fn markdown_list_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::with_format(dir.path(), Format::Markdown);
        let scope = Scope::Global;

        store.write(&scope, "alpha", json!("a")).await.unwrap();
        store.write(&scope, "beta", json!("b")).await.unwrap();

        let mut keys = store.list(&scope, "").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["alpha", "beta"]);
    }

    // --- Search tests ---

    #[tokio::test]
    async fn search_finds_substring() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        store
            .write(&scope, "a", json!("the quick brown fox"))
            .await
            .unwrap();
        store
            .write(&scope, "b", json!("jumped over the lazy dog"))
            .await
            .unwrap();
        store
            .write(&scope, "c", json!("completely unrelated content"))
            .await
            .unwrap();

        let results = store.search(&scope, "fox", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "a");
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        store
            .write(&scope, "k1", json!("Hello World"))
            .await
            .unwrap();

        let results = store.search(&scope, "hello", 10).await.unwrap();
        assert_eq!(results.len(), 1, "case-insensitive match should find Hello");

        let results2 = store.search(&scope, "WORLD", 10).await.unwrap();
        assert_eq!(results2.len(), 1, "case-insensitive match should find World");
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        for i in 0..10 {
            store
                .write(&scope, &format!("key{i}"), json!(format!("needle content {i}")))
                .await
                .unwrap();
        }

        let results = store.search(&scope, "needle", 3).await.unwrap();
        assert_eq!(results.len(), 3, "limit of 3 must be respected");
    }

    #[tokio::test]
    async fn search_returns_empty_for_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path());
        let scope = Scope::Global;

        store
            .write(&scope, "k1", json!("some content here"))
            .await
            .unwrap();

        let results = store
            .search(&scope, "xyzzy_nonexistent", 10)
            .await
            .unwrap();
        assert!(results.is_empty());
    }
}
