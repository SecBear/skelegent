# Durable-Orch Consolidation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the three good design ideas from `origin/integration/durable-orch` onto `main`, then delete the remote branch so we can do one clean push.

**Architecture:** Effects move into `Context` (eliminating the `EffectEmitter` parameter from all signatures). `CognitiveOperator` is slimmed to a thin adapter that delegates to the `DispatchContext` it receives. The budget bug is fixed. All changes happen on `main` using the existing architecture (middleware, `DispatchContext`, `EffectHandler`).

**Tech Stack:** Rust, async_trait, tokio, layer0, skg-context-engine, skg-tool

**Verification commands (run after every phase):**
```bash
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

---

## Phase 1: Fix the budget bug (isolated, no API change)

This is a one-line correctness fix with a test change. Ship it first because it is independent of everything else.

### Task 1.1: Fix tool_calls exceeded returning wrong ExitReason

**Files:**
- Modify: `op/skg-context-engine/src/rules/budget.rs:142-143`
- Modify: `op/skg-context-engine/src/rules/budget.rs` (test at bottom)

**Bug:** When `max_tool_calls` is exceeded, `BudgetGuard` returns `ExitReason::MaxTurns` instead of `ExitReason::BudgetExhausted`. `MaxTurns` is semantically wrong — tool call exhaustion is a budget concern, not a turn count concern.

- [ ] **Step 1: Write a failing test exposing the bug**

Add to `budget.rs` tests:

```rust
#[tokio::test]
async fn budget_guard_tool_limit_returns_budget_exhausted_not_max_turns() {
    let mut ctx = Context::new();
    ctx.metrics.tool_calls_total = 20;

    let guard = BudgetGuard::with_config(BudgetGuardConfig {
        max_cost: None,
        max_turns: None,
        max_duration: None,
        max_tool_calls: Some(10),
    });

    let err = guard.execute(&mut ctx).await.unwrap_err();
    assert_exit(err, ExitReason::BudgetExhausted);
}
```

- [ ] **Step 2: Run the test — expect FAIL**

```bash
nix develop -c cargo test -p skg-context-engine budget_guard_tool_limit_returns_budget_exhausted -- --exact
```

Expected: FAIL — currently returns `MaxTurns`, test expects `BudgetExhausted`.

- [ ] **Step 3: Fix the bug**

In `budget.rs` line 143, change:

```rust
// Before:
ExitReason::MaxTurns,
// After:
ExitReason::BudgetExhausted,
```

- [ ] **Step 4: Run the test — expect PASS**

```bash
nix develop -c cargo test -p skg-context-engine budget_guard -- --nocapture
```

Expected: all budget_guard tests pass.

- [ ] **Step 5: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "fix(context-engine): tool call limit returns BudgetExhausted not MaxTurns"
```

---

## Phase 2: Move effects into Context (the big design change)

This is the highest-value change. It eliminates the `EffectEmitter` parameter from `react_loop`, `react_loop_structured`, `stream_react_loop`, `dispatch_function_tools`, the `Operator` trait, and 73+ call sites that pass `EffectEmitter::noop()`.

The key insight: `OperatorOutput` already has a `pub effects: Vec<Effect>` field. And `make_output()` already builds `OperatorOutput` from `Context`. So effects belong on `Context`, and `make_output` should drain them into `OperatorOutput::effects`.

### Task 2.1: Add effects storage to Context

**Files:**
- Modify: `op/skg-context-engine/src/context.rs`
- Modify: `op/skg-context-engine/src/stream.rs` (add `ContextMutation::EffectDeclared`)

- [ ] **Step 1: Write a failing test**

```rust
// In context.rs tests
#[tokio::test]
async fn push_effect_stores_and_drains() {
    let mut ctx = Context::new();
    let effect = Effect::log("test effect");

    ctx.push_effect(effect.clone());
    assert_eq!(ctx.effects().len(), 1);

    let drained = ctx.drain_effects();
    assert_eq!(drained.len(), 1);
    assert!(ctx.effects().is_empty());
}
```

