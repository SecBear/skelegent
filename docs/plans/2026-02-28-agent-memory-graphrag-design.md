# Agent Memory System: Hybrid GraphRAG on CozoDB

**Date**: 2026-02-28
**Status**: Approved
**Scope**: `neuron-state-graph` — new crate implementing `StateStore` backed by CozoDB with GraphRAG retrieval

## Problem

Neuron's existing state backends (`neuron-state-memory`, `neuron-state-fs`) are key-value stores with no search capability. Agents in the research-to-spec-to-build pipeline need:

- **Knowledge accumulation**: Research agents produce interconnected findings at scale
- **Relationship discovery**: Connections between findings matter as much as the findings themselves
- **Multi-hop retrieval**: "Find patterns related to memory systems that work with Rust"
- **Temporal reasoning**: "What did we know about X last Tuesday?"
- **Cross-agent handoff**: Research findings become specs become build artifacts

Flat files and key-value stores collapse structure into text. A Hybrid GraphRAG architecture — combining knowledge graphs, vector similarity, and full-text search — addresses all of these.

## Decision: CozoDB as Storage Engine

### Why CozoDB

CozoDB is the only mature, Rust-native, embedded engine that combines graph traversal (recursive Datalog), HNSW vector search, and full-text search in a single query language. Vector search is a *predicate inside Datalog*, enabling queries that chain semantic similarity with graph traversal atomically.

Evaluated alternatives (Feb 2026):

