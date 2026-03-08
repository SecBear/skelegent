# State Core

## Purpose

State provides continuity across operator cycles. The `StateStore` protocol is the
persistence boundary: operators read through a `StateReader` (read-only), declare writes
as `Effect::WriteMemory`, and the calling layer executes them.

## Protocol

Layer 0 defines:

- `StateStore` — CRUD + search + list + hinted variants
- `StateReader` — read-only capability; blanket-implemented for all `StateStore` implementations
- `StoreOptions` — advisory metadata carried on reads and writes

## Required Semantics

- `Scope` must be treated as part of the keyspace. Keys in different scopes are distinct.
- `list(prefix)` must be deterministic.
- `search` may be unimplemented by some backends (returns empty vec, not an error), but
  implementations must document whether they support it.

Compaction is coordinated via lifecycle vocabulary, not inside the `StateStore` trait.
Versioning is not part of this trait; implementations that support it expose additional
traits or methods.

## API Surface

### Basic CRUD (StateStore)

```rust
async fn read(&self, scope: &Scope, key: &str)
    -> Result<Option<serde_json::Value>, StateError>;

async fn write(&self, scope: &Scope, key: &str, value: serde_json::Value)
    -> Result<(), StateError>;

async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError>;

async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError>;

async fn search(&self, scope: &Scope, query: &str, limit: usize)
    -> Result<Vec<SearchResult>, StateError>;
```

### Hinted Variants

`write_hinted` and `read_hinted` accept a `&StoreOptions` alongside the normal
parameters. The defaults delegate to the unhinted variants — backends that wish to act on
hints override these methods:

```rust
async fn write_hinted(
    &self,
    scope: &Scope,
    key: &str,
    value: serde_json::Value,
    _options: &StoreOptions,
) -> Result<(), StateError> {
    self.write(scope, key, value).await  // default: ignore hints
}

async fn read_hinted(
    &self,
    scope: &Scope,
    key: &str,
    _options: &StoreOptions,
) -> Result<Option<serde_json::Value>, StateError> {
    self.read(scope, key).await  // default: ignore hints
}
```

`Effect::WriteMemory` carries the same five advisory fields so that the effect executor
can forward them to `write_hinted` without information loss.

### Transient Flush

```rust
fn clear_transient(&self) {}  // default: no-op
```

Called by operators at turn boundaries to discard scratchpad data written with
`Lifetime::Transient`. Backends that do not track lifetime semantics leave this as the
default no-op.

## StoreOptions

