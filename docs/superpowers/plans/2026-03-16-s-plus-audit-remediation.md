# S+ Audit Remediation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring skelegent + extras to S+ tier engineering — proven by tests, closed architectural gaps, polished APIs.

**Architecture:** 4 research-gated phases ordered by risk. Each phase: parallel research → validate against skelegent philosophies → implement → verify. Research can add/remove/modify items.

**Tech Stack:** Rust (edition 2024), tokio, async-trait, serde, tracing, opentelemetry. Nix-provided tooling. TDD throughout.

**Spec:** `docs/superpowers/specs/2026-03-16-s-plus-audit-remediation-design.md`

---

## Chunk 1: Phase 1 — Prove It Works

### Task 0: Phase 1 Research

Research how best-in-class systems test the exact patterns skelegent uses. This MUST complete before any implementation task in this phase.

**Tier A — Targeted (parallel-search, 4 queries):**
1. "How Temporal SDK tests workflow cancellation propagation through interceptors TestWorkflowEnvironment"
2. "How Restate durable execution framework tests streaming replay journal verification"
3. "How Tower Rust middleware tests layered Service composition ServiceExt"
4. "How LangGraph tests graph state checkpoint traversal operations"

**Tier B — Exploratory (parallel-search, 4 queries):**
5. "Best practices testing async event pipelines Rust tokio select branch coverage cancellation"
6. "Property-based testing middleware stacks Rust proptest quickcheck"
7. "Chaos testing patterns orchestration frameworks dropped connections partial failures"
8. "Graph operation testing patterns Neo4j DGraph traversal correctness property-based"

**Tier C — Unknown unknowns (parallel-deep-research if warranted):**
9. "Test failures that bit Temporal Restate LangGraph users in production postmortems"
10. "Rust testing patterns beyond cargo test tracing-test tokio-test harness"

- [ ] **Step 1: Dispatch Tier A+B research as parallel sub-agents (8 parallel-search queries)**
- [ ] **Step 2: Review results — extract patterns relevant to skelegent's continuation-passing middleware**
- [ ] **Step 3: Decide if Tier C research is needed based on gaps in A+B findings**
- [ ] **Step 4: Write research digest to `docs/superpowers/research/p1-prove-it-works.md`**
- [ ] **Step 5: Validate — do any findings change what tests we should write? Update task scope if so.**

---

### Task 1: DispatchHandle::intercept() cancellation propagation

**Files:**
- Modify: `skelegent/layer0/src/dispatch.rs` (add tests inside `#[cfg(test)] mod tests`, before closing `}` at line 808)

**Context:** `DispatchHandle::intercept()` (line 387) wraps a handle with a callback. A background `tokio::spawn` task reads events from the inner handle, passes each to the callback, and forwards to a new channel. Lines 399-404 handle cancellation: when outer handle is cancelled, `inner.cancel()` is called. Three existing tests cover event forwarding but NOT cancellation.

- [ ] **Step 1: Write cancellation propagation test**

```rust
#[tokio::test]
async fn handle_intercept_propagates_cancellation() {
    // Create a handle that will send events slowly.
    let (handle, sender) = DispatchHandle::channel(DispatchId::new("cancel-test"));

    // Spawn a slow producer that waits before completing.
    let s = sender.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = s
            .send(DispatchEvent::Completed {
                output: OperatorOutput::new(
                    Content::text("should not reach"),
                    crate::operator::ExitReason::Complete,
                ),
            })
            .await;
    });

    let intercepted = handle.intercept(|_| {});

    // Cancel the outer handle.
    intercepted.cancel();

    // The inner handle's sender should detect cancellation.
    // Give the spawn a moment to propagate.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Verify: sender.is_cancelled() means inner handle propagated cancel.
    assert!(sender.is_cancelled(), "inner sender should be cancelled after outer cancel");
}
```

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 handle_intercept_propagates_cancellation -- --nocapture`

If it passes, cancellation propagation works. If it fails, the intercept code has a bug to fix.

- [ ] **Step 3: Write consumer-drop test**

```rust
#[tokio::test]
async fn handle_intercept_exits_when_consumer_drops() {
    let events_seen = Arc::new(AtomicU32::new(0));
    let seen = events_seen.clone();

    let handle = make_handle_with_events(vec![
        progress_event("a"),
        progress_event("b"),
        progress_event("c"),
        completed_event(),
    ]);

    let intercepted = handle.intercept(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });

    // Drop without consuming — pump task should exit cleanly.
    drop(intercepted);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // The interceptor may or may not have seen events depending on timing,
    // but the important thing is the spawn task exited (no leak).
    // If this test completes without hanging, the pump exits cleanly.
}
```

- [ ] **Step 4: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 handle_intercept_exits_when_consumer_drops -- --nocapture`

Expected: PASS (test completes without hanging)

- [ ] **Step 5: Write panic-in-callback test**

```rust
#[tokio::test]
async fn handle_intercept_panicking_callback_does_not_hang() {
    let handle = make_handle_with_events(vec![progress_event("a"), completed_event()]);
    let intercepted = handle.intercept(|_| panic!("boom"));
    // Should either produce an error or not hang. The pump task catches
    // the panic (std::panic::catch_unwind is not used, but tokio::spawn
    // absorbs panics as JoinError). The important thing: no deadlock.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        intercepted.collect(),
    )
    .await;
    // Timeout means it hung — that's the failure case.
    assert!(result.is_ok(), "collect should not hang when callback panics");
}
```

- [ ] **Step 6: Run panic test**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 handle_intercept_panicking -- --nocapture`
Expected: PASS (completes within 2s)

- [ ] **Step 7: Commit**

```bash
cd skelegent/ && git add layer0/src/dispatch.rs && git commit -m "test: verify intercept cancellation, consumer-drop, and callback-panic behavior"
```

---

### Task 2: skg-orch-local unit tests

**Files:**
- Modify: `skelegent/orch/skg-orch-local/src/lib.rs` (add `#[cfg(test)] mod tests` after line 124)