| Engine | Why Not |
|--------|---------|
| SQLite + sqlite-vec + FTS5 | No native graph — recursive CTEs are limited vs Datalog |
| SurrealDB 3.0 | HNSW lock starvation + 2000x WHERE regression (Issue #6800); BSL license |
| Kuzu | Archived Oct 2025 (Apple acquisition) |
| LanceDB + custom graph | Excellent vectors but no graph; composite stack = more complexity |
| DuckDB | OLAP-focused, not transactional agent memory |

### Known Risks

- **Maintenance**: Last release v0.7.6 (Dec 2023). Single maintainer, minimally responsive.
- **API**: String-based Datalog scripts, no type safety. Requires thick wrapper.
- **Pre-1.0**: No API/storage stability guarantees.
- **Bus factor**: 1. Deep query planner bugs would require understanding someone else's Datalog optimizer.

### Mitigations

1. **Pin v0.7.6 and vendor the dependency** — don't rely on upstream existing forever
2. **Adapter pattern** — `StateStore` trait means CozoDB is an implementation detail, swappable
3. **Don't fork immediately** — use as-is first; fork only for concrete type safety or feature needs
4. **Keep LanceDB in back pocket** — if vector scale exceeds 100K with latency requirements, add as sidecar
5. **License**: MPL-2.0 (file-level copyleft). Modifications to CozoDB source files must be open-sourced. Adapter layer is unaffected.

## Architecture

```
                         ┌─────────────────────────────────┐
                         │        Agent (Operator)          │
                         │   research / planning / builder  │
                         └──────┬────────────────┬──────────┘
                                │ read           │ Effects
                    ┌───────────▼──────┐  ┌──────▼──────────┐
                    │   StateReader    │  │  EffectExecutor  │
                    │  (read, search,  │  │  (write, delete, │
                    │   list)          │  │   link, unlink)  │
                    └───────────┬──────┘  └──────┬──────────┘
                                │                │
                    ┌───────────▼────────────────▼──────────┐
                    │         neuron-state-graph             │
                    │                                        │
                    │  ┌──────────┐  ┌───────────────────┐  │
                    │  │  Schema  │  │   Retrieval        │  │
                    │  │  Layer   │  │  (CozoDB candidates │  │
                    │  │ (nodes,  │  │   + Rust ranking)  │  │
                    │  │  edges,  │  │                    │  │
                    │  │  layers) │  │                    │  │
                    │  └──────────┘  └───────────────────┘  │
                    │       ┌────────────────────┐          │
                    │       │  Scope Isolation    │          │
                    │       └────────┬───────────┘          │
                    └────────────────┼──────────────────────┘
                                     │
                    ┌────────────────▼─────────────────────┐
                    │            CozoDB Engine              │
                    │  Datalog | HNSW | FTS | SQLite/Mem   │
                    └──────────────────────────────────────┘
```

### Key Integration Points

- `StateStore::read/write/delete/list` → CozoDB CRUD (zero impedance mismatch)
- `StateStore::search` → Hybrid GraphRAG (vector + BM25 + graph expansion → RRF + rerank in Rust)
- Scope isolation → Composite key `(scope_string, key)` in CozoDB relations
- Effects → `WriteMemory` creates nodes + generates embeddings; `LinkMemory`/`UnlinkMemory` create/remove edges

## Schema Design

### Memory Layers

Inspired by MindGraph's 6-layer cognitive architecture, simplified from 48 to ~20 node types:

| Layer | Node Types | Purpose in Pipeline |
|-------|-----------|---------------------|
| **Reality** (4) | Source, Entity, Snippet, Observation | What research agents find |
| **Epistemic** (7) | Claim, Evidence, Concept, Hypothesis, Pattern, Question, Model | What agents know/reason about |
| **Intent** (4) | Goal, Decision, Project, Spec | What gets built and why |
| **Memory** (3) | Session, Summary, Preference | Cross-session continuity |
| **Agent** (3) | Agent, Task, Plan | Multi-agent coordination |

All types support `Custom(String)` escape hatch.

### Edge Types (~25)

| Category | Edges |
|----------|-------|
| Structural | ExtractedFrom, PartOf, DerivedFrom, Contains |
| Epistemic | Supports, Refutes, Contradicts, DependsOn, RelatesTo |
| Intent | DecomposesInto, MotivatedBy, Blocks, Informs, Targets |
| Memory | CapturedIn, Summarizes, Recalls |
| Agent | AssignedTo, PlannedBy, ProducedBy |
| Temporal | SupersededBy, InvalidatedAt |
| Custom | Custom(String) |

### Bi-Temporal Model

Every edge carries:
- `valid_at` / `invalid_at` — when the fact was true in reality
- `created_at` — when it was ingested into the graph

Enables: "What did we know about X at time T?"

### CozoDB Relations

```
node     { uid: String => node_type, layer, label, summary, props: Json,
           salience: Float, is_tombstoned: Bool, created_at: Float, ... }
edge     { uid: String => from_uid, to_uid, edge_type, weight: Float,
           valid_at: Float, invalid_at: Float, is_tombstoned: Bool, props: Json, ... }
node_ver { node_uid: String, version: Int => snapshot: Json, ... }
edge_ver { edge_uid: String, version: Int => snapshot: Json, ... }
alias    { alias_text: String, canonical_uid: String => ... }
```

Plus HNSW index on node embeddings and FTS indices on label + summary.

## Retrieval Pipeline

### Three Modes

| Mode | Use Case | Agent Stage |
|------|----------|-------------|
| **Local** | Targeted: "What's the spec for feature X?" | Builder, Spec |
| **Global** | Thematic: "What themes have emerged?" | Research |
| **DRIFT** | Exploratory: "What do we know about X?" | Planning |

### Local Search Pipeline (primary mode)

```
  Query
    │
    ▼
  [Optional: Query Decomposition for complex queries]
    │
    ▼
  ┌─────────── CozoDB (Candidate Generation) ──────┐
  │                                                  │
  │  HNSW top-20 ──┐                               │
  │                 ├── Graph Expand 1-2 hops       │
  │  BM25 top-20 ──┘   (temporal filter applied)   │
  │                                                  │
  │  Output: candidates + raw scores                │
  └──────────────────┬──────────────────────────────┘
                     │
  ┌──────────────────▼────────── Rust ──────────────┐
  │                                                  │
  │  1. RRF Fusion (vector rank + BM25 rank + graph) │
  │  2. MMR Diversity filter                         │
  │  3. Cross-encoder rerank (optional, for builders)│
  │  4. Salience weighting                           │
  │                                                  │
  │  Output: Vec<SearchResult>                       │
  └──────────────────────────────────────────────────┘
```

CozoDB handles candidate generation (what it's great at). Rust handles scoring/ranking (what it's great at).

### Stage-Aware Defaults

| Agent Stage | Mode | Initial k | Reranking | Temporal Filter |
|-------------|------|-----------|-----------|-----------------|
| Research | Global/DRIFT | 20 | MMR (diversity) | Relaxed (include superseded) |
| Planning/Spec | Local | 15 | RRF + MMR | Default (valid only) |
| Builder | Local | 10 | RRF + Cross-encoder | Strict (latest valid only) |

### Scoring: Reciprocal Rank Fusion

```
RRF(d) = 1/(k + rank_vector(d)) + 1/(k + rank_bm25(d)) + 1/(k + rank_graph(d))
where k = 60
```

### Global Search

Uses Leiden algorithm for community detection. Communities re-detected when graph churn exceeds ~5-10% or on daily cadence. Community summaries generated by LLM, stored as dedicated nodes with their own embeddings. Map-reduce pattern: query against all community summaries, LLM scores each, aggregate top results.

### DRIFT Search

Validated by Microsoft Research (78% comprehensiveness improvement, 81% diversity improvement over Local). Pattern: Global primer → local follow-ups → cap at 2 iterations to control latency.

## Scope Isolation & Multi-Agent Memory

### Scope Mapping

```
neuron Scope                 CozoDB key prefix           Usage
────────────                 ─────────────────           ─────
Global                       "global"                    Shared knowledge
Session("s-abc")             "session:s-abc"             Conversation context
Workflow("wf-1")             "workflow:wf-1"             Research project
Agent{wf:"wf-1", a:"plan"}  "agent:wf-1:plan"           Per-agent private
Custom("research/topicX")    "custom:research/topicX"    Pipeline stages
```

### Pipeline-Stage Namespaces

```
research/topicX  ──promote──►  specs/featureY  ──promote──►  builds/featureY
(research agents)    review    (spec agents)      review     (builder agents)
```

Promote = subgraph extraction + re-scoping (preserves edges). Human review at each boundary.

## Compaction & Sleep-Time Compute

Sleep-time agent is a standard `Operator` dispatched by the orchestrator on schedule:

1. **Community re-detection** — Leiden on changed subgraphs
2. **Entity resolution** — merge duplicates via alias + embedding similarity
3. **Summary refresh** — LLM re-summarizes changed communities
4. **Salience decay** — `salience *= exp(-λ * Δt)`, auto-tombstone below threshold
5. **Pattern extraction** — promote recurring patterns to permanent nodes
6. **Contradiction detection** — flag Supports + Refutes to same Claim for human review

### Salience Decay

```
decay(base, last_accessed, half_life) = base * exp(-ln(2)/half_life * elapsed)
```

Default half-life: 1 week. Configurable per scope.

## Embedding Strategy

- **Dimension**: 1024d (100K nodes @ 1024d FP32 ≈ 400MB — negligible)
- **What to embed**: Node labels + summaries. Community summaries get separate embeddings for Global search.
- **Provider**: Pluggable `EmbeddingProvider` trait. Start with API-based (Anthropic/OpenAI), add local (Ollama) later.
- **When**: On `write()` — every searchable node gets embedded at write time.

## New Effect Variants

```rust
pub enum Effect {
    // Existing
    WriteMemory { scope, key, value },
    DeleteMemory { scope, key },
    // New
    LinkMemory { scope, from_key, relation, to_key, metadata },
    UnlinkMemory { scope, from_key, relation, to_key },
}
```

## Extension Trait

```rust
#[async_trait]
pub trait GraphStateStore: StateStore {
    async fn link(&self, scope, from_key, relation, to_key, metadata) -> Result<()>;
    async fn unlink(&self, scope, from_key, relation, to_key) -> Result<()>;
    async fn traverse(&self, scope, from_key, relation, depth, limit) -> Result<Vec<TraversalResult>>;
}
```

Added when operators need explicit graph operations. `StateStore` core trait unchanged.

## Crate Structure

```
neuron-state-graph/               # ~3-4K SLoC for v1
├── Cargo.toml                    # deps: cozo, serde, async-trait
├── src/
│   ├── lib.rs                    # pub mod + re-exports
│   ├── store.rs                  # impl StateStore + impl GraphStateStore
│   ├── engine.rs                 # CozoEngine: typed wrapper over DbInstance
│   ├── schema/
│   │   ├── mod.rs                # Layer, NodeType (~20), EdgeType (~25)
│   │   ├── node.rs               # GraphNode, CreateNode, NodeProps
│   │   ├── edge.rs               # GraphEdge, CreateEdge, EdgeProps
│   │   └── temporal.rs           # BiTemporal, valid_at/invalid_at
│   ├── retrieval/
│   │   ├── mod.rs                # RetrievalMode, SearchConfig
│   │   ├── local.rs              # Seed → Expand → RRF → MMR
│   │   ├── global.rs             # Community summaries → map-reduce
│   │   └── hybrid.rs             # Score fusion (RRF, MMR, cross-encoder)
│   ├── scope.rs                  # Scope → CozoDB key mapping
│   ├── embedding.rs              # EmbeddingProvider trait
│   ├── compaction.rs             # Salience decay, entity resolution
│   ├── migration.rs              # CozoDB schema DDL
│   └── query.rs                  # Typed Datalog query builder
└── tests/
    ├── store_tests.rs            # StateStore trait compliance
    ├── graph_tests.rs            # Graph operations
    ├── retrieval_tests.rs        # Search pipeline
    └── compaction_tests.rs       # Sleep-time compute
```

## Research Sources

- [LangChain Agent Builder Memory System](https://blog.langchain.com/how-we-built-agent-builders-memory-system/) — COALA framework, filesystem-as-memory
- [Tacnode: Three Memory Layers](https://tacnode.io/post/ai-agent-memory-architecture-explained) — Episodic/Semantic/State
- [Letta: Benchmarking Agent Memory](https://www.letta.com/blog/benchmarking-ai-agent-memory) — Filesystem beats specialized tools for retrieval
- [Zep/Graphiti](https://github.com/getzep/graphiti) — Bi-temporal KG, 94.8% DMR, 300ms P95
- [Microsoft DRIFT Search](https://www.microsoft.com/en-us/research/blog/introducing-drift-search-combining-global-and-local-search-methods-to-improve-quality-and-efficiency/) — 78% comprehensiveness improvement
- [LazyGraphRAG](https://www.microsoft.com/en-us/research/blog/lazygraphrag-setting-a-new-standard-for-quality-and-cost/) — 700x cheaper indexing
- [CozoDB](https://github.com/cozodb/cozo) — Embedded Datalog + HNSW + FTS
- [MindGraph-rs](https://github.com/shuruheel/mindgraph-rs) — Schema inspiration (6-layer cognitive architecture)
- [SAGE: Structure Aware Graph Expansion](https://arxiv.org/html/2602.16964v1) — Percentile pruning for graph expansion
- [Leiden Algorithm](https://www.nature.com/articles/s41598-019-41695-z) — Guarantees well-connected communities
- [Zep: How to Search a Knowledge Graph](https://blog.getzep.com/how-do-you-search-a-knowledge-graph/) — RRF fusion, reranking recipes
