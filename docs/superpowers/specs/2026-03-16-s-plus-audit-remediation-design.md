# S+ Audit Remediation — Design Spec

**Date**: 2026-03-16
**Scope**: skelegent + extras
**Goal**: Bring every corner of both repositories to S+ tier engineering — proven by tests, closed architectural gaps, and polished APIs.

## Context

A comprehensive 5-agent audit identified 20 findings across skelegent and extras:
- 6 test coverage gaps (Tier 1 — prove the architecture works)
- 1 composability blocker (Tier 2 — secret middleware)
- 3 architecture refinements (Tier 3 — OTel bridge, HITL mutation, handoff fix)
- 10 polish items (Tier 4 — derives, parsers, stubs, naming)

No CRITICALs. Zero unsafe code. 1,159 tests passing. The codebase is already clean — this work makes it exceptional.

## Approach: Research-Gated Phased Implementation

Each phase follows:

```
Research (3 tiers) → Validate → Implement → Verify
```

### Research Tiers

Every phase includes three tiers of research via parallel sub-agents:

- **Tier A — Targeted**: How do best-in-class systems solve the specific problem? (parallel-search)
- **Tier B — Exploratory**: What patterns might we be missing? What alternatives exist? (parallel-search)
- **Tier C — Unknown unknowns**: What has gone wrong in production for others? What don't we know we don't know? (parallel-deep-research)

Research results are validated against skelegent's core philosophies before implementation:
1. Protocol, not framework
2. Continuation-passing middleware
3. Effects as declarations, not executions
4. Terraform provider model (one crate, one concern)
5. Zero-cost for paths you don't use

Research can change scope — items may be added, modified, or removed based on findings.

### Effort Key

| Size | Meaning |
|---|---|
| S | < 1 hour — single file, mechanical change |
| M | 1-4 hours — new test module or small feature |
| L | 4-8 hours — new implementation + prerequisite work |
| XL | 8+ hours — design-dependent, touches core protocol |

---

## Phase 1: Prove It Works

**Goal**: Write the tests that prove skelegent's architecture works, not just compiles.

### Research

**Tier A — Targeted:**
1. How Temporal tests replay cancellation propagation — their `TestWorkflowEnvironment` pattern
2. How Restate tests durable execution streaming — journal-based replay verification
3. How Tower tests layered Service composition — `ServiceExt` testing patterns
4. How LangGraph tests graph state traversal — checkpoint graph operation tests

**Tier B — Exploratory:**
5. Best practices for testing async event pipelines in Rust — tokio::select! branch coverage, channel forwarding, cancellation verification
6. Property-based testing for middleware stacks — proptest/quickcheck for random middleware orderings and invariant verification
7. Chaos testing patterns for orchestration frameworks — dropped connections, partial failures, out-of-order delivery
8. Graph operation testing patterns — how graph databases (Neo4j, DGraph) verify traversal correctness, property-based testing for graph invariants

**Tier C — Unknown unknowns:**
9. Test failures that bit Temporal/Restate/LangGraph users in production — post-mortems, GitHub issues, blog posts about testing gaps
10. Rust testing patterns beyond cargo test — tracing-test, tokio-test, test harness patterns

### Items

#### #1 — DispatchHandle::intercept() cancellation propagation
- **Repo**: skelegent
- **File**: `layer0/src/dispatch.rs` (add tests inside `#[cfg(test)] mod tests` block)
- **Effort**: M
- **Tests**:
  - Create inner handle, intercept it, cancel outer handle, assert inner's `cancelled()` future resolves
  - Create intercepted handle, drop outer consumer mid-stream, assert inner pump task exits cleanly
  - Interceptor callback that panics — verify consumer still gets events or clean error
- **Code under test**: Lines 397-404 (tokio::select! cancellation branch)

#### #2 — MiddlewareProvider StreamProvider path
- **Repo**: skelegent
- **File**: `provider/skg-provider-router/src/lib.rs` (add test module)
- **Effort**: M
- **Tests**:
  - Build MiddlewareProvider with InferMiddleware observer, invoke `stream_infer()`, assert middleware fired
  - Same for embed path
  - Empty middleware stack passes through cleanly