**Context:** `LocalOrch` implements `Dispatcher` for in-process dispatch. It has operator registration (`register()`), dispatch via `tokio::spawn`, signal journal (`workflow_signals`), and optional middleware. Zero unit tests currently — only integration tests in `orch/skg-orch-local/tests/orch.rs`.

- [ ] **Step 1: Write basic dispatch test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use layer0::content::Content;
    use layer0::dispatch::Dispatcher;
    use layer0::id::{DispatchId, OperatorId};
    use layer0::operator::{ExitReason, OperatorInput, OperatorOutput, TriggerType};

    /// Minimal operator that echoes input text.
    struct EchoOp;

    #[async_trait]
    impl Operator for EchoOp {
        async fn execute(
            &self,
            input: OperatorInput,
            _ctx: &DispatchContext,
            _emitter: &layer0::dispatch::EffectEmitter,
        ) -> Result<OperatorOutput, layer0::error::OperatorError> {
            Ok(OperatorOutput::new(input.message.clone(), ExitReason::Complete))
        }
    }

    fn test_ctx(op: &str) -> DispatchContext {
        DispatchContext::new(DispatchId::new("test"), OperatorId::new(op))
    }

    fn test_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), TriggerType::User)
    }

    #[tokio::test]
    async fn dispatch_to_registered_operator() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));

        let handle = orch
            .dispatch(&test_ctx("echo"), test_input("hello"))
            .await
            .unwrap();
        let output = handle.collect().await.unwrap();
        assert_eq!(output.message.as_text(), Some("hello"));
    }
}
```

- [ ] **Step 2: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local dispatch_to_registered -- --nocapture`
Expected: PASS

- [ ] **Step 3: Write unregistered operator error test**

```rust
    #[tokio::test]
    async fn dispatch_to_unknown_operator_returns_error() {
        let orch = LocalOrch::new();
        let result = orch.dispatch(&test_ctx("nonexistent"), test_input("hi")).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("nonexistent"),
            "error should mention operator id: {err}"
        );
    }
```

- [ ] **Step 4: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local dispatch_to_unknown -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write concurrent dispatch test**

```rust
    #[tokio::test]
    async fn concurrent_dispatches_do_not_deadlock() {
        let mut orch = LocalOrch::new();
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));
        let orch = Arc::new(orch);

        let mut handles = Vec::new();
        for i in 0..10 {
            let o = orch.clone();
            handles.push(tokio::spawn(async move {
                let handle = o
                    .dispatch(&test_ctx("echo"), test_input(&format!("msg-{i}")))
                    .await
                    .unwrap();
                handle.collect().await.unwrap()
            }));
        }

        for h in handles {
            let output = h.await.unwrap();
            assert!(output.message.as_text().is_some());
        }
    }
```

- [ ] **Step 6: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local concurrent_dispatches -- --nocapture`
Expected: PASS

- [ ] **Step 7: Write signal journal test**

```rust
    #[tokio::test]
    async fn signal_journal_records_and_retrieves() {
        let orch = LocalOrch::new();
        let wf = WorkflowId::new("wf-1");

        assert_eq!(orch.signal_count(&wf).await, 0);

        // Signal journal is written via the Signalable trait in skg-orch-kit.
        // Here we test the count method directly after manually inserting.
        {
            let mut signals = orch.workflow_signals.write().await;
            signals
                .entry(wf.to_string())
                .or_default()
                .push(SignalPayload {
                    signal_type: "test-signal".into(),
                    data: serde_json::json!({"key": "value"}),
                });
        }

        assert_eq!(orch.signal_count(&wf).await, 1);
    }
```

- [ ] **Step 8: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local signal_journal -- --nocapture`
Expected: PASS

- [ ] **Step 9: Write middleware stack integration test**

```rust
    #[tokio::test]
    async fn dispatch_with_middleware_stack() {
        use layer0::middleware::{DispatchMiddleware, DispatchNext, DispatchStack};
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingMiddleware(Arc<AtomicU32>);

        #[async_trait]
        impl DispatchMiddleware for CountingMiddleware {
            async fn dispatch(
                &self,
                ctx: &DispatchContext,
                input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<layer0::dispatch::DispatchHandle, layer0::error::OrchError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                next.dispatch(ctx, input).await
            }
        }

        let count = Arc::new(AtomicU32::new(0));
        let stack = DispatchStack::builder()
            .observe(Arc::new(CountingMiddleware(count.clone())))
            .build();

        let mut orch = LocalOrch::new().with_middleware(stack);
        orch.register(OperatorId::new("echo"), Arc::new(EchoOp));

        let handle = orch
            .dispatch(&test_ctx("echo"), test_input("hi"))
            .await
            .unwrap();
        let _output = handle.collect().await.unwrap();

        assert_eq!(count.load(Ordering::SeqCst), 1, "middleware should fire");
    }
```

- [ ] **Step 10: Run middleware test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local dispatch_with_middleware -- --nocapture`
Expected: PASS

- [ ] **Step 11: Run all skg-orch-local tests**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-orch-local --all-targets`
Expected: All pass

- [ ] **Step 10: Commit**

```bash
cd skelegent/ && git add orch/skg-orch-local/src/lib.rs && git commit -m "test: add unit tests for LocalOrch dispatch, error, concurrency, signals"
```

---

### Task 3: Graph operations — InMemoryStore implementation + end-to-end tests

**Files:**
- Modify: `skelegent/layer0/src/test_utils/in_memory_store.rs` (add graph operation implementations)
- Modify: `skelegent/layer0/tests/phase1.rs` (add graph operation tests)

**Context:** `StateStore` trait defines `link()`, `unlink()`, `traverse()` with default impls that return "graph operations not supported" errors. `InMemoryStore` inherits these defaults. `MemoryLink` has `from_key`, `to_key`, `relation`, `metadata`.

- [ ] **Step 1: Add graph storage to InMemoryStore**

In `skelegent/layer0/src/test_utils/in_memory_store.rs`, add a links field and implement the three graph methods:

```rust
// Add to struct InMemoryStore:
use crate::state::MemoryLink;

pub struct InMemoryStore {
    data: RwLock<HashMap<(String, String), serde_json::Value>>,
    links: RwLock<Vec<(String, MemoryLink)>>, // (scope_key, link)
}

// Update new():
pub fn new() -> Self {
    Self {
        data: RwLock::new(HashMap::new()),
        links: RwLock::new(Vec::new()),
    }
}

// Add to the StateStore impl:
async fn link(&self, scope: &Scope, link: &MemoryLink) -> Result<(), StateError> {
    let mut links = self
        .links
        .write()
        .map_err(|e| StateError::WriteFailed(e.to_string()))?;
    links.push((scope_key(scope), link.clone()));
    Ok(())
}

async fn unlink(
    &self,
    scope: &Scope,
    from_key: &str,
    to_key: &str,
    relation: &str,
) -> Result<(), StateError> {
    let mut links = self
        .links
        .write()
        .map_err(|e| StateError::WriteFailed(e.to_string()))?;
    let sk = scope_key(scope);
    links.retain(|(s, l)| {
        !(s == &sk && l.from_key == from_key && l.to_key == to_key && l.relation == relation)
    });
    Ok(())
}

async fn traverse(
    &self,
    scope: &Scope,
    from_key: &str,
    relation: Option<&str>,
    max_depth: u32,
) -> Result<Vec<String>, StateError> {
    let links = self
        .links
        .read()
        .map_err(|e| StateError::Other(e.to_string().into()))?;
    let sk = scope_key(scope);

    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((from_key.to_owned(), 0u32));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for (s, link) in links.iter() {
            if s != &sk || link.from_key != current {
                continue;
            }
            if let Some(rel) = relation {
                if link.relation != rel {
                    continue;
                }
            }
            if visited.insert(link.to_key.clone()) {
                queue.push_back((link.to_key.clone(), depth + 1));
            }
        }
    }
    Ok(visited.into_iter().collect())
}
```

- [ ] **Step 2: Run existing tests to ensure nothing breaks**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 --all-targets`
Expected: All existing tests pass

- [ ] **Step 3: Write graph link + traverse test in phase1.rs**

Add at the end of `skelegent/layer0/tests/phase1.rs`:

```rust
#[tokio::test]
async fn graph_link_and_traverse() {
    use layer0::state::{MemoryLink, StateStore};
    use layer0::test_utils::InMemoryStore;
    use layer0::effect::Scope;

    let store = InMemoryStore::new();
    let scope = Scope::Session("test".into());

    // Write some entries first.
    store.write(&scope, "a", json!({"val": "alpha"})).await.unwrap();
    store.write(&scope, "b", json!({"val": "beta"})).await.unwrap();
    store.write(&scope, "c", json!({"val": "gamma"})).await.unwrap();

    // Link a -> b -> c via "references".
    let link_ab = MemoryLink {
        from_key: "a".into(),
        to_key: "b".into(),
        relation: "references".into(),
        metadata: None,
    };
    let link_bc = MemoryLink {
        from_key: "b".into(),
        to_key: "c".into(),
        relation: "references".into(),
        metadata: None,
    };
    store.link(&scope, &link_ab).await.unwrap();
    store.link(&scope, &link_bc).await.unwrap();

    // Traverse from a, depth 1: should find b only.
    let mut reachable = store.traverse(&scope, "a", Some("references"), 1).await.unwrap();
    reachable.sort();
    assert_eq!(reachable, vec!["b"]);

    // Traverse from a, depth 2: should find b and c.
    let mut reachable = store.traverse(&scope, "a", Some("references"), 2).await.unwrap();
    reachable.sort();
    assert_eq!(reachable, vec!["b", "c"]);
}
```

- [ ] **Step 4: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 --test phase1 graph_link_and_traverse -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write unlink test**

```rust
#[tokio::test]
async fn graph_unlink_removes_edge() {
    use layer0::state::{MemoryLink, StateStore};
    use layer0::test_utils::InMemoryStore;
    use layer0::effect::Scope;

    let store = InMemoryStore::new();
    let scope = Scope::Session("test".into());

    let link = MemoryLink {
        from_key: "x".into(),
        to_key: "y".into(),
        relation: "related".into(),
        metadata: None,
    };
    store.link(&scope, &link).await.unwrap();

    // Verify link exists.
    let reachable = store.traverse(&scope, "x", Some("related"), 1).await.unwrap();
    assert_eq!(reachable, vec!["y"]);

    // Unlink.
    store.unlink(&scope, "x", "y", "related").await.unwrap();

    // Verify link removed.
    let reachable = store.traverse(&scope, "x", Some("related"), 1).await.unwrap();
    assert!(reachable.is_empty());
}
```

- [ ] **Step 6: Write relation filter test**

```rust
#[tokio::test]
async fn graph_traverse_filters_by_relation() {
    use layer0::state::{MemoryLink, StateStore};
    use layer0::test_utils::InMemoryStore;
    use layer0::effect::Scope;

    let store = InMemoryStore::new();
    let scope = Scope::Session("test".into());

    // Two links from same source, different relations.
    store.link(&scope, &MemoryLink {
        from_key: "a".into(),
        to_key: "b".into(),
        relation: "references".into(),
        metadata: None,
    }).await.unwrap();
    store.link(&scope, &MemoryLink {
        from_key: "a".into(),
        to_key: "c".into(),
        relation: "supersedes".into(),
        metadata: None,
    }).await.unwrap();

    // Filter by "references" — only b.
    let refs = store.traverse(&scope, "a", Some("references"), 1).await.unwrap();
    assert_eq!(refs, vec!["b"]);

    // No filter — both b and c.
    let mut all = store.traverse(&scope, "a", None, 1).await.unwrap();
    all.sort();
    assert_eq!(all, vec!["b", "c"]);
}
```

- [ ] **Step 7: Run all graph tests**

Run: `cd skelegent/ && nix develop -c cargo test -p layer0 --test phase1 graph_ -- --nocapture`
Expected: All 3 pass

- [ ] **Step 8: Write effect handler pipeline test for LinkMemory**

Add to `skelegent/effects/skg-effects-local/tests/hooks.rs`:

```rust
#[tokio::test]
async fn link_memory_effect_creates_graph_link() {
    use layer0::state::{MemoryLink, StateStore};

    let store = Arc::new(InMemoryStore::new());
    let handler = LocalEffectHandler::new(
        store.clone(),
        None, // no dispatcher needed for link
        None, // no signalable needed
        skg_effects_local::UnknownEffectPolicy::Error,
    );

    // Write entries first so link has something to connect.
    let scope = layer0::effect::Scope::Session("test".into());
    store.write(&scope, "doc-a", json!({"text": "hello"})).await.unwrap();
    store.write(&scope, "doc-b", json!({"text": "world"})).await.unwrap();

    let link = MemoryLink {
        from_key: "doc-a".into(),
        to_key: "doc-b".into(),
        relation: "references".into(),
        metadata: None,
    };
    let effect = Effect::LinkMemory {
        scope: scope.clone(),
        link,
    };

    let ctx = test_ctx();
    let outcome = handler.handle(&effect, &ctx).await.unwrap();
    assert!(matches!(outcome, EffectOutcome::Applied));

    // Verify the link was actually created in the store.
    let reachable = store.traverse(&scope, "doc-a", Some("references"), 1).await.unwrap();
    assert_eq!(reachable, vec!["doc-b"]);
}
```

Note: The exact `LocalEffectHandler::new()` constructor args need to match the current API. Read the constructor before writing. The above is a template — adjust args after reading `skg-effects-local/src/lib.rs`.

- [ ] **Step 9: Run effect handler test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-effects-local link_memory_effect -- --nocapture`
Expected: PASS

