# State Search & Conversation Persistence Design

Status: **Draft** (brainstorm output)
Covers: A6 (conversation persistence), A7 (vector search), search polymorphism

## Problem

The `StateStore` trait defines a universal text search interface (`search(scope, query, limit)`).
Every backend implements it, but each has fundamentally different capabilities:

| Store | Internal engine | Unique capability |
|-------|----------------|-------------------|
| MemoryStore | substring count | LRU eviction, durable pinning |
| FsStore | substring / regex (feature) | Markdown format, filesystem portability |
| SqliteStore | FTS5 + BM25 | Ranked full-text search, snippets |
| CozoStore | substring (v1), Datalog (planned) | Graph traversal, relation queries |

Forcing all search through `search(scope, &str, limit)` loses backend-specific power.
The current `SearchOptions` struct adds `min_score`, `content_kind`, `tier`, `max_depth` —
but cannot express FTS5 query syntax, vector similarity, Datalog programs, or regex patterns.

Separately, `Context` is in-memory only. No mechanism exists to save and restore a
conversation across sessions. Only `Vec<Message>` and `Vec<Effect>` are serializable;
`Extensions`, `TurnMetrics`, and `Rules` contain non-serializable runtime state.

## Design Principles

1. **The protocol trait stays minimal.** `StateStore` is the lowest common denominator.
   Every backend can implement it. Don't grow it with backend-specific methods.

2. **Each state crate owns its capabilities.** Rich, type-safe methods live on the
   concrete type (`SqliteStore::fts_search()`, `CozoStore::datalog_query()`).
   The developer picked the concrete type — they can call its methods directly.

3. **Context ops work with results, not search strategies.** A generic
   `InjectSearchResults` op takes `Vec<SearchResult>` regardless of how they were
   obtained. Context-engine never imports a state crate.

4. **Conversation persistence is explicit.** Two ops (`SaveConversation`,
   `LoadConversation`) serialize/deserialize `Vec<Message>` through the `StateStore`
   trait. No magic. The developer calls them when they want persistence.

## Architecture

### Search: Per-Crate Rich APIs

```
                    ┌──────────────────────────┐
                    │    StateStore trait       │
                    │  search(scope, &str, N)   │  ← protocol minimum
                    └──────────────────────────┘
                               ▲
          ┌────────────────────┼────────────────────┐
          │                    │                     │
  ┌───────┴──────┐   ┌────────┴───────┐   ┌────────┴───────┐
  │ SqliteStore  │   │  CozoStore     │   │   FsStore      │
  │              │   │                │   │                │
  │ fts_search() │   │ datalog()      │   │ regex_search() │
  │ (future:     │   │ traverse_full()│   │                │
  │  vec_search) │   │                │   │                │
  └──────────────┘   └────────────────┘   └────────────────┘
          │                    │                     │
          └────────────────────┼─────────────────────┘
                               ▼
                    ┌──────────────────────────┐
                    │  Vec<SearchResult>        │
                    └──────────────────────────┘
                               │
                               ▼
                    ┌──────────────────────────┐
                    │  InjectSearchResults op   │  ← context-engine
                    │  (formats + injects into  │
                    │   ctx.messages)            │
                    └──────────────────────────┘
```

#### SqliteStore additions

```rust
impl SqliteStore {
    /// Full-text search using FTS5 query syntax with BM25 ranking.
    ///
    /// Accepts FTS5 MATCH expressions: `"neural AND network"`, `"prefix*"`,
    /// `"NEAR(agent runtime)"`. Returns results with BM25 scores and snippets.
    pub async fn fts_search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
}

// Future (behind `sqlite-vec` feature flag):
impl SqliteStore {
    /// Vector similarity search using sqlite-vec.
    ///
    /// The caller provides the embedding — SqliteStore does not embed.
    /// The embedding provider is the developer's choice.
    pub async fn vector_search(
        &self,
        scope: &Scope,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
}
```

#### CozoStore additions

```rust
impl CozoStore {
    /// Execute a raw CozoScript/Datalog program.
    ///
    /// Returns results as JSON rows. The developer writes the query —
    /// CozoStore provides the execution engine.
    pub async fn datalog_query(
        &self,
        program: &str,
    ) -> Result<Vec<serde_json::Value>, StateError>;

    /// Graph traversal returning full values, not just keys.
    ///
    /// Starts from `from_key`, follows edges matching `relation` (None = any),
    /// up to `max_depth` hops. Returns SearchResult with snippets from values.
    pub async fn traverse_full(
        &self,
        scope: &Scope,
        from_key: &str,
        relation: Option<&str>,
        max_depth: u32,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
}
```

#### FsStore additions

```rust
// Behind `regex` feature (already has the internal impl)
impl FsStore {
    /// Search values using a compiled regex pattern.
    ///
    /// Scores by match density (matches / text length).
    pub async fn regex_search(
        &self,
        scope: &Scope,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, StateError>;
}
```

#### Context-engine: InjectSearchResults

```rust
/// Inject pre-obtained search results into context as messages.
///
/// This op is search-strategy-agnostic. It takes results from any source
/// (FTS5, vector, graph, regex, external API) and formats them into messages.
pub struct InjectSearchResults {
    results: Vec<(String, serde_json::Value)>,  // (key, value) pairs
    position: InjectionPosition,
    role: Role,
    policy: CompactionPolicy,
    formatter: Formatter,
}
```

