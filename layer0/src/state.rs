//! The State protocol — how data persists and is retrieved across turns.

use crate::{duration::DurationMs, effect::Scope, error::StateError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Storage tier hint for reads and writes.
///
/// Backends may ignore this hint. The hint is advisory only —
/// callers must not assume any specific latency or durability guarantee.
/// Default is [`MemoryTier::Hot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Frequently accessed data. Prefer low-latency storage.
    #[default]
    Hot,
    /// Moderately accessed data. In-process or near cache.
    Warm,
    /// Rarely accessed data. May use slower, cheaper storage.
    Cold,
}

/// Persistence lifetime policy hint for memory writes.
///
/// All hints are advisory — backends MAY ignore them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    /// Persists only within the current turn/step. Never written to durable storage.
    /// Use for intermediate reasoning scratchpad data.
    Transient,
    /// Persists for the duration of the session. Discarded when the session ends.
    Session,
    /// Persists indefinitely across sessions.
    Durable,
}

/// Cognitive category hint for memory writes.
///
/// Based on memory taxonomy: episodic/semantic/procedural/structural.
/// All hints are advisory — backends MAY use this to route storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// Specific past events: "user approved PR #42 on Jan 15".
    Episodic,
    /// Generalized facts: "the API uses OAuth2".
    Semantic,
    /// How-to knowledge: "to deploy, run make release".
    Procedural,
    /// Environment or file-system state: "file X exists at path Y".
    Structural,
    /// Escape hatch for domain-specific categories.
    Custom(String),
}

/// Advisory options for StateStore reads and writes.
///
/// Backends may ignore any or all of these hints. The contract
/// promises at-least-once delivery but not specific performance
/// characteristics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreOptions {
    /// Advisory tier hint for the backend.
    pub tier: Option<MemoryTier>,
    /// Advisory persistence policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<Lifetime>,
    /// Cognitive category of the memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_kind: Option<ContentKind>,
    /// Write-time importance hint (0.0–1.0). Higher = more important to preserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// Auto-expire after this duration. Backends may ignore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<DurationMs>,
}

/// Advisory options for enhanced search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchOptions {
    /// Minimum relevance score threshold (0.0-1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    /// Filter to specific content kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_kind: Option<ContentKind>,
    /// Filter to specific memory tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<MemoryTier>,
    /// Maximum graph traversal depth for graph-aware backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

/// A link between two memory entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLink {
    /// Source key.
    pub from_key: String,
    /// Target key.
    pub to_key: String,
    /// Relationship type (e.g. "references", "supersedes", "related_to").
    pub relation: String,
    /// Optional link metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl MemoryLink {
    /// Create a new memory link.
    pub fn new(
        from_key: impl Into<String>,
        to_key: impl Into<String>,
        relation: impl Into<String>,
    ) -> Self {
        Self {
            from_key: from_key.into(),
            to_key: to_key.into(),
            relation: relation.into(),
            metadata: None,
        }
    }
}

/// Protocol ③ — State
///
/// How data persists and is retrieved across turns and sessions.
///
/// Implementations:
/// - InMemoryStore: HashMap (testing, ephemeral)
/// - FsStore: filesystem (CLAUDE.md, plain files)
/// - GitStore: git-backed (versioned, auditable, mergeable)
/// - SqliteStore: embedded database
/// - PgStore: PostgreSQL (queryable, transactional)
///
/// The trait is deliberately minimal — CRUD + search + list.
/// Compaction is NOT part of this trait because compaction requires
/// coordination across protocols (the Lifecycle Interface).
/// Versioning is NOT part of this trait because not all backends
/// support it — implementations that do can expose it via
/// additional traits or methods.
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Read a value by key within a scope.
    /// Returns None if the key doesn't exist.
    async fn read(&self, scope: &Scope, key: &str)
    -> Result<Option<serde_json::Value>, StateError>;

    /// Write a value. Creates or overwrites.
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError>;

    /// Delete a value. No-op if key doesn't exist.
    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError>;

    /// List keys under a prefix within a scope.
    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError>;

    /// Semantic search within a scope. Returns matching keys
    /// with relevance scores. Implementations that don't support
    /// search return an empty vec (not an error).
    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
    /// Read a value with advisory options. Backends may ignore options.
    ///
    /// Default: delegates to [`StateStore::read`], ignoring options.
    async fn read_hinted(
        &self,
        scope: &Scope,
        key: &str,
        _options: &StoreOptions,
    ) -> Result<Option<serde_json::Value>, StateError> {
        self.read(scope, key).await
    }

    /// Write a value with advisory options. Backends may ignore options.
    ///
    /// Default: delegates to [`StateStore::write`], ignoring options.
    async fn write_hinted(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        _options: &StoreOptions,
    ) -> Result<(), StateError> {
        self.write(scope, key, value).await
    }
    /// Clear all transient-lifetime entries from the store.
    ///
    /// Called by operators at turn boundaries to discard scratchpad data
    /// written with `Lifetime::Transient`. Backends that do not support
    /// lifetime semantics implement this as a no-op (the default).
    fn clear_transient(&self) {}
    /// Create a link between two memory entries.
    ///
    /// Graph-aware backends create a typed edge. Default returns an error
    /// indicating graph operations are not supported.
    async fn link(&self, _scope: &Scope, _link: &MemoryLink) -> Result<(), StateError> {
        Err(StateError::Other(
            "graph operations not supported by this store".into(),
        ))
    }

    /// Remove a link between two memory entries.
    ///
    /// Default returns an error indicating graph operations are not supported.
    async fn unlink(
        &self,
        _scope: &Scope,
        _from_key: &str,
        _to_key: &str,
        _relation: &str,
    ) -> Result<(), StateError> {
        Err(StateError::Other(
            "graph operations not supported by this store".into(),
        ))
    }

    /// Traverse links from a starting key.
    ///
    /// Returns keys reachable within `max_depth` hops via edges matching
    /// `relation` (None = any relation). Default returns an error indicating
    /// graph operations are not supported.
    async fn traverse(
        &self,
        _scope: &Scope,
        _from_key: &str,
        _relation: Option<&str>,
        _max_depth: u32,
    ) -> Result<Vec<String>, StateError> {
        Err(StateError::Other(
            "graph operations not supported by this store".into(),
        ))
    }

    /// Enhanced search with advisory options.
    ///
    /// Default: delegates to `search()`, ignoring options.
    async fn search_hinted(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
        _options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, StateError> {
        self.search(scope, query, limit).await
    }
}

