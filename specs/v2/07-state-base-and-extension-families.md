# State Base and Extension Families

## Purpose

Prevent the state protocol from growing into a monolithic trait while still
supporting richer backends.

## Base Trait

The base state trait remains intentionally small:

- `read`
- `write`
- `delete`
- `list`

`StateReader` remains the read-only capability view.

## Extension Families

Optional capabilities live in extension traits, including:

- `SearchState`
- `GraphState`
- `TransactionalState`
- `WatchableState`
- `BlobState`
- `VersionedState`
- `LeaseState`

Backends implement only what they support.

## Rust Ergonomics

The base trait must expose typed capability accessors instead of forcing
callers into downcasting:

```rust
fn as_searchable(&self) -> Option<&dyn SearchState> { None }
fn as_graph(&self) -> Option<&dyn GraphState> { None }
```

This keeps common in-process use ergonomic while preserving trait-family
separation.

## Scope and Hint Rules

V2 preserves:

- scope as part of the keyspace
- deterministic `list`
- advisory read/write hints

V2 moves richer behavior into extension traits rather than repeatedly adding
default methods to the base trait.

## Relationship to Current Specs

This spec supersedes `specs/07-state-core.md` for the v2 track while preserving
scope isolation, deterministic listing, and advisory hints.

## Minimum Proving Tests

- A base-only backend remains valid and compiles without extension support.
- A search-capable backend exposes search through typed accessors without downcasting.
- Unsupported extensions degrade predictably without breaking base trait behavior.