Usage:
```rust
// FTS5 search → inject
let hits = sqlite_store.fts_search(&scope, "user preferences", 5).await?;
let pairs = load_values(&sqlite_store, &scope, &hits).await?;
ctx.run(InjectSearchResults::new(pairs)).await?;

// Graph traversal → inject
let hits = cozo_store.traverse_full(&scope, "user:123", None, 2, 10).await?;
let pairs = load_values(&cozo_store, &scope, &hits).await?;
ctx.run(InjectSearchResults::new(pairs)).await?;

// Or use existing InjectFromStore for simple text search (unchanged)
ctx.run(InjectFromStore::new(store.clone(), scope, "query", 5)).await?;
```

### Conversation Persistence

#### What gets saved

Only `Vec<Message>`. This is deliberate:

| Field | Serializable | Saved | Why |
|-------|-------------|-------|-----|
| `messages` | Yes | Yes | The conversation IS the messages |
| `effects` | Yes | No | Effects are execution artifacts, not conversation state |
| `metrics` | No (`Instant`) | No | Runtime performance data, not conversation |
| `extensions` | No (`dyn Any`) | No | Runtime type-erased state |
| `rules` | No (closures) | No | Behavior config, reconstructed at startup |

#### Two new ContextOps

```rust
/// Save conversation messages to a StateStore.
///
/// Serializes `ctx.messages` as JSON and writes to the store
/// under `{scope}/{key}`. Overwrites any existing value.
pub struct SaveConversation {
    store: Arc<dyn StateStore>,
    scope: Scope,
    key: String,
}

impl SaveConversation {
    pub fn new(
        store: Arc<dyn StateStore>,
        scope: Scope,
        key: impl Into<String>,
    ) -> Self;
}

// ContextOp<Output = ()>
```

```rust
/// Load conversation messages from a StateStore.
///
/// Reads from `{scope}/{key}`, deserializes as `Vec<Message>`,
/// and REPLACES `ctx.messages` with the loaded messages.
///
/// Returns the number of messages loaded, or 0 if the key doesn't exist.
///
/// Note: this REPLACES, not appends. The caller should call this
/// before adding new messages to the context.
pub struct LoadConversation {
    store: Arc<dyn StateStore>,
    scope: Scope,
    key: String,
}

impl LoadConversation {
    pub fn new(
        store: Arc<dyn StateStore>,
        scope: Scope,
        key: impl Into<String>,
    ) -> Self;
}

// ContextOp<Output = usize>  (number of messages loaded)
```

#### Usage pattern

```rust
let session_key = format!("conversation:{session_id}");
let scope = Scope::Session(session_id.clone());

// Resume a conversation
let mut ctx = Context::new();
ctx.add_rule(budget_guard);
ctx.add_rule(telemetry);

let loaded = ctx.run(LoadConversation::new(
    store.clone(), scope.clone(), &session_key,
)).await?;
tracing::info!(loaded, "restored conversation");

// Add the new user message
ctx.run(InjectMessage(Message::new(Role::User, Content::text(user_input)))).await?;

// Run the agent loop
let output = react_loop(&mut ctx, &provider, &tools, &tool_ctx, &config).await?;

// Persist after the turn
ctx.run(SaveConversation::new(
    store.clone(), scope.clone(), &session_key,
)).await?;
```

#### Store-specific optimizations (future)

Each store can optimize conversation storage beyond JSON blobs:

- **SqliteStore**: Could store messages in a dedicated `conversations` table with
  per-message rows for incremental append and FTS indexing of conversation content.
- **FsStore**: With `Format::Markdown`, could write conversations as readable
  markdown transcripts.
- **CozoStore**: Could store messages as graph nodes with temporal edges for
  conversation-aware graph queries.

These would be new methods on the concrete types (e.g., `SqliteStore::save_conversation_incremental()`),
NOT changes to the trait. The basic `SaveConversation`/`LoadConversation` ops use `StateStore::write/read`
and work with every backend immediately.

## What This Does NOT Cover

- **Vector embeddings pipeline**: Who computes embeddings is the developer's problem.
  `SqliteStore::vector_search()` takes `&[f32]`, not text.
- **Compaction**: Already complete. `sliding_window`, `policy_trim`, `summarize`,
  `extract_cognitive_state` compose freely. No changes needed.
- **Durability / crash recovery** (A9): Separate design. Requires event-sourcing
  or checkpoint/replay, which is an orchestrator concern.
- **MCP integration** (A8): Separate crate, separate design.

## Implementation Order

1. **SqliteStore::fts_search()** — promote existing `pub(crate)` to `pub`. Minimal work.
2. **FsStore::regex_search()** — promote existing internal impl. Minimal work.
3. **SaveConversation + LoadConversation** — new ops in context-engine. ~150 lines + tests.
4. **InjectSearchResults** — new generic op in context-engine. ~100 lines + tests.
5. **CozoStore::datalog_query() + traverse_full()** — new methods. Medium effort.
6. **SqliteStore::vector_search()** — behind `sqlite-vec` feature. Larger effort, new dependency.

Items 1-4 are the minimal viable set. Items 5-6 are follow-up.