/// A search result from a state store query.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The key that matched.
    pub key: String,
    /// Relevance score (higher is more relevant).
    pub score: f64,
    /// Preview/snippet of the matched content.
    /// Implementations decide what to include.
    pub snippet: Option<String>,
}

impl SearchResult {
    /// Create a new search result.
    pub fn new(key: impl Into<String>, score: f64) -> Self {
        Self {
            key: key.into(),
            score,
            snippet: None,
        }
    }
}

/// Read-only view of state, given to the operator runtime during
/// context assembly. The operator can read but cannot write — writes
/// go through Effects in OperatorOutput.
///
/// This trait exists to enforce the read/write asymmetry at the
/// type level. An Operator receives `&dyn StateReader`, not `&dyn StateStore`.
#[async_trait]
pub trait StateReader: Send + Sync {
    /// Read a value by key within a scope.
    async fn read(&self, scope: &Scope, key: &str)
    -> Result<Option<serde_json::Value>, StateError>;

    /// List keys under a prefix within a scope.
    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError>;

    /// Semantic search within a scope.
    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
    /// Read a value with advisory options. Default: ignores options.
    async fn read_hinted(
        &self,
        scope: &Scope,
        key: &str,
        _options: &StoreOptions,
    ) -> Result<Option<serde_json::Value>, StateError> {
        self.read(scope, key).await
    }
    /// Clear all transient-lifetime entries. Default: no-op.
    ///
    /// See [`StateStore::clear_transient`] for semantics.
    fn clear_transient(&self) {}
    /// Traverse links from a starting key.
    ///
    /// Returns keys reachable within `max_depth` hops via edges matching
    /// `relation` (None = any relation). Default: empty vec.
    async fn traverse(
        &self,
        _scope: &Scope,
        _from_key: &str,
        _relation: Option<&str>,
        _max_depth: u32,
    ) -> Result<Vec<String>, StateError> {
        Ok(vec![])
    }

    /// Enhanced search with advisory options.
    ///
    /// Default: delegates to `search()`, ignoring options.
    async fn search_hinted(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
        _options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, StateError> {
        self.search(scope, query, limit).await
    }
}

/// Blanket implementation: every StateStore is a StateReader.
#[async_trait]
impl<T: StateStore> StateReader for T {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        StateStore::read(self, scope, key).await
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        StateStore::list(self, scope, prefix).await
    }

    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        StateStore::search(self, scope, query, limit).await
    }
    async fn read_hinted(
        &self,
        scope: &Scope,
        key: &str,
        options: &StoreOptions,
    ) -> Result<Option<serde_json::Value>, StateError> {
        StateStore::read_hinted(self, scope, key, options).await
    }
    fn clear_transient(&self) {
        StateStore::clear_transient(self);
    }
    async fn traverse(
        &self,
        scope: &Scope,
        from_key: &str,
        relation: Option<&str>,
        max_depth: u32,
    ) -> Result<Vec<String>, StateError> {
        StateStore::traverse(self, scope, from_key, relation, max_depth).await
    }

    async fn search_hinted(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, StateError> {
        StateStore::search_hinted(self, scope, query, limit, options).await
    }
}