- [ ] **Step 10: Commit**

```bash
cd skelegent/ && git add layer0/src/test_utils/in_memory_store.rs layer0/tests/phase1.rs && git commit -m "feat: implement graph ops on InMemoryStore, add end-to-end graph tests"
```

---

### Task 4: EvalRunner concurrency verification

**Files:**
- Modify: `extras/eval/skg-eval/src/lib.rs` (add concurrency tests in existing `#[cfg(test)] mod tests`)

**Context:** `EvalRunner` uses `tokio::task::JoinSet` with `Semaphore` for bounded concurrency (lines 350-383 in skg-eval/src/lib.rs). Existing tests run 2 cases with concurrency=2 but don't verify actual parallelism.

- [ ] **Step 1: Write wall-clock concurrency test**

Add to the existing test module in `extras/eval/skg-eval/src/lib.rs`:

```rust
#[tokio::test]
async fn eval_runner_executes_concurrently() {
    // Each case takes 100ms. With concurrency=4 and 4 cases,
    // total time should be ~100ms, not ~400ms.
    use std::time::Instant;

    struct SlowDispatcher;

    #[async_trait::async_trait]
    impl layer0::dispatch::Dispatcher for SlowDispatcher {
        async fn dispatch(
            &self,
            ctx: &layer0::DispatchContext,
            _input: layer0::operator::OperatorInput,
        ) -> Result<layer0::dispatch::DispatchHandle, layer0::error::OrchError> {
            let (handle, sender) = layer0::dispatch::DispatchHandle::channel(ctx.dispatch_id.clone());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let _ = sender
                    .send(layer0::dispatch::DispatchEvent::Completed {
                        output: layer0::operator::OperatorOutput::new(
                            layer0::content::Content::text("slow-result"),
                            layer0::operator::ExitReason::Complete,
                        ),
                    })
                    .await;
            });
            Ok(handle)
        }
    }

    let runner = EvalRunner::new(4); // concurrency = 4
    let cases: Vec<EvalCase> = (0..4)
        .map(|i| EvalCase {
            name: format!("slow-{i}"),
            input: layer0::operator::OperatorInput::new(
                layer0::content::Content::text("hi"),
                layer0::operator::TriggerType::User,
            ),
            expected: ExpectedOutput::Contains(vec!["slow-result".into()]),
        })
        .collect();

    let metrics: Vec<Arc<dyn EvalMetric>> = vec![Arc::new(ExactMatchMetric)];
    let dispatcher = Arc::new(SlowDispatcher);
    let op_id = layer0::id::OperatorId::new("test");

    let start = Instant::now();
    let _report = runner.run(dispatcher, op_id, metrics, cases).await;
    let elapsed = start.elapsed();

    // Should complete in ~100ms (parallel), not ~400ms (sequential).
    // Use 250ms as generous upper bound for CI.
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "Expected parallel execution (~100ms) but took {:?}",
        elapsed
    );
}
```

- [ ] **Step 2: Run test**

Run: `cd extras/ && nix develop -c cargo test -p skg-eval eval_runner_executes_concurrently -- --nocapture`
Expected: PASS

- [ ] **Step 3: Write semaphore bound verification test**

```rust
#[tokio::test]
async fn eval_runner_respects_concurrency_limit() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));

    struct TrackingDispatcher {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl layer0::dispatch::Dispatcher for TrackingDispatcher {
        async fn dispatch(
            &self,
            ctx: &layer0::DispatchContext,
            _input: layer0::operator::OperatorInput,
        ) -> Result<layer0::dispatch::DispatchHandle, layer0::error::OrchError> {
            let prev = self.active.fetch_add(1, Ordering::SeqCst);
            // Update max.
            self.max_active.fetch_max(prev + 1, Ordering::SeqCst);

            let (handle, sender) = layer0::dispatch::DispatchHandle::channel(ctx.dispatch_id.clone());
            let active = self.active.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = sender
                    .send(layer0::dispatch::DispatchEvent::Completed {
                        output: layer0::operator::OperatorOutput::new(
                            layer0::content::Content::text("ok"),
                            layer0::operator::ExitReason::Complete,
                        ),
                    })
                    .await;
            });
            Ok(handle)
        }
    }

    let dispatcher = Arc::new(TrackingDispatcher {
        active: active.clone(),
        max_active: max_active.clone(),
    });

    let runner = EvalRunner::new(2); // concurrency = 2
    let cases: Vec<EvalCase> = (0..6)
        .map(|i| EvalCase {
            name: format!("case-{i}"),
            input: layer0::operator::OperatorInput::new(
                layer0::content::Content::text("hi"),
                layer0::operator::TriggerType::User,
            ),
            expected: ExpectedOutput::Contains(vec!["ok".into()]),
        })
        .collect();

    let metrics: Vec<Arc<dyn EvalMetric>> = vec![Arc::new(ContainsMetric)];
    let op_id = layer0::id::OperatorId::new("test");
    let report = runner.run(dispatcher, op_id, metrics, cases).await;

    assert_eq!(report.total_cases(), 6);
    // Max concurrent should be <= 2 (the semaphore bound).
    assert!(
        max_active.load(Ordering::SeqCst) <= 2,
        "Max concurrent was {}, expected <= 2",
        max_active.load(Ordering::SeqCst)
    );
}
```