- [ ] **Step 2: Run test — expect FAIL** (method doesn't exist)

- [ ] **Step 3: Implement**

Add to `Context` struct:

```rust
/// Effects declared during this operator invocation.
/// Drained by `make_output` into `OperatorOutput::effects`.
effects: Vec<Effect>,
```

Add methods:

```rust
/// Declare an effect. Stored until drained into OperatorOutput.
pub fn push_effect(&mut self, effect: Effect) {
    if let Some(ref tx) = self.event_tx {
        let _ = tx.send(ContextEvent {
            mutation: ContextMutation::EffectDeclared(effect.clone()),
            message_count: self.messages.len(),
        });
    }
    self.effects.push(effect);
}

/// Declare multiple effects.
pub fn extend_effects(&mut self, effects: impl IntoIterator<Item = Effect>) {
    for effect in effects {
        self.push_effect(effect);
    }
}

/// Read current effects without draining.
pub fn effects(&self) -> &[Effect] {
    &self.effects
}

/// Drain all effects (transfers ownership to caller).
pub fn drain_effects(&mut self) -> Vec<Effect> {
    std::mem::take(&mut self.effects)
}
```

Add `ContextMutation::EffectDeclared(Effect)` variant to the enum in `stream.rs`.

Initialize `effects: Vec::new()` in `Context::new()` and `Context::with_rules()`.

- [ ] **Step 4: Run test — expect PASS**

- [ ] **Step 5: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "feat(context-engine): add effect storage to Context"
```

### Task 2.2: Wire make_output to drain effects from Context

**Files:**
- Modify: `op/skg-context-engine/src/react.rs` (`make_output` function)

- [ ] **Step 1: Change make_output**

```rust
fn make_output(response: InferResponse, exit: ExitReason, ctx: &mut Context) -> OperatorOutput {
    let mut output = OperatorOutput::new(response.content, exit);
    let mut meta = OperatorMetadata::default();
    meta.tokens_in = ctx.metrics.tokens_in;
    meta.tokens_out = ctx.metrics.tokens_out;
    meta.cost = ctx.metrics.cost;
    meta.turns_used = ctx.metrics.turns_completed;
    meta.duration = DurationMs::from_millis(ctx.metrics.elapsed_ms());
    output.metadata = meta;
    output.effects = ctx.drain_effects();
    output
}
```

Note: `ctx` parameter changes from `&Context` to `&mut Context` for `drain_effects()`. Update all callers of `make_output` in `react.rs` accordingly (they already have `&mut Context`).

- [ ] **Step 2: Run tests — expect PASS** (no behavior change yet, effects vec was empty before)

- [ ] **Step 3: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "refactor(context-engine): make_output drains effects from Context"
```

### Task 2.3: Move approval effects from emitter to Context in react_loop

**Files:**
- Modify: `op/skg-context-engine/src/react.rs` (approval handling in `react_loop` and `dispatch_function_tools`)

This is the critical behavior change: instead of `emitter.effect(effect)`, we do `ctx.push_effect(effect)`.

- [ ] **Step 1: Write a test that verifies effects appear in OperatorOutput**

```rust
#[tokio::test]
async fn react_loop_approval_effects_appear_in_output() {
    // Set up a provider that returns a tool call requiring approval
    // Verify that the returned OperatorOutput.effects is non-empty
    // (Currently it's always empty because make_output hard-codes vec![])
}
```

Use the existing `react_loop_exits_on_approval_required` test as a template, but assert on `output.effects` length.

- [ ] **Step 2: Run — expect FAIL** (effects currently lost)

- [ ] **Step 3: Replace emitter calls with ctx.push_effect in react_loop**

In `react_loop` (~lines 251-254):
```rust
// Before:
for effect in &approval_effects {
    emitter.effect(effect.clone()).await;
}
// After:
ctx.extend_effects(approval_effects);
```

In `dispatch_function_tools` (~lines 534-537):
```rust
// Before:
for effect in &approval_effects {
    emitter.effect(effect.clone()).await;
}
// After:
ctx.extend_effects(approval_effects);
```

- [ ] **Step 4: Run test — expect PASS**

- [ ] **Step 5: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "refactor(context-engine): approval effects stored in Context, not emitted"
```

### Task 2.4: Remove EffectEmitter from react_loop signatures

**Files:**
- Modify: `op/skg-context-engine/src/react.rs` (3 public fns + internal dispatch fn)
- Modify: `op/skg-context-engine/src/stream_react.rs` (1 public fn)
- Modify: `op/skg-context-engine/src/cognitive_operator.rs` (caller)
- Modify: `op/skg-context-engine/src/lib.rs` (re-exports if any)
- Modify: `op/skg-context-engine/src/stream_react.rs` tests
- Modify: `op/skg-context-engine/src/react.rs` tests
- Modify: `tests/cross_provider.rs`
- Modify: `tests/poc.rs`

This is the largest mechanical change. Every call site that passes `&EffectEmitter::noop()` or `emitter` loses that argument.

**Functions losing the `emitter` parameter:**

| Function | File |
|---|---|
| `react_loop()` | `react.rs` |
| `react_loop_structured()` | `react.rs` |
| `dispatch_function_tools()` | `react.rs` (private) |
| `stream_react_loop()` | `stream_react.rs` |

- [ ] **Step 1: Remove `emitter: &EffectEmitter` from all four function signatures**

- [ ] **Step 2: Update CognitiveOperator::execute to stop passing emitter**

In `cognitive_operator.rs`, the `react_loop(...)` call drops the `emitter` arg. The `emitter` parameter on `execute()` is still required by the `Operator` trait — we handle that in Phase 3. For now, just stop forwarding it.

- [ ] **Step 3: Update all test call sites**

Use `lsp references` or grep to find every `react_loop(`, `react_loop_structured(`, `stream_react_loop(` call and remove the `&EffectEmitter::noop()` argument. There are ~40 sites in react.rs tests, ~10 in stream_react.rs tests, ~7 in cross_provider.rs, ~5 in poc.rs.

- [ ] **Step 4: Run full test suite**

```bash
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all pass, no unused import warnings for EffectEmitter in context-engine.

- [ ] **Step 5: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "refactor(context-engine): remove EffectEmitter from react_loop signatures"
```

---

## Phase 3: Remove EffectEmitter from the Operator trait

This is the layer0 API change. It affects every `Operator` implementation and every caller of `operator.execute()`.

### Task 3.1: Remove emitter from Operator::execute signature

**Files:**
- Modify: `layer0/src/operator.rs` (trait definition + blanket impls)
- Modify: `layer0/src/dispatch.rs` (EffectEmitter struct stays — it's still used by DispatchHandle for progress/artifact events, just not threaded through Operator)
- Modify: every `impl Operator for ...` across the workspace
- Modify: every `.execute(input, &ctx, &EffectEmitter::noop())` call site

**Important:** `EffectEmitter` is NOT deleted. It still serves `progress()` and `artifact()` for streaming dispatch events. We only remove it from the `Operator::execute` signature because *effect declaration* now goes through `Context`.

The `DispatchHandle::collect()` method currently scrapes `EffectEmitted` events from the channel to populate `output.effects`. After this change, `output.effects` is populated by the operator itself (via `Context::drain_effects()`). The collect path becomes a fallback for operators that emit effects through the channel directly (non-context-engine operators).

- [ ] **Step 1: Change the trait**

```rust
#[async_trait]
pub trait Operator: Send + Sync {
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
    ) -> Result<OperatorOutput, OperatorError>;
}
```

- [ ] **Step 2: Find and update all implementations**

Use `lsp implementation` on `Operator::execute` to find every concrete impl. Update each to drop the `emitter` parameter. Key files:

- `op/skg-context-engine/src/cognitive_operator.rs`
- `op/skg-op-single-shot/src/lib.rs`
- `examples/custom_operator_barrier/src/lib.rs`
- `examples/hello-claude/main.rs`
- `layer0/src/operator.rs` (test mocks)
- `layer0/tests/phase2.rs`
- Any other `impl Operator` across the workspace

- [ ] **Step 3: Find and update all call sites**

Use grep for `.execute(` combined with `EffectEmitter::noop()` to find all 73 call sites. Remove the emitter argument from each.

Key areas:
- `layer0/tests/phase2.rs`
- `op/skg-op-single-shot/src/lib.rs` tests
- `op/skg-context-engine/src/cognitive_operator.rs` tests
- `tests/poc.rs`
- `tests/cross_provider.rs`
- `turn/skg-mcp/src/server.rs`
- `turn/skg-tool/src/adapter.rs`
- `env/skg-env-local/src/lib.rs`
- `skelegent/src/agent.rs`
- `examples/hello-claude/main.rs`

- [ ] **Step 4: Update dispatch.rs collect() logic**

In `DispatchHandle::collect()` and `collect_all()`, keep the `EffectEmitted` event scraping as a fallback, but add a comment that context-engine operators now populate `output.effects` directly.

- [ ] **Step 5: Run full verification**

```bash
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "refactor(layer0): remove EffectEmitter from Operator::execute signature"
```

---

## Phase 4: Slim CognitiveOperator

Now that effects are on Context and the Operator trait is cleaner, slim `CognitiveOperator` to a thin adapter.

### Task 4.1: Use the received DispatchContext instead of fabricating one

**Files:**
- Modify: `op/skg-context-engine/src/cognitive_operator.rs`

**Bug:** `CognitiveOperator::execute` receives `ctx: &DispatchContext` from the dispatcher but ignores it (`_ctx`). Instead, it fabricates a synthetic `DispatchContext` with `DispatchId::new(format!("cogop-{}-{}", ...))`. This means tool calls inside `react_loop` get a fake identity that doesn't correlate with the actual dispatch chain.

- [ ] **Step 1: Write a test that verifies the received DispatchContext reaches tool calls**

Set up a `CognitiveOperator` with a tool that captures the `DispatchContext` it receives. Assert that the `dispatch_id` matches what the test passed in, not a synthetic `cogop-*` ID.

- [ ] **Step 2: Run — expect FAIL** (currently gets synthetic ID)

- [ ] **Step 3: Fix**

Replace the synthetic construction with the received context:

```rust
async fn execute(
    &self,
    input: OperatorInput,
    ctx: &DispatchContext,
) -> Result<OperatorOutput, OperatorError> {
    let mut engine_ctx = self.create_context();
    // ... inject system prompt and user message ...

    react_loop(
        &mut engine_ctx,
        &self.provider,
        &self.tools,
        ctx,  // Use the real DispatchContext, not a fabricated one
        &config,
    )
    .await
    .map_err(map_engine_error)
}
```

Delete the synthetic `DispatchContext::new(...)` block (~lines 184-190).

- [ ] **Step 4: Run test — expect PASS**

- [ ] **Step 5: Run full suite to check for regressions**

- [ ] **Step 6: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "fix(context-engine): CognitiveOperator uses received DispatchContext"
```

### Task 4.2: Audit and slim CognitiveOperator (optional cleanup)

**Files:**
- Modify: `op/skg-context-engine/src/cognitive_operator.rs`

Review whether `CognitiveOperatorConfig` duplicates `ReactLoopConfig` fields. If so, collapse them. Review whether `RuleFactory` should be a simpler pattern. This is cleanup, not a correctness issue.

- [ ] **Step 1: Audit config duplication**
- [ ] **Step 2: If duplicated, merge into one config type or have CognitiveOperatorConfig contain ReactLoopConfig**
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
nix develop -c ./scripts/agent-commit.sh "refactor(context-engine): slim CognitiveOperator config"
```

---

## Phase 5: Cleanup and delete the remote branch

### Task 5.1: Final verification and commit

- [ ] **Step 1: Run full verification**

```bash
nix develop -c nix fmt
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: Review git log for clean commit history**

```bash
git log --oneline -20
```

- [ ] **Step 3: Delete the remote branch**

```bash
git push origin --delete integration/durable-orch
```

- [ ] **Step 4: Clean up remote refs**

```bash
git remote prune origin
```

---

## Dependency graph

```
Phase 1 (budget bug)     — independent, ship first
Phase 2 (effects-in-ctx) — independent of Phase 1, 4 sequential tasks
Phase 3 (Operator trait)  — depends on Phase 2 completing
Phase 4 (slim CogOp)     — depends on Phase 3 completing
Phase 5 (cleanup)         — depends on all above
```

Phases 1 and 2 can run in parallel. Phase 3 must wait for Phase 2. Phase 4 must wait for Phase 3.

## Risk notes

- **Phase 3 is the widest blast radius.** Changing the `Operator` trait signature touches every operator impl and every test that calls `.execute()`. Use `lsp references` to find all sites before starting; do not rely on grep alone.
- **`DispatchHandle::collect()` must keep working.** Non-context-engine operators may still emit effects via the channel. The scraping logic in `collect()` must remain as a fallback.
- **`EffectEmitter` is NOT deleted.** It still serves `progress()` and `artifact()` on the dispatch channel. Only its role in *effect declaration* is replaced by `Context`.
- **The `_ctx` bug in CognitiveOperator** means existing tests fabricate their own `DispatchContext` via `test_ctx()`. After the fix, those test contexts will actually be threaded through. Verify no test relies on the synthetic ID format.
