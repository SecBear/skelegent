# Cognitive Memory Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement ACC, ReMe, MemSearcher, and A-MEM patterns across `skg-context-engine` (new context ops) and `skg-state-cozo` (unlocking CozoDB's full capabilities: HNSW vector search, FTS, MinHash-LSH, recursive Datalog graph traversal).

**Architecture:** Two-layer approach. Layer 1: Unlock CozoDB's native proximity indices (HNSW, FTS, MinHash-LSH) and recursive Datalog in `skg-state-cozo`. Layer 2: Build cognitive memory ops in `skg-context-engine` that compose with any `StateStore` backend, with CozoDB as the power backend.

**Tech Stack:** Rust, CozoDB v0.7.6 (HNSW + FTS + MinHash-LSH + Datalog), `skg-context-engine`, `layer0` traits

## Source Material

| Pattern | Paper/Source | Key Concept |
|---------|-------------|-------------|
| ACC | [Bousetouane 2026, Qeios](https://www.qeios.com/read/MZQB3T) | Bounded Compressed Cognitive State (CCS) with schema-governed commitment. Replaces transcript replay |
| ReMe | [Cao et al. 2024, arXiv:2512.10696](https://arxiv.org/html/2512.10696v1) | Dynamic procedural memory lifecycle: distill → adaptive reuse → utility-based refinement |
| MemSearcher | [arXiv:2511.02805](https://arxiv.org/html/2511.02805v1) | Compact iterative memory managed by the LLM itself, trained via multi-context GRPO |
| A-MEM | [Xu et al. 2025, NeurIPS](https://arxiv.org/abs/2502.12110) | Zettelkasten-inspired: note construction + link generation + memory evolution |
| memsearch | [Zilliz, GitHub](https://github.com/zilliztech/memsearch) | Markdown-first, hybrid BM25+vector retrieval, LLM compaction |
| CozoDB Proximity | [CozoDB v0.7 docs](https://docs.cozodb.org/en/latest/vector.html) | HNSW, MinHash-LSH, FTS indices on stored relations |

---

## Phase 1: Unlock CozoDB (skg-state-cozo)

**Why first:** All cognitive memory ops need powerful search. CozoDB has HNSW, FTS, and graph algorithms built in — we just aren't using them. This phase activates them.

**Crate:** `extras/state/skg-state-cozo`

### Task 1.1: Add FTS index to kv relation

**Files:**
- Modify: `extras/state/skg-state-cozo/src/schema.rs`
- Modify: `extras/state/skg-state-cozo/src/store.rs`
- Test: `extras/state/skg-state-cozo/tests/store_tests.rs`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn fts_search_returns_ranked_results() {
    let store = CozoStore::memory().unwrap();
    let scope = Scope::Global;
    store.write(&scope, "doc1", json!("Rust is a systems programming language")).await.unwrap();
    store.write(&scope, "doc2", json!("Python is great for data science")).await.unwrap();
    store.write(&scope, "doc3", json!("Rust and memory safety go together")).await.unwrap();

    let results = store.search(&scope, "Rust programming", 10).await.unwrap();
    assert!(!results.is_empty());
    // doc1 and doc3 mention Rust, doc2 doesn't
    assert!(results.iter().any(|r| r.key == "doc1"));
    assert!(results.iter().any(|r| r.key == "doc3"));
    assert!(results.iter().all(|r| r.key != "doc2"));
    // Scores should be ordered descending
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
}
```

**Step 2:** Run test, expect FAIL (current `search` is substring match, should fail on ranking or miss "programming" without exact substring)

**Step 3: Implement**

In `schema.rs`, add FTS index DDL:
```rust
/// FTS index on the kv relation's value field.
pub const KV_FTS_DDL: &str = r#"::fts create kv:fts_val {
    extractor: value,
    tokenizer: Simple,
    filters: [Lowercase],
}"#;
```

In `store.rs` (real CozoDB backend), replace substring `search` with FTS query:
```rust
// Use ~kv:fts_val for FTS search
let query = r#"
    ?[key, score] := ~kv:fts_val {scope: $scope, key, value | query: $query, score}
    :order -score
    :limit $limit
"#;
```

Run FTS DDL during `init_schema()` after KV_DDL.

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(cozo): activate FTS index on kv relation`

---

### Task 1.2: Add HNSW vector index and vector_search method (Tier 2)

**Files:**
- Modify: `extras/state/skg-state-cozo/src/schema.rs`
- Modify: `extras/state/skg-state-cozo/src/store.rs`
- Test: `extras/state/skg-state-cozo/tests/vector_tests.rs` (new)

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn hnsw_vector_search_returns_nearest() {
    let store = CozoStore::memory().unwrap();
    let scope = Scope::Global;

    // Store entries with embedding vectors
    store.write_node(&scope, "cat", json!({"text": "cats are fluffy"}), &[0.9, 0.1, 0.0]).await.unwrap();
    store.write_node(&scope, "dog", json!({"text": "dogs are loyal"}), &[0.8, 0.2, 0.0]).await.unwrap();
    store.write_node(&scope, "car", json!({"text": "cars have engines"}), &[0.0, 0.1, 0.9]).await.unwrap();

    // Search for something near "cat"
    let results = store.vector_search(&scope, &[0.85, 0.15, 0.0], 2).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].key, "cat");
    assert_eq!(results[1].key, "dog");
}
```

**Step 2:** Run test, expect FAIL (method doesn't exist)

**Step 3: Implement**

New schema in `schema.rs`:
```rust
/// DDL for the node relation with embedding vector.
pub const NODE_V2_DDL: &str =
    ":create node { scope: String, key: String => data: String, node_type: String, salience: Float, embedding: <F32; 1536>, created_at: Float }";

/// HNSW index on node embeddings.
pub const NODE_HNSW_DDL: &str = r#"::hnsw create node:emb_idx {
    dim: 1536,
    m: 16,
    ef_construction: 200,
    fields: [embedding],
    distance: Cosine,
    filter: !is_null(embedding),
}"#;
```

New Tier 2 methods on `CozoStore`:
```rust
/// Write a node with embedding vector (Tier 2 — CozoStore only).
pub async fn write_node(&self, scope: &Scope, key: &str, data: serde_json::Value, embedding: &[f32]) -> Result<(), StateError>;

/// Vector similarity search using HNSW index (Tier 2 — CozoStore only).
pub async fn vector_search(&self, scope: &Scope, query_vector: &[f32], limit: usize) -> Result<Vec<SearchResult>, StateError>;
```

The HNSW query:
```
?[key, data, dist] := ~node:emb_idx {scope: $scope, key, data, embedding | query: $vec, k: $limit, ef: 200, bind_distance: dist}
:order dist
:limit $limit
```

Note: The `1536` dimension is configurable — make it a const defaulting to 1536 (OpenAI ada-002 dimension). Allow override in CozoStore constructor.

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(cozo): HNSW vector search on node relation`

---

### Task 1.3: Hybrid search (FTS + HNSW + RRF fusion)

**Files:**
- Modify: `extras/state/skg-state-cozo/src/store.rs`
- Create: `extras/state/skg-state-cozo/src/search.rs`
- Test: `extras/state/skg-state-cozo/tests/hybrid_tests.rs` (new)

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn hybrid_search_fuses_fts_and_vector() {
    let store = CozoStore::memory().unwrap();
    let scope = Scope::Global;

    // "rust" appears in FTS but vector is about cars
    store.write_node(&scope, "misleading", json!({"text": "rust on car body"}), &[0.0, 0.1, 0.9]).await.unwrap();
    // Strong FTS match AND vector match
    store.write_node(&scope, "relevant", json!({"text": "rust programming language"}), &[0.9, 0.1, 0.0]).await.unwrap();

    let results = store.hybrid_search(&scope, "rust programming", &[0.85, 0.15, 0.0], 10).await.unwrap();
    assert_eq!(results[0].key, "relevant"); // Both signals agree
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

In new `search.rs`:
```rust
/// Reciprocal Rank Fusion score for combining multiple ranked lists.
/// RRF(d) = Σ 1/(k + rank_i(d)) for each ranking i
pub fn rrf_fuse(ranked_lists: &[&[SearchResult]], k: f64) -> Vec<SearchResult>;
```

`hybrid_search` on CozoStore:
1. Run FTS query → ranked list A
2. Run HNSW query → ranked list B
3. RRF fuse with k=60 (standard)
4. Return merged, deduplicated, re-ranked results

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(cozo): hybrid FTS+HNSW search with RRF fusion`

---

### Task 1.4: Replace manual BFS with recursive Datalog in traverse()

**Files:**
- Modify: `extras/state/skg-state-cozo/src/store.rs`
- Test: `extras/state/skg-state-cozo/tests/store_tests.rs` (existing graph tests)

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn traverse_recursive_finds_deep_paths() {
    let store = CozoStore::memory().unwrap();
    let scope = Scope::Global;
    // A -> B -> C -> D (chain of depth 3)
    store.link(&scope, &MemoryLink::new("A", "B", "refs")).await.unwrap();
    store.link(&scope, &MemoryLink::new("B", "C", "refs")).await.unwrap();
    store.link(&scope, &MemoryLink::new("C", "D", "refs")).await.unwrap();

    let depth1 = store.traverse(&scope, "A", Some("refs"), 1).await.unwrap();
    assert_eq!(depth1, vec!["B"]);

    let depth3 = store.traverse(&scope, "A", Some("refs"), 3).await.unwrap();
    assert!(depth3.contains(&"B".to_string()));
    assert!(depth3.contains(&"C".to_string()));
    assert!(depth3.contains(&"D".to_string()));
}
```

**Step 2:** Run test — current BFS impl may pass, but verify correctness. If it passes, this is a refactor for performance.

**Step 3: Implement**

Replace manual BFS loop with single recursive Datalog query:
```
?[to_key] := *edge{scope: $scope, from_key: $start, to_key, relation: $rel}
?[to_key] := *edge{scope: $scope, from_key: mid, to_key, relation: $rel}, ?[mid]
:limit $limit
```

With depth control via Datalog's built-in fixpoint with limit.

**Step 4:** Run ALL existing graph tests, expect PASS

**Step 5:** Commit: `refactor(cozo): replace manual BFS with recursive Datalog traverse`

---

### Task 1.5: Implement clear_transient with dedicated table

**Files:**
- Modify: `extras/state/skg-state-cozo/src/schema.rs`
- Modify: `extras/state/skg-state-cozo/src/store.rs`
- Test: `extras/state/skg-state-cozo/tests/store_tests.rs`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn clear_transient_removes_only_transient_entries() {
    let store = CozoStore::memory().unwrap();
    let scope = Scope::Global;
    let opts_transient = StoreOptions { lifetime: Some(Lifetime::Transient), ..Default::default() };

    store.write(&scope, "durable", json!("stays")).await.unwrap();
    store.write_hinted(&scope, "temp", json!("gone"), &opts_transient).await.unwrap();

    store.clear_transient();

    assert!(store.read(&scope, "durable").await.unwrap().is_some());
    assert!(store.read(&scope, "temp").await.unwrap().is_none());
}
```

**Step 2:** Run test, expect FAIL (clear_transient is no-op)

**Step 3: Implement**

Add `TRANSIENT_DDL` in schema.rs:
```rust
pub const TRANSIENT_DDL: &str =
    ":create transient { scope: String, key: String => value: String, created_at: Float }";
```

Route `write_hinted` with `Lifetime::Transient` to transient table. `read` checks both tables. `clear_transient` purges the transient table.

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(cozo): dedicated transient table with clear_transient`

---

## Phase 2: Compressed Cognitive State (ACC pattern, skg-context-engine)

**Why:** This is the highest-value pattern — bounded memory that resists drift. Builds on existing `SummarizeConfig` and `ExtractConfig`.

**Crate:** `skelegent/op/skg-context-engine`

### Task 2.1: Define CCS schema types

**Files:**
- Create: `skelegent/op/skg-context-engine/src/ops/cognitive.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/mod.rs`
- Test: `skelegent/op/skg-context-engine/tests/cognitive_tests.rs` (new)

**Step 1: Write failing test**

```rust
#[test]
fn ccs_roundtrips_through_json() {
    let ccs = CognitiveState {
        episodic_trace: "User asked about deployment constraints".into(),
        semantic_gist: "Infrastructure migration planning".into(),
        focal_entities: vec![
            Entity { name: "prod-db-01".into(), entity_type: "server".into() },
        ],
        relational_map: vec![
            Relation { from: "prod-db-01".into(), to: "app-cluster".into(), relation: "serves".into() },
        ],
        goal: "Migrate to new region without downtime".into(),
        constraints: vec!["No restarts during business hours".into()],
        predictive_cue: "Next: verify backup completion".into(),
        uncertainty: vec!["Backup ETA unknown".into()],
        artifact_refs: vec!["turn:3:tool:check_backup".into()],
    };
    let json = serde_json::to_value(&ccs).unwrap();
    let restored: CognitiveState = serde_json::from_value(json).unwrap();
    assert_eq!(restored.goal, ccs.goal);
    assert_eq!(restored.constraints.len(), 1);
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

```rust
/// Compressed Cognitive State (CCS) — ACC paper §3.2.
///
/// A bounded, schema-governed internal state that replaces transcript
/// replay. Updated once per turn via controlled replacement.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CognitiveState {
    /// What changed in the current turn (episodic update).
    pub episodic_trace: String,
    /// Dominant intent or topic (semantic abstraction).
    pub semantic_gist: String,
    /// Canonicalized entities with types.
    pub focal_entities: Vec<Entity>,
    /// Causal and temporal dependencies.
    pub relational_map: Vec<Relation>,
    /// Persistent objective guiding the interaction.
    pub goal: String,
    /// Invariant rules, policies, safety constraints.
    pub constraints: Vec<String>,
    /// Expected next cognitive operation.
    pub predictive_cue: String,
    /// Unresolved or low-confidence elements.
    pub uncertainty: Vec<String>,
    /// References to external evidence (not internalized).
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub relation: String,
}
```

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): CognitiveState type (ACC CCS schema)`

---

### Task 2.2: CompressCognitiveState op (ACC compression)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/cognitive.rs`
- Test: `skelegent/op/skg-context-engine/tests/cognitive_tests.rs`

**Step 1: Write failing test**

```rust
#[test]
fn compress_cognitive_state_builds_valid_request() {
    let config = CompressCognitiveStateConfig::default();
    let prev_ccs = CognitiveState { goal: "migrate DB".into(), ..Default::default() };
    let messages = vec![
        Message::new(Role::User, "The backup finished."),
        Message::new(Role::Assistant, "Great, I'll proceed with the cutover."),
    ];

    let request = config.build_request(&messages, Some(&prev_ccs), &[]);
    assert!(request.system.is_some());
    let system = request.system.unwrap();
    assert!(system.contains("CognitiveState")); // schema in prompt
    assert!(system.contains("migrate DB")); // previous CCS included
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

`CompressCognitiveStateConfig` follows the same DIY-first pattern as `SummarizeConfig`:
- `build_request(&messages, prev_ccs, recalled_artifacts) -> InferRequest` — builds the prompt for the CCM (Cognitive Compressor Model)
- `parse_response(response) -> Result<CognitiveState>` — parses LLM output into CCS

The prompt includes:
1. The CCS JSON schema
2. The previous CCS (if any)
3. The current turn's messages
4. Qualified recalled artifacts
5. Instructions to produce a new CCS conforming to schema

Then the context op:
```rust
/// ACC-style cognitive compression. Builds an LLM request to produce
/// a new CognitiveState from the current messages and previous state.
///
/// This is the DIY primitive — call `build_request` and `parse_response`
/// yourself, or use `CommitCognitiveState` for the full op.
pub struct CompressCognitiveState {
    pub config: CompressCognitiveStateConfig,
    pub previous: Option<CognitiveState>,
    pub recalled_artifacts: Vec<String>,
}
```

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): CompressCognitiveState op (ACC §3.1)`

---

### Task 2.3: CommitCognitiveState op (state replacement)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/cognitive.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/store.rs`
- Test: `skelegent/op/skg-context-engine/tests/cognitive_tests.rs`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn commit_replaces_previous_state() {
    let store = MemoryStore::new();
    let scope = Scope::Global;
    let ccs1 = CognitiveState { goal: "first".into(), ..Default::default() };
    let ccs2 = CognitiveState { goal: "second".into(), ..Default::default() };

    CommitCognitiveState::commit(&store, &scope, &ccs1).await.unwrap();
    let loaded = CommitCognitiveState::load(&store, &scope).await.unwrap();
    assert_eq!(loaded.unwrap().goal, "first");

    CommitCognitiveState::commit(&store, &scope, &ccs2).await.unwrap();
    let loaded = CommitCognitiveState::load(&store, &scope).await.unwrap();
    assert_eq!(loaded.unwrap().goal, "second");
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

```rust
/// Writes/overwrites the CognitiveState to the store under a well-known key.
/// ACC principle: state is REPLACED, never accumulated.
pub struct CommitCognitiveState;

impl CommitCognitiveState {
    const KEY: &'static str = "__skg_cognitive_state";

    pub async fn commit(store: &dyn StateStore, scope: &Scope, state: &CognitiveState) -> Result<(), StateError> {
        let json = serde_json::to_value(state).map_err(|e| StateError::Serialization(e.to_string()))?;
        store.write(scope, Self::KEY, json).await
    }

    pub async fn load(store: &dyn StateStore, scope: &Scope) -> Result<Option<CognitiveState>, StateError> {
        match store.read(scope, Self::KEY).await? {
            Some(val) => Ok(Some(serde_json::from_value(val).map_err(|e| StateError::Serialization(e.to_string()))?)),
            None => Ok(None),
        }
    }
}
```

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): CommitCognitiveState op (ACC §3.3 state replacement)`

---

## Phase 3: Recall Qualification Gate (ACC artifact filtering)

### Task 3.1: QualifyRecall op

**Files:**
- Create: `skelegent/op/skg-context-engine/src/ops/qualify.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/mod.rs`
- Test: `skelegent/op/skg-context-engine/tests/qualify_tests.rs` (new)

**Step 1: Write failing test**

```rust
#[test]
fn qualify_recall_builds_request_with_candidates() {
    let config = QualifyRecallConfig::default();
    let ccs = CognitiveState { goal: "migrate DB".into(), ..Default::default() };
    let candidates = vec![
        RecalledArtifact { key: "runbook-v1".into(), snippet: "Restart the service".into() },
        RecalledArtifact { key: "backup-log".into(), snippet: "Backup completed at 14:00".into() },
    ];

    let request = config.build_request(&ccs, &candidates);
    assert!(request.system.is_some());
    // Should ask LLM to filter candidates by decision-relevance
}

#[test]
fn qualify_recall_parses_approved_list() {
    let config = QualifyRecallConfig::default();
    let response_text = r#"{"approved": ["backup-log"]}"#;
    let approved = config.parse_response(response_text).unwrap();
    assert_eq!(approved, vec!["backup-log"]);
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

ACC §3.1 qualification gate: retrieval proposes, qualification filters. Only decision-relevant artifacts pass through to CCS commitment.

```rust
pub struct QualifyRecallConfig {
    pub prompt_template: String,
}

pub struct RecalledArtifact {
    pub key: String,
    pub snippet: String,
}
```

DIY primitives:
- `build_request(ccs, candidates) -> InferRequest`
- `parse_response(text) -> Result<Vec<String>>` (returns approved keys)

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): QualifyRecall op (ACC §3.1 decision gate)`

---

## Phase 4: Memory Evolution (A-MEM + ReMe patterns)

### Task 4.1: MemoryNote type (A-MEM Zettelkasten notes)

**Files:**
- Create: `skelegent/op/skg-context-engine/src/ops/memory_note.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/mod.rs`
- Test: `skelegent/op/skg-context-engine/tests/memory_note_tests.rs` (new)

**Step 1: Write failing test**

```rust
#[test]
fn memory_note_serializes_with_metadata() {
    let note = MemoryNote {
        key: "note-001".into(),
        content: "User prefers conservative deployment strategies".into(),
        keywords: vec!["deployment".into(), "conservative".into()],
        tags: vec!["preference".into(), "operations".into()],
        description: "User's risk tolerance for deployments".into(),
        source_turn: Some(5),
        created_at: 1710000000.0,
    };
    let json = serde_json::to_value(&note).unwrap();
    assert_eq!(json["keywords"][0], "deployment");
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement** the `MemoryNote` struct with full A-MEM attributes.

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): MemoryNote type (A-MEM §3.1)`

---

### Task 4.2: ConstructNote op (extract structured note from interaction)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/memory_note.rs`
- Test: `skelegent/op/skg-context-engine/tests/memory_note_tests.rs`

DIY primitives:
- `ConstructNoteConfig::build_request(messages) -> InferRequest`
- `ConstructNoteConfig::parse_response(text) -> Result<MemoryNote>`

**Commit:** `feat(context-engine): ConstructNote op (A-MEM §3.1 note construction)`

---

### Task 4.3: LinkGeneration op (find + create links between notes)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/memory_note.rs`
- Test: `skelegent/op/skg-context-engine/tests/memory_note_tests.rs`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn link_generation_finds_related_notes() {
    let store = MemoryStore::new();
    let scope = Scope::Global;

    // Store existing notes
    store.write(&scope, "note-001", json!({"content": "Rust memory safety", "keywords": ["rust", "safety"]})).await.unwrap();
    store.write(&scope, "note-002", json!({"content": "Python GIL limitations", "keywords": ["python", "concurrency"]})).await.unwrap();

    let new_note = MemoryNote {
        content: "Rust's borrow checker prevents data races".into(),
        keywords: vec!["rust".into(), "concurrency".into(), "borrow-checker".into()],
        ..Default::default()
    };

    let config = LinkGenerationConfig::default();
    let links = config.find_related(&store, &scope, &new_note).await.unwrap();
    // Should link to note-001 (Rust + safety) and note-002 (concurrency)
    assert!(links.iter().any(|l| l.to == "note-001"));
}
```

**Step 2:** Run test, expect FAIL

**Step 3: Implement**

Uses keyword overlap for candidate generation, then LLM to analyze shared attributes:
- `find_related(store, scope, note) -> Result<Vec<MemoryLink>>`
- On CozoStore: uses FTS/vector search for candidates
- On other stores: falls back to keyword-based `search()`

**Step 4:** Run test, expect PASS

**Step 5:** Commit: `feat(context-engine): LinkGeneration op (A-MEM §3.2)`

---

### Task 4.4: EvolveMemory op (update existing notes when new evidence arrives)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/memory_note.rs`
- Test: `skelegent/op/skg-context-engine/tests/memory_note_tests.rs`

This is the A-MEM §3.3 / ReMe refinement pattern. When a new note is added, linked existing notes may need updating.

DIY primitives:
- `EvolveMemoryConfig::build_request(new_note, existing_note) -> InferRequest`
- `EvolveMemoryConfig::parse_response(text) -> Result<Option<MemoryNote>>` (None = no update needed)

**Commit:** `feat(context-engine): EvolveMemory op (A-MEM §3.3 + ReMe refinement)`

---

## Phase 5: Procedural Memory (ReMe pattern)

### Task 5.1: ProceduralMemory type and DistillProcedure op

**Files:**
- Create: `skelegent/op/skg-context-engine/src/ops/procedural.rs`
- Modify: `skelegent/op/skg-context-engine/src/ops/mod.rs`
- Test: `skelegent/op/skg-context-engine/tests/procedural_tests.rs` (new)

```rust
/// A reusable procedure distilled from successful tool sequences.
/// ReMe §3.1 experience acquisition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    /// Unique identifier.
    pub key: String,
    /// Human-readable description of what this procedure does.
    pub description: String,
    /// Ordered sequence of tool names/actions.
    pub steps: Vec<ProcedureStep>,
    /// Success count (utility tracking for refinement).
    pub success_count: u32,
    /// Failure count.
    pub failure_count: u32,
    /// Keywords for retrieval.
    pub keywords: Vec<String>,
    /// When this was last successfully used.
    pub last_used: Option<f64>,
}
```

DIY primitives:
- `DistillProcedureConfig::build_request(tool_trace) -> InferRequest`
- `DistillProcedureConfig::parse_response(text) -> Result<Procedure>`

**Commit:** `feat(context-engine): ProceduralMemory + DistillProcedure op (ReMe §3.1)`

---

### Task 5.2: RecallProcedure op (context-adaptive reuse)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/procedural.rs`
- Test: `skelegent/op/skg-context-engine/tests/procedural_tests.rs`

Given current context, recall relevant procedures from the store and inject as guidance.

DIY primitives:
- `RecallProcedureConfig::build_query(messages) -> String` (search query for store)
- `RecallProcedureConfig::format_guidance(procedures) -> Message` (inject into context)

**Commit:** `feat(context-engine): RecallProcedure op (ReMe §3.2 adaptive reuse)`

---

### Task 5.3: RefineProcedure op (utility-based maintenance)

**Files:**
- Modify: `skelegent/op/skg-context-engine/src/ops/procedural.rs`
- Test: `skelegent/op/skg-context-engine/tests/procedural_tests.rs`

After task execution, update procedure success/failure counts. Prune low-utility procedures. Merge similar ones.

**Commit:** `feat(context-engine): RefineProcedure op (ReMe §3.3 utility refinement)`

---

## Phase 6: Integration & Full-Loop Tests

### Task 6.1: ACC full loop test (compress → qualify → commit → condition)

**Files:**
- Create: `skelegent/op/skg-context-engine/tests/acc_integration_test.rs`

Test the full ACC cycle:
1. Start with empty CCS
2. Process turn 1 → compress → commit CCS_1
3. Process turn 2 with CCS_1 → recall artifacts → qualify → compress → commit CCS_2
4. Verify CCS_2 preserves constraints from turn 1
5. Verify CCS token size stays bounded

**Commit:** `test(context-engine): ACC full-loop integration test`

---

### Task 6.2: CozoDB hybrid search integration test

**Files:**
- Create: `extras/state/skg-state-cozo/tests/hybrid_integration_test.rs`

Test the full CozoDB capability stack:
1. Write nodes with embeddings
2. Create FTS-indexed content
3. Link nodes via edges
4. Hybrid search (FTS + HNSW + RRF)
5. Recursive Datalog traverse
6. Verify results combine all signals

**Commit:** `test(cozo): full hybrid search integration test`

---

### Task 6.3: Memory evolution with CozoDB graph

**Files:**
- Create: `extras/state/skg-state-cozo/tests/memory_evolution_test.rs`

Test A-MEM patterns on CozoDB:
1. Store 3 notes as nodes with embeddings
2. Auto-link via vector similarity
3. Add a new note → trigger evolution of linked notes
4. Traverse the note graph
5. Verify salience updates

**Commit:** `test(cozo): memory evolution with graph traversal`

---

### Task 6.4: Strip misleading docs from CozoStore

**Files:**
- Modify: `extras/state/skg-state-cozo/src/lib.rs`
- Modify: `extras/state/skg-state-cozo/src/store.rs`

Replace "planned for v2" comments with documentation of actual capabilities. Remove dead `node` DDL (replaced by `NODE_V2_DDL` with embeddings).

**Commit:** `docs(cozo): update docs to reflect actual v2 capabilities`

---

## Verification

After all phases:

```bash
cd skelegent && nix develop --command cargo test --workspace --all-targets
cd extras && nix develop --command cargo test --workspace --all-targets
cd skelegent && nix develop --command cargo clippy --workspace -- -D warnings
cd extras && nix develop --command cargo clippy --workspace -- -D warnings
```

All tests must pass. Zero warnings.

---

## Summary

| Phase | Tasks | Crate | Pattern |
|-------|-------|-------|---------|
| 1 | 1.1–1.5 | skg-state-cozo | Unlock CozoDB: FTS, HNSW, hybrid search, recursive Datalog, transient table |
| 2 | 2.1–2.3 | skg-context-engine | ACC: CognitiveState type, compress op, commit op |
| 3 | 3.1 | skg-context-engine | ACC: QualifyRecall gate |
| 4 | 4.1–4.4 | skg-context-engine | A-MEM: MemoryNote, ConstructNote, LinkGeneration, EvolveMemory |
| 5 | 5.1–5.3 | skg-context-engine | ReMe: ProceduralMemory, DistillProcedure, RecallProcedure, RefineProcedure |
| 6 | 6.1–6.4 | both | Integration tests + doc cleanup |

**Total: 18 tasks, ~20 new test files, 2 crates modified**