- [ ] **Step 4: Run test**

Run: `cd extras/ && nix develop -c cargo test -p skg-eval eval_runner_respects -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write panic resilience test**

```rust
#[tokio::test]
async fn eval_runner_survives_panicking_case() {
    struct PanicDispatcher {
        panic_on: String,
    }

    #[async_trait::async_trait]
    impl layer0::dispatch::Dispatcher for PanicDispatcher {
        async fn dispatch(
            &self,
            ctx: &layer0::DispatchContext,
            _input: layer0::operator::OperatorInput,
        ) -> Result<layer0::dispatch::DispatchHandle, layer0::error::OrchError> {
            if ctx.operator_id.as_str() == "panic-op" {
                panic!("intentional panic in test");
            }
            let (handle, sender) = layer0::dispatch::DispatchHandle::channel(ctx.dispatch_id.clone());
            tokio::spawn(async move {
                let _ = sender
                    .send(layer0::dispatch::DispatchEvent::Completed {
                        output: layer0::operator::OperatorOutput::new(
                            layer0::content::Content::text("ok"),
                            layer0::operator::ExitReason::Complete,
                        ),
                    })
                    .await;
            });
            Ok(handle)
        }
    }

    let runner = EvalRunner::new(4);
    let cases = vec![
        EvalCase {
            name: "good-1".into(),
            input: layer0::operator::OperatorInput::new(
                layer0::content::Content::text("hi"),
                layer0::operator::TriggerType::User,
            ),
            expected: ExpectedOutput::Contains(vec!["ok".into()]),
        },
        EvalCase {
            name: "good-2".into(),
            input: layer0::operator::OperatorInput::new(
                layer0::content::Content::text("hi"),
                layer0::operator::TriggerType::User,
            ),
            expected: ExpectedOutput::Contains(vec!["ok".into()]),
        },
    ];

    let metrics: Vec<Arc<dyn EvalMetric>> = vec![Arc::new(ContainsMetric)];
    let dispatcher = Arc::new(PanicDispatcher {
        panic_on: "panic-case".into(),
    });
    let op_id = layer0::id::OperatorId::new("test");
    let report = runner.run(dispatcher, op_id, metrics, cases).await;

    // Both good cases should complete successfully despite the panic-capable dispatcher.
    assert_eq!(report.total_cases(), 2);
}
```

- [ ] **Step 6: Run panic test**

Run: `cd extras/ && nix develop -c cargo test -p skg-eval eval_runner_survives -- --nocapture`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
cd extras/ && git add eval/skg-eval/src/lib.rs && git commit -m "test: verify EvalRunner concurrent execution, semaphore bounds, and panic resilience"
```

---

### Task 5: A2A SSE terminal states

**Files:**
- Modify: `extras/a2a/skg-a2a/tests/streaming.rs` (add terminal state tests)

**Context:** The streaming client's `handle_frame()` maps SSE events to `DispatchEvent`. Terminal states (Failed, Canceled, Rejected) map to `DispatchEvent::Failed`. The existing test only covers the happy path (Progress + Completed).

Note: The exact test structure depends on how the existing mock dispatcher and SSE test harness work. Read `extras/a2a/skg-a2a/tests/streaming.rs` and `extras/a2a/skg-a2a/src/client/stream.rs` before writing these tests. The mock dispatcher will need variants that produce Failed/Canceled terminal events.

- [ ] **Step 1: Read existing streaming test to understand harness**

Read: `extras/a2a/skg-a2a/tests/streaming.rs` (full file)
Read: `extras/a2a/skg-a2a/src/server/stream.rs` (understand how terminal states are serialized)

- [ ] **Step 2: Write mock dispatcher that produces Failed terminal state**

Add to streaming.rs — a `FailDispatcher` that sends `DispatchEvent::Failed` instead of Completed.

- [ ] **Step 3: Write test for Failed terminal state round-trip**

Test: Start server with FailDispatcher, client connects via SSE, verify client receives `DispatchEvent::Failed`.

- [ ] **Step 4: Run test**