- **Depends on**: Understanding how the middleware intercept pattern works (conceptual dependency on #1's pattern, not on #1's implementation — these are different crates)

#### #3 — skg-orch-local unit tests
- **Repo**: skelegent
- **File**: `orch/skg-orch-local/src/lib.rs` (add `#[cfg(test)] mod tests`)
- **Effort**: M
- **Tests**:
  - Register operator, dispatch, collect output
  - Dispatch to unregistered operator returns error
  - Concurrent dispatches don't deadlock
  - Signal journal records and retrieves
  - Middleware stack integration

#### #4 — Graph operations end-to-end
- **Repo**: skelegent
- **File**: `layer0/tests/phase1.rs` (add graph tests)
- **Effort**: L
- **Prerequisite**: InMemoryStore (`layer0/src/test_utils/in_memory_store.rs`) does NOT implement link/unlink/traverse — it inherits default error-returning stubs. Sub-task: implement graph ops on InMemoryStore (~40 lines, backed by `HashMap<(scope, from_key, to_key, relation), ()>`)
- **Tests**:
  - Link two entries, traverse, find target
  - Unlink, traverse, target not found
  - Traverse with max_depth constraint
  - Traverse with relation filter
  - Test through effect handler pipeline (Effect::LinkMemory → StateStore::link)

#### #5 — EvalRunner concurrency verification
- **Repo**: extras
- **File**: `eval/skg-eval/src/lib.rs` (add concurrency tests)
- **Effort**: M
- **Tests**:
  - N slow cases (sleep 100ms each), concurrency=N, assert wall time < N*100ms
  - N+M cases with concurrency=N, AtomicUsize counter proves at most N run simultaneously
  - One case panics, others still complete with correct results

#### #6 — A2A SSE terminal states
- **Repo**: extras
- **File**: `a2a/skg-a2a/tests/streaming.rs` (add terminal state tests)
- **Effort**: M
- **Tests**:
  - Failed status_update → DispatchEvent::Failed
  - Canceled status_update → DispatchEvent::Failed
  - Rejected status_update → DispatchEvent::Failed
  - Progress → status_update → Progress round-trip with state verification

### Parallelism

```
[#1 || #3 || #4 || #5 || #6] → #2
```

Items #1, #3, #4 are in skelegent (different crates, no shared files).
Items #5, #6 are in extras (different crates).
Item #2 depends on understanding #1's intercept pattern.

### Verification

```bash
cd skelegent/ && ./scripts/verify.sh
cd extras/ && nix develop -c cargo test --workspace --all-targets
cd extras/ && nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

Note: `verify.sh` runs `nix fmt` + `cargo test` + `cargo clippy`. All phases use it for skelegent; extras uses explicit commands (no verify.sh in that repo).

---

## Phase 2: Close the Secrets Gap

**Goal**: Add middleware wrapping around secret resolution. Lift secrets composability from 6/10 to 9/10.

### Research

**Tier A — Targeted:**
1. How Semantic Kernel integrates Azure Key Vault with function filters — pre/post interception for secret access
2. How Hashicorp Vault Agent sidecar intercepts secret access — proxy pattern, lease renewal, caching
3. How OpenBao handles secret access policies — ACL model, audit logging

**Tier B — Exploratory:**
4. Secret access middleware patterns in distributed systems — caching layers, lease pooling, automatic rotation
5. Should secret middleware live in layer0, skg-secret, or a hook crate? — blast radius analysis
6. Secret access patterns in Kubernetes operator frameworks — how Rust/Go operators handle secret injection with audit

**Tier C — Unknown unknowns:**
7. Production secret management incidents caused by missing audit or middleware — what breaks without it?
8. Novel secret lifecycle patterns in AI agent systems — per-turn scoping, secret redaction in context windows

### Design Options (research-dependent)

- **Option A**: Full `SecretMiddleware` trait + `SecretStack` + builder — 6th middleware boundary, same pattern as all others
- **Option B**: Extend `SecretRegistry` with pre/post hooks — lighter weight, no new layer0 trait
- **Option C**: `SecretMiddleware` as a hook crate wrapping `SecretRegistry` — no layer0 changes

### Current State

`skg-secret` already provides:
- `SecretResolver` trait (async, object-safe)
- `SecretRegistry` (routes by `SecretSource` variant)
- `SecretEventSink` for audit logging
- `SecretLease` with TTL, renewal, expiry
- `SecretValue` with zeroize-on-drop, scoped exposure, redacted Debug

The gap is: no middleware chain wrapping resolution. No way to add policy guards, caching, or per-operator access control without modifying the registry itself.

Research will determine which option best fits the continuation-passing philosophy.

### Item

#### #7 — SecretMiddleware
- **Repo**: skelegent (if layer0) or extras (if hook crate)
- **Effort**: L-XL (depends on design option chosen)
- **Decision deferred to research validation**
- **Tests**: Policy guard denies access, audit observer records all resolutions, caching middleware returns cached lease, middleware chain composes correctly
- **Risk**: If Phase 1 #2 reveals middleware composition issues, this phase's design must account for them
- **Note**: If SecretMiddleware becomes a new layer0 trait (Option A), update ARCHITECTURE.md to document the 6th middleware boundary

### Verification

```bash
cd skelegent/ && nix develop -c cargo test --workspace --all-targets
cd skelegent/ && nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

---

## Phase 3: Architecture Refinement

**Goal**: Close three architectural gaps — OTel event bridge, HITL context mutation, and Handoff lossy conversion.

### Research

**Tier A — Targeted:**
1. How OTel collector bridges async events to spans — span events vs child spans for progress updates
2. How LangGraph implements HITL context modification — `interrupt()` + state mutation pattern
3. How Temporal handles workflow input serialization — structured data vs byte serialization
4. How Letta's Context Repository handles mid-execution context injection

**Tier B — Exploratory:**
5. Patterns for bridging event streams to distributed tracing — Datadog, Honeycomb, Jaeger recommendations
6. HITL patterns that preserve replay determinism — does context mutation break record/replay? How do durable execution systems handle it?
7. Structured effect payloads in actor systems — Akka, Erlang, Orleans typed message passing

**Tier C — Unknown unknowns:**
8. Agent framework production incidents caused by lossy serialization or missing observability
9. Emerging AI agent observability primitives beyond traces — execution journals, decision trees, reasoning traces

### Items

#### #8 — OTel dispatch event → span bridge
- **Repo**: extras (skg-hook-otel)
- **Effort**: M
- **Design**: Add capability to convert `DispatchEvent::Progress` to OTel span events
- **Research determines**: interceptor on DispatchHandle vs observer middleware vs separate bridge type
- **Tests**: Dispatch with progress events, verify OTel spans contain progress as span events

#### #9 — HITL context mutation
- **Repo**: skelegent (location TBD — layer0 or skg-context-engine)
- **Effort**: XL (riskiest item — touches core protocol)
- **Design**: Channel/callback for external code to inject modified context during a middleware pause
- **Critical constraint**: Must be recordable — record/replay must capture mutations for deterministic replay. A replayed HITL mutation journal entry should contain: the boundary where the pause occurred, the original context, the mutated context, and a timestamp. Replay replays the mutation without waiting for human input.
- **Concrete mechanism (research validates)**: A `DispatchMiddleware` guard that sends context to an external channel (e.g., `tokio::sync::oneshot`), awaits a modified context back, and resumes the chain with the modified context. The recorder captures the mutation as a `RecordEntry` with `Boundary::Dispatch`, `Phase::Mutation`, and the delta.
- **Research determines**: Whether this belongs in layer0 (new Phase variant?) or a hook crate, whether `Phase::Mutation` is the right abstraction, whether the channel-based approach is correct vs. a callback
- **Tests**:
  - DispatchMiddleware guard pauses, external code sends modified context via channel, guard resumes with modified context, downstream middleware sees modified context
  - Record the mutation, replay, verify same downstream behavior
  - Timeout: if no human responds within deadline, guard returns error (not hang)

#### #10 — Handoff lossy JSON→text conversion
- **Repo**: skelegent (skg-effects-local)
- **File**: `effects/skg-effects-local/src/lib.rs` lines 163-170
- **Effort**: S
- **Current**: `Content::text(state.to_string())` serializes JSON Value to text, then `input.metadata = serde_json::Value::Null` overwrites metadata
- **Fix**: Set `input.metadata = state.clone()` instead of `serde_json::Value::Null`, preserving structured JSON. The text content can remain as a human-readable summary, but metadata carries the structured payload.
- **Tests**: Handoff with complex JSON state (nested objects, arrays), verify receiving operator gets structured data via `input.metadata`

### Parallelism

```
[#8 || #9 || #10]
```

All touch different repos/crates. But #9 is highest risk — research validation before implementation is critical.

### Verification

```bash
cd skelegent/ && ./scripts/verify.sh
cd extras/ && nix develop -c cargo test --workspace --all-targets
cd extras/ && nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

---

## Phase 4: Polish & Completeness

**Goal**: Fix every remaining rough edge. After this, every corner is S+ tier.

### Research

**Tier A — Targeted:**
1. OTel gen_ai semantic conventions spec — exact attributes for embed responses (`gen_ai.response.model`)
2. SSE specification (WHATWG EventSource) — reconnection semantics, retry field, Last-Event-ID
3. Float parsing in ML evaluation systems — how production frameworks parse model-generated scores

**Tier B — Exploratory:**
4. Rust crate API completeness patterns — what makes a crate feel complete? Missing derives, convenience methods, From/Into impls?
5. MatchStrategy patterns in replay systems — is content-hash matching useful? Implement or remove?
6. SSE client resilience patterns — compression, auth refresh, proxy buffering

**Tier C — Unknown unknowns:**
7. What beloved Rust crates (serde, tokio, axum) do that makes APIs feel magical
8. Common pitfalls in cosine similarity — empty vectors, NaN, numerical stability

### Items

| # | Item | Repo | File | Fix |
|---|---|---|---|---|
| 11 | OTel embed `gen_ai.response.model` | extras | skg-hook-otel/src/lib.rs | Add attribute after embed response |
| 12 | LlmJudge float parser | extras | skg-eval/src/lib.rs | Handle negatives, scientific notation, or use stdlib |
| 13 | ByContentHash stub | extras | skg-hook-replay/src/lib.rs | Implement or document as no-op (research decides). Note: removing the variant is semver-breaking; prefer implementing or leaving with clear docs. |
| 14 | A2A SSE reconnection | extras | skg-a2a/src/client/stream.rs | Implement retry/id or document limitation |
| 15 | Progress/Artifact executor test | skelegent | skg-effects-local tests | Add test that these skip cleanly |
| 16 | cosine_similarity mismatched/NaN inputs | extras | skg-eval/src/lib.rs | Guard against mismatched-length vectors and NaN propagation (empty vec already handled) |
| 17 | Progress round-trip test | extras | skg-a2a/tests/streaming.rs | Add integration test |
| 18 | OAuthDeviceFlowProvider Clone | extras | skg-auth-oauth/src/lib.rs | Add `#[derive(Clone)]` to `OAuthDeviceFlowProvider` (all fields are Clone-able) |
| 19 | EvalRunner panic case name | extras | skg-eval/src/lib.rs | Capture name before spawn |
| 20 | SubDispatchRecord naming | skelegent | layer0 | Rename or document (research decides) |

### Parallelism

All items touch different files. Maximum parallelism:

```
Skelegent: [#15 || #20]
Extras:    [#11 || #12 || #13 || #14 || #16 || #17 || #18 || #19]
```

### Verification

```bash
cd skelegent/ && ./scripts/verify.sh
cd extras/ && nix develop -c cargo test --workspace --all-targets
cd extras/ && nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

---

## Cross-Phase Constraints

1. **Each phase gates the next** — P1 must pass verification before P2 starts. P1 might reveal deeper issues that change P2-P4 scope.
2. **Research can change scope** — Items may be added, modified, or removed based on findings. If research reveals something is wrong, we fix it. If research reveals something is missing, we add it.
3. **No commit without verification** — Every phase ends with full workspace `cargo test` + `cargo clippy` clean.
4. **Research cost management** — Tier A/B use parallel-search (fast, cheap). Tier C uses parallel-deep-research only when needed.
5. **Research digests are preserved** — Each phase's research output is saved for future reference.

## Success Criteria

- All 1,159+ tests passing (new tests bring total higher)
- Zero clippy warnings
- Every public function in both repos has direct or integration test coverage
- Secrets composability score: 9/10+
- Every composability scenario from the audit: 8/10+
- No known untested critical paths