`StoreOptions` bundles five advisory fields passed to `write_hinted` / `read_hinted`.
All fields default to `None` (all hints absent). All fields are **advisory**: backends
**MAY** ignore any or all of them; callers **MUST NOT** assume any specific latency,
durability, or routing behavior as a result of setting them.

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreOptions {
    pub tier: Option<MemoryTier>,
    pub lifetime: Option<Lifetime>,
    pub content_kind: Option<ContentKind>,
    pub salience: Option<f64>,
    pub ttl: Option<DurationMs>,
}
```

### Field Reference

| Field | Type | Meaning | When to set |
|---|---|---|---|
| `tier` | `Option<MemoryTier>` | Preferred storage speed tier | When access latency matters (e.g., hot path vs. archive) |
| `lifetime` | `Option<Lifetime>` | How long the data should persist | For scratchpad data (`Transient`) or cross-session facts (`Durable`) |
| `content_kind` | `Option<ContentKind>` | Cognitive category of the content | When the backend routes or indexes by memory type |
| `salience` | `Option<f64>` | Write-time importance (0.0–1.0, higher = more important) | When compaction or eviction priority matters |
| `ttl` | `Option<DurationMs>` | Auto-expiry hint in milliseconds | For data with a known validity window |

### Backend Guidance

| Hint | If backend supports it | If backend ignores it |
|---|---|---|
| `tier` | Route to appropriate storage layer | Treat as `Hot` (serve from whatever is available) |
| `lifetime` | Enforce persistence policy; `Transient` entries deleted on `clear_transient()` | Data persists until explicitly deleted |
| `content_kind` | Index or route by category (e.g., vector DB namespace) | Store uniformly |
| `salience` | Prefer to retain high-salience entries during eviction | No eviction preference |
| `ttl` | Schedule expiry; remove after the duration elapses | Data persists until explicitly deleted |

Backends that partially support hints (e.g., only `lifetime`) **MUST** document which
fields they honor.

## Advisory Enums

### MemoryTier

Hint for storage speed. Default is `Hot`.

```rust
pub enum MemoryTier {
    Hot,   // default
    Warm,
    Cold,
}
```

| Variant | Semantic intent | Backend guidance |
|---|---|---|
| `Hot` | Frequently accessed; latency-sensitive | Prefer in-process or near-cache storage (HashMap, Redis) |
| `Warm` | Moderately accessed | In-process or near cache; slightly higher latency acceptable |
| `Cold` | Rarely accessed; latency-tolerant | May use slower, cheaper storage (disk, object store) |

Callers set `Hot` for data that will be read in the same or next turn, `Warm` for session
summaries, and `Cold` for archival facts unlikely to be needed soon.

### Lifetime

Hint for how long data should survive.

```rust
pub enum Lifetime {
    Transient,
    Session,
    Durable,
}
```

| Variant | Semantic intent | Backend guidance |
|---|---|---|
| `Transient` | Within the current turn only; intermediate reasoning scratchpad | Eligible for removal on `clear_transient()`; never written to durable storage |
| `Session` | Survives turns; discarded when the session ends | Persist in session-scoped storage; discard on session teardown |
| `Durable` | Survives sessions indefinitely | Write to persistent storage; do not expire |

Use `Transient` for chain-of-thought notes and tool intermediate results that are
meaningless after the turn completes. Use `Durable` for facts the agent needs across
independent invocations (e.g., user preferences, repository conventions).

### ContentKind

Cognitive category based on memory taxonomy.

```rust
pub enum ContentKind {
    Episodic,
    Semantic,
    Procedural,
    Structural,
    Custom(String),
}
```

| Variant | Semantic intent | Example |
|---|---|---|
| `Episodic` | Specific past events | "User approved PR #42 on Jan 15" |
| `Semantic` | Generalized facts | "The API uses OAuth2" |
| `Procedural` | How-to knowledge | "To deploy, run `make release`" |
| `Structural` | Environment or file-system state | "File X exists at path Y" |
| `Custom(String)` | Domain-specific escape hatch | Any category not covered above |

Backends with category-aware storage (e.g., separate vector namespaces per kind) can use
`ContentKind` for routing. Backends without category support ignore it.

## Message and CompactionPolicy

`Message` (from `layer0::context`) wraps a provider message with optional per-message compaction metadata
(`policy`, `source`, `salience`). `CompactionPolicy` controls how a compaction strategy
treats the message (Pinned / Normal / CompressFirst / DiscardWhenDone).

`Message` is defined in `layer0/src/context.rs` and `CompactionPolicy` in
`layer0/src/lifecycle.rs`. These types are used by the context-engine's assembly and
compaction functions (e.g., `sliding_window_compactor`, `tiered_compactor`) to decide
what to retain, compress, or discard during compaction.

For the full reference — struct fields, variants, convenience constructors, and how
metadata flows through context assembly — see
`specs/04-operator-turn-runtime.md §Context Assembly`.

## Current Implementation Status

Implemented:

- `neuron-state-memory` — in-memory `StateStore`.
- `neuron-state-fs` — filesystem `StateStore`.
- `StoreOptions`, `MemoryTier`, `Lifetime`, `ContentKind` — defined in `layer0/src/state.rs`.
- `write_hinted` / `read_hinted` on `StateStore` and `StateReader` — defaults delegate to
  unhinted variants; backends override to act on hints.
- `clear_transient` — default no-op; backends that track `Lifetime::Transient` override.
- `Message`, `CompactionPolicy` — defined in `layer0/src/context.rs`
  and `layer0/src/lifecycle.rs` respectively.
- `Effect::WriteMemory` carries the same five advisory fields as `StoreOptions`.

Still required for "core complete":

- Explicit examples and tests demonstrating scope isolation and persistence semantics.
- At least one backend that honors `Lifetime` hints (beyond the default no-op).