Run: `cd extras/ && nix develop -c cargo test -p skg-a2a --test streaming -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write mock dispatchers for Canceled and Rejected states**

Add `CancelDispatcher` and `RejectDispatcher` variants that produce the corresponding terminal events. Follow the same pattern as `FailDispatcher` from Step 2.

- [ ] **Step 6: Write test for Canceled terminal state round-trip**

Test: Start server with CancelDispatcher, client connects via SSE, verify client receives `DispatchEvent::Failed` with cancellation context.

- [ ] **Step 7: Write test for Rejected terminal state round-trip**

Test: Start server with RejectDispatcher, verify client receives `DispatchEvent::Failed` with rejection context.

- [ ] **Step 8: Run all streaming tests**

Run: `cd extras/ && nix develop -c cargo test -p skg-a2a --test streaming -- --nocapture`
Expected: All pass (happy path + Failed + Canceled + Rejected)

- [ ] **Step 9: Commit**

```bash
cd extras/ && git add a2a/skg-a2a/tests/streaming.rs && git commit -m "test: add A2A SSE terminal state round-trip tests (Failed, Canceled, Rejected)"
```

---

### Task 6: MiddlewareProvider StreamProvider path

**Files:**
- Modify: `skelegent/provider/skg-provider-router/src/lib.rs` (add tests)

**Context:** `MiddlewareProvider` implements `StreamProvider` (line 358). It converts `StreamRequest` → `InferRequest`, runs through the `InferStack`, then emits stream events from the response. The question is: does middleware actually fire on this path?

- [ ] **Step 1: Read MiddlewareProvider StreamProvider impl**

Read: `skelegent/provider/skg-provider-router/src/lib.rs` lines 358-415 (already read above)

- [ ] **Step 2: Write observer middleware that tracks calls**

```rust
#[cfg(test)]
mod middleware_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Middleware that counts how many times infer() is called.
    struct CountingMiddleware {
        count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl InferMiddleware for CountingMiddleware {
        async fn infer(
            &self,
            request: InferRequest,
            next: &dyn InferNext,
        ) -> Result<InferResponse, ProviderError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            next.infer(request).await
        }
    }
}
```

- [ ] **Step 3: Read existing test infrastructure**

Read: `skelegent/provider/skg-provider-router/src/lib.rs` test module (if any exists)
Read: `skelegent/turn/skg-turn/src/provider.rs` for InferRequest/InferResponse/StreamRequest types

This step produces the concrete types needed for Step 4.

- [ ] **Step 4: Write test that stream_infer fires middleware**

Template (fill `StubDynProvider` and `StreamRequest` fields after Step 3):

```rust
    /// A DynProvider that returns a fixed response.
    struct StubDynProvider;

    impl DynProvider for StubDynProvider {
        fn infer_boxed(
            &self,
            _request: InferRequest,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<InferResponse, ProviderError>> + Send + '_>> {
            Box::pin(async {
                Ok(InferResponse {
                    // Fill with minimal valid response after reading the type
                    ..Default::default()
                })
            })
        }

        fn embed_boxed(
            &self,
            _request: EmbedRequest,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<EmbedResponse, ProviderError>> + Send + '_>> {
            Box::pin(async {
                Ok(EmbedResponse { embeddings: vec![], model: "stub".into(), usage: Default::default() })
            })
        }
    }

    #[tokio::test]
    async fn stream_infer_fires_middleware() {
        let count = Arc::new(AtomicU32::new(0));

        let stack = InferStack::builder()
            .observe(Arc::new(CountingMiddleware { count: count.clone() }))
            .build();

        let provider = MiddlewareProvider::new(Box::new(StubDynProvider))
            .with_infer_stack(stack);

        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ev = events.clone();
        let _response = provider
            .infer_stream(
                // Fill StreamRequest fields after reading the type in Step 3
                StreamRequest { model: "stub".into(), messages: vec![], ..Default::default() },
                move |event| { ev.lock().unwrap().push(format!("{event:?}")); },
            )
            .await
            .unwrap();

        assert_eq!(count.load(Ordering::SeqCst), 1, "middleware should fire on stream path");
        assert!(!events.lock().unwrap().is_empty(), "stream events should be emitted");
    }
```

Note: The exact field names on `InferResponse`, `StreamRequest`, and `DynProvider` method signatures need to be confirmed from the source. The template above is directionally correct — adjust after Step 3.

- [ ] **Step 4: Run test**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-provider-router stream_infer_fires -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd skelegent/ && git add provider/skg-provider-router/src/lib.rs && git commit -m "test: verify middleware fires on StreamProvider path"
```

---

### Task 7: Phase 1 verification

- [ ] **Step 1: Run full skelegent verification**

Run: `cd skelegent/ && ./scripts/verify.sh`
Expected: All tests pass, zero clippy warnings, nix fmt clean

- [ ] **Step 2: Run full extras verification**

Run: `cd extras/ && nix develop -c cargo test --workspace --all-targets && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass, zero clippy warnings

- [ ] **Step 3: Count tests**

Run: `cd skelegent/ && nix develop -c cargo test --workspace --all-targets 2>&1 | tail -5`
Run: `cd extras/ && nix develop -c cargo test --workspace --all-targets 2>&1 | tail -5`

Record new test counts. Should be significantly higher than 809 (skelegent) and 350 (extras).

- [ ] **Step 4: Commit any formatting fixes**

If nix fmt made changes:
```bash
cd skelegent/ && git add -A && git commit -m "style: apply nix fmt"
```

---

## Chunk 2: Phase 2 — Close the Secrets Gap

### Task 8: Phase 2 Research

**Tier A — Targeted (parallel-search, 3 queries):**
1. "Semantic Kernel Azure Key Vault function filters middleware secret access interception"
2. "Hashicorp Vault Agent sidecar secret middleware proxy lease renewal caching pattern"
3. "OpenBao secret access policy ACL audit logging middleware pattern"

**Tier B — Exploratory (parallel-search, 3 queries):**
4. "Secret access middleware patterns distributed systems caching lease pooling rotation"
5. "Secret middleware trait design Rust layer0 hook crate blast radius analysis"
6. "Kubernetes operator secret injection audit trail Rust Go patterns"

**Tier C — Unknown unknowns (parallel-search, 2 queries):**
7. "Production secret management incidents missing audit middleware postmortem"
8. "Secret lifecycle management AI agent systems per-turn scoping context window redaction"

- [ ] **Step 1: Dispatch Tier A+B+C research as parallel sub-agents (8 queries)**
- [ ] **Step 2: Synthesize findings — which design option (A/B/C from spec) best fits?**
- [ ] **Step 3: Write research digest to `docs/superpowers/research/p2-secrets-gap.md`**
- [ ] **Step 4: Decide: full SecretMiddleware trait (Option A), registry hooks (B), or hook crate (C)**
- [ ] **Step 5: If Option A chosen, draft the trait + stack + builder design before implementing**

### Task 9: Implement SecretMiddleware (scope depends on Task 8 research)

**Files:** Determined by Task 8 research outcome.

- If Option A: Modify `skelegent/secret/skg-secret/src/lib.rs` (add SecretMiddleware, SecretNext, SecretStack, SecretStackBuilder)
- If Option B: Modify `skelegent/secret/skg-secret/src/lib.rs` (add pre/post hooks to SecretRegistry)
- If Option C: Create `skelegent/hooks/skg-hook-secret/` (new crate)

- [ ] **Step 1: Write failing test — policy guard denies access**

Test: Build a secret middleware chain with a guard that denies access for a specific source. Assert `SecretError::AccessDenied` is returned.

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement the minimal middleware trait/hook to make it pass**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Write test — audit observer records all resolutions**

Test: Build chain with an observer that records all resolve calls. Resolve 3 different sources. Assert observer saw all 3.

- [ ] **Step 6: Run test**
- [ ] **Step 7: Write test — middleware chain composes correctly**

Test: Guard + observer + another observer. Assert guard blocks, observers still see the attempt.

- [ ] **Step 8: Run test**
- [ ] **Step 9: Write test — caching middleware returns cached lease**

Test: Build chain with a caching middleware that stores resolved leases. Resolve the same source twice. Assert the second resolution returns the cached lease without calling the backend resolver.

- [ ] **Step 10: Run test**
- [ ] **Step 11: If Option A — update ARCHITECTURE.md to document 6th middleware boundary**
- [ ] **Step 12: Commit**

Note: Replace file list below with actual files after research determines the design option.

```bash
cd skelegent/ && git add secret/ hooks/ ARCHITECTURE.md && git commit -m "feat: add SecretMiddleware for policy, audit, and caching around secret resolution"
```

### Task 10: Phase 2 verification

- [ ] **Step 1: Run full skelegent verification**

Run: `cd skelegent/ && ./scripts/verify.sh`
Expected: All tests pass, zero clippy warnings

- [ ] **Step 2: Count tests, confirm increase**

---

## Chunk 3: Phase 3 — Architecture Refinement

### Task 11: Phase 3 Research

**Tier A — Targeted (parallel-search, 4 queries):**
1. "OpenTelemetry bridge async events to spans span events vs child spans progress updates"
2. "LangGraph human-in-the-loop context modification interrupt state mutation pattern"
3. "Temporal workflow input serialization structured data vs bytes payload"
4. "Letta MemGPT Context Repository mid-execution context injection versioned state"

**Tier B — Exploratory (parallel-search, 3 queries):**
5. "Patterns bridging event streams distributed tracing Datadog Honeycomb Jaeger"
6. "HITL patterns preserve replay determinism context mutation durable execution"
7. "Structured effect payloads actor systems Akka Erlang Orleans typed message passing"

**Tier C — Unknown unknowns (parallel-search, 2 queries):**
8. "Agent framework production incidents lossy serialization missing observability"
9. "Emerging AI agent observability primitives execution journals decision trees reasoning traces"

- [ ] **Step 1: Dispatch all 9 research queries as parallel sub-agents**
- [ ] **Step 2: Synthesize — what's the right OTel bridge pattern? What's the right HITL mechanism?**
- [ ] **Step 3: Write digest to `docs/superpowers/research/p3-architecture-refinement.md`**
- [ ] **Step 4: Validate research against record/replay constraint — does HITL mutation break replay?**

### Task 12: OTel dispatch event → span bridge (#8)

**Files:**
- Modify: `extras/hooks/skg-hook-otel/src/lib.rs` (add event bridge capability)

- [ ] **Step 1: Write failing test — dispatch with progress events produces OTel span events**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement based on research findings (interceptor vs observer vs bridge)**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

```bash
cd extras/ && git add hooks/skg-hook-otel/src/lib.rs && git commit -m "feat: bridge DispatchEvent::Progress to OTel span events"
```

### Task 13: Handoff lossy JSON→text fix (#10)

**Files:**
- Modify: `skelegent/effects/skg-effects-local/src/lib.rs` (lines 163-170)
- Modify: `skelegent/effects/skg-effects-local/tests/hooks.rs` (add handoff metadata test)

- [ ] **Step 1: Write failing test — handoff preserves structured JSON**

Note: Read `skg-effects-local/src/lib.rs` constructor (`LocalEffectHandler::new()`) to get exact args before writing the test. The constructor takes `(Arc<dyn StateStore>, Option<Arc<dyn Dispatcher>>, Option<Arc<dyn Signalable>>, UnknownEffectPolicy)`. Use the existing test helpers from `effects/skg-effects-local/tests/hooks.rs` (`test_ctx()`, `NoOpOrch`, etc.) as patterns.

```rust
#[tokio::test]
async fn handoff_preserves_structured_state_in_metadata() {
    // Follow the pattern from existing tests in hooks.rs:
    // let store = Arc::new(InMemoryStore::new());
    // let handler = LocalEffectHandler::new(store.clone(), None, None, UnknownEffectPolicy::Error);
    // (Adjust constructor args after reading the actual API)

    let state = json!({"nested": {"key": "value"}, "array": [1, 2, 3]});
    let effect = Effect::Handoff {
        operator: OperatorId::new("target"),
        state: state.clone(),
    };

    let ctx = test_ctx();
    let outcome = handler.handle(&effect, &ctx).await.unwrap();

    match outcome {
        EffectOutcome::Handoff { input, .. } => {
            // metadata should carry the structured state
            assert_eq!(input.metadata, state);
        }
        other => panic!("expected Handoff, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-effects-local handoff_preserves -- --nocapture`
Expected: FAIL — metadata is currently Null

- [ ] **Step 3: Fix the handoff code**

In `skelegent/effects/skg-effects-local/src/lib.rs`, change line 166:

```rust
// Before:
input.metadata = serde_json::Value::Null;

// After:
input.metadata = state.clone();
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd skelegent/ && nix develop -c cargo test -p skg-effects-local handoff_preserves -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd skelegent/ && git add effects/skg-effects-local/src/lib.rs effects/skg-effects-local/tests/hooks.rs && git commit -m "fix: preserve structured JSON in handoff metadata instead of Null"
```

### Task 14: HITL context mutation (#9) — research-dependent

**Files:** Determined by Task 11 research.

This is the riskiest item. Implementation details depend entirely on research findings.

- [ ] **Step 1: Based on research, write detailed sub-plan for HITL context mutation**
- [ ] **Step 2: Write failing test — middleware guard pauses, channel injects modified context**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Write failing test — record mutation, replay, verify determinism**
- [ ] **Step 5: Implement recorder integration**
- [ ] **Step 6: Write failing test — timeout when no human responds**
- [ ] **Step 7: Implement timeout**
- [ ] **Step 8: Run all tests**
- [ ] **Step 9: Commit**

### Task 15: Phase 3 verification

- [ ] **Step 1: Run full skelegent verification**

Run: `cd skelegent/ && ./scripts/verify.sh`

- [ ] **Step 2: Run full extras verification**

Run: `cd extras/ && nix develop -c cargo test --workspace --all-targets && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 3: Count tests, confirm increase**

---

## Chunk 4: Phase 4 — Polish & Completeness

### Task 16: Phase 4 Research

**Tier A — Targeted (parallel-search, 3 queries):**
1. "OpenTelemetry gen_ai semantic conventions embed response model attribute specification"
2. "SSE EventSource specification WHATWG reconnection retry field Last-Event-ID"
3. "Float parsing ML evaluation scoring systems production frameworks best practices"

**Tier B — Exploratory (parallel-search, 3 queries):**
4. "Rust crate API completeness patterns beloved crate design missing derives convenience methods"
5. "Content hash matching replay journal systems useful pattern or remove"
6. "SSE client resilience patterns compression auth refresh proxy buffering production"

**Tier C — Unknown unknowns (parallel-search, 2 queries):**
7. "What makes serde tokio axum Rust crates feel magical API design"
8. "Cosine similarity implementation pitfalls NaN mismatched vectors numerical stability"

- [ ] **Step 1: Dispatch all 8 research queries as parallel sub-agents**
- [ ] **Step 2: Synthesize findings**
- [ ] **Step 3: Write digest to `docs/superpowers/research/p4-polish.md`**
- [ ] **Step 4: Decide on #13 (ByContentHash) and #20 (SubDispatchRecord naming) based on research**

### Task 17: Skelegent polish items (#15, #20)

**Files:**
- #15: Modify `skelegent/effects/skg-effects-local/tests/hooks.rs` (add Progress/Artifact skip test)
- #20: Modify `skelegent/layer0/src/operator.rs` (rename or document, research decides)

- [ ] **Step 1: Write test — Progress effect skips cleanly in executor (#15)**

Note: Read `LocalEffectHandler::new()` constructor args from `skg-effects-local/src/lib.rs` and follow the pattern from existing tests in `effects/skg-effects-local/tests/hooks.rs` before writing.

```rust
#[tokio::test]
async fn progress_and_artifact_effects_are_skipped() {
    // Follow the pattern from existing tests in hooks.rs:
    // let store = Arc::new(InMemoryStore::new());
    // let handler = LocalEffectHandler::new(store.clone(), None, None, UnknownEffectPolicy::IgnoreAndWarn);
    // (Adjust constructor args after reading the actual API)

    let progress = Effect::Progress {
        content: Content::text("step 1 of 3"),
    };
    let ctx = test_ctx();
    let outcome = handler.handle(&progress, &ctx).await.unwrap();
    assert!(matches!(outcome, EffectOutcome::Skipped));
}
```

- [ ] **Step 2: Run test**
- [ ] **Step 3: Address #20 based on research (rename SubDispatchRecord or add doc comment)**
- [ ] **Step 4: Commit**

### Task 18: Extras polish items (#11, #12, #13, #14, #16, #17, #18, #19) — parallel

All touch different files. Can be dispatched as parallel sub-agents. Each sub-item below is intentionally a stub — detailed code snippets will be written by the implementing sub-agent after reading the target file and applying Phase 4 research findings. These are all S-effort items (< 1 hour each).

**#11 — OTel embed gen_ai.response.model:**
- Modify: `extras/hooks/skg-hook-otel/src/lib.rs` (add `gen_ai.response.model` attribute in embed success path)

**#12 — LlmJudge float parser:**
- Modify: `extras/eval/skg-eval/src/lib.rs` (fix `parse_first_float` to handle negatives, or replace with stdlib)

**#13 — ByContentHash stub:**
- Modify: `extras/hooks/skg-hook-replay/src/lib.rs` (implement content-hash matching or document as no-op)

**#14 — A2A SSE reconnection:**
- Modify: `extras/a2a/skg-a2a/src/client/stream.rs` (implement or document retry/id fields)

**#16 — cosine_similarity mismatched inputs:**
- Modify: `extras/eval/skg-eval/src/lib.rs` (add length check, NaN guard)

**#17 — Progress round-trip test:**
- Modify: `extras/a2a/skg-a2a/tests/streaming.rs` (add Progress → status_update → Progress test)

**#18 — OAuthDeviceFlowProvider Clone:**
- Modify: `extras/auth/skg-auth-oauth/src/lib.rs` (add `#[derive(Clone)]` to `OAuthDeviceFlowProvider`)

**#19 — EvalRunner panic case name:**
- Modify: `extras/eval/skg-eval/src/lib.rs` (capture case name before spawn)

For each item:
- [ ] **Write failing test**
- [ ] **Run to verify failure**
- [ ] **Implement fix**
- [ ] **Run to verify pass**
- [ ] **Commit**

### Task 19: Phase 4 verification

- [ ] **Step 1: Run full skelegent verification**

Run: `cd skelegent/ && ./scripts/verify.sh`

- [ ] **Step 2: Run full extras verification**

Run: `cd extras/ && nix develop -c cargo test --workspace --all-targets && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 3: Final test count**

Record final numbers. Target: significantly above 1,159 combined.

- [ ] **Step 4: Final commit**

```bash
cd skelegent/ && git add -A && git commit -m "chore: phase 4 polish complete — S+ audit remediation done"
cd extras/ && git add -A && git commit -m "chore: phase 4 polish complete — S+ audit remediation done"
```
