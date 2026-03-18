# Ergonomics & Polish Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Elevate skelegent from 8/10 (works as designed) to 9/10 (delightful to use) and lay groundwork for 10/10 (feels magical).

**Architecture:** Two tracks — ergonomics (8→9) and polish (9→10). Ergonomics items are builder methods, conversions, re-exports that eliminate API friction. Polish items are documentation, pre-built utilities, and production hardening.

**Tech Stack:** Rust (edition 2024), tokio, async-trait, serde. Nix-provided tooling. TDD throughout.

**Spec:** `docs/superpowers/specs/2026-03-16-s-plus-audit-remediation-design.md` (original audit) + this plan's own findings.

**Repos:** skelegent @ `/Users/bear/dev/golden-neuron/skelegent/`, extras @ `/Users/bear/dev/golden-neuron/extras/`

---

## File Map

### Foundation (Track 0 — Architectural)

| File | Changes |
|---|---|
| `ARCHITECTURE.md` | Add "Middleware Blueprint" section documenting the stack pattern |
| `layer0/src/middleware.rs` | Add blueprint comment headers linking to ARCHITECTURE.md |
| `turn/skg-turn/src/infer_middleware.rs` | Add blueprint comment headers |
| `secret/skg-secret/src/middleware.rs` | Add blueprint comment headers |
| `turn/skg-turn/src/provider.rs` | Move DynProvider here with blanket impl |
| `turn/skg-turn/src/lib.rs` | Re-export DynProvider, box_provider |
| `provider/skg-provider-router/src/lib.rs` | Remove DynProvider definition, import from skg-turn |
| `tests/middleware_contract.rs` | New — contract tests all 6 stacks must pass |

### Skelegent Core (Track A — Ergonomics)

| File | Changes |
|---|---|
| `layer0/src/operator.rs` | OperatorInput builder, OperatorConfig builder, ExitReason/TriggerType helpers + Display |
| `turn/skg-turn/src/infer.rs` | InferResponse → Content conversion, InferResponse::into_message() |
| `secret/skg-secret/src/lib.rs` | Re-export middleware types at crate root |
| `layer0/src/lib.rs` | Verify re-exports are complete |

### Extras (Track B — Production Hardening)

| File | Changes |
|---|---|
| `eval/skg-eval/src/lib.rs` | LLM judge retry/timeout, embedding cache |
| `hooks/skg-hook-approval/src/lib.rs` | Audit logging wrapper, dynamic policy |
| `hooks/skg-hook-replay/src/lib.rs` | Structured ReplayError context |
| `a2a/skg-a2a/src/client/mod.rs` | A2aDispatcher builder, interface fallback |
| `hooks/skg-hook-otel/src/lib.rs` | Complete gen_ai.* semconv attributes |

### Cross-Cutting (Track C — Integration & Docs)

| File | Changes |
|---|---|
| `skelegent/tests/` | Cross-crate integration tests |
| `extras/tests/` | Cross-crate integration tests |

---

## Chunk 0: Foundation — Architectural Improvements

### Task 0A: Middleware Blueprint + Contract Tests

**Files:**
- Modify: `skelegent/ARCHITECTURE.md` (add Middleware Blueprint section)
- Modify: `skelegent/layer0/src/middleware.rs` (add blueprint header comments)
- Modify: `skelegent/turn/skg-turn/src/infer_middleware.rs` (add blueprint header comments)
- Modify: `skelegent/secret/skg-secret/src/middleware.rs` (add blueprint header comments)
- Create: `skelegent/tests/middleware_contract.rs` (contract tests for all 6 stacks)

**Context:** 6 middleware stacks follow identical patterns (~400 LOC of structural repetition). Research confirms this is intentional — Tower uses one `Service` type but skelegent has 7 different method signatures per boundary. The repetition IS the pattern. Make it obviously intentional.

- [ ] **Step 1: Add Middleware Blueprint section to ARCHITECTURE.md**

Document the exact pattern: traits → stack → builder → chain. Include a template showing what a new middleware boundary needs. Explain WHY each stack is hand-written (different type signatures, object safety constraints, IDE navigability).

- [ ] **Step 2: Add comment headers to each middleware file**

At the top of each Stack section, add:
```rust
// This stack follows the Middleware Blueprint (ARCHITECTURE.md § Middleware Pattern).
// Traits are hand-written (each boundary has unique method signatures).
// Stack, Builder, Chain are structurally identical across all 6 boundaries.
```

- [ ] **Step 3: Write contract tests**

Create `skelegent/tests/middleware_contract.rs` that tests the invariants all stacks share:
1. Empty stack passes through to terminal
2. Observer middleware sees all calls and always forwards
3. Guard middleware can short-circuit (not call next)
4. Multiple middleware execute in observe → transform → guard order
5. Stack builder produces correct ordering

Test each of the 6 stacks (Dispatch, StoreWrite, StoreRead, Exec, Infer, Embed, Secret) against these invariants.

- [ ] **Step 4: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --test middleware_contract -- --nocapture`

- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add ARCHITECTURE.md layer0/src/middleware.rs turn/skg-turn/src/infer_middleware.rs secret/skg-secret/src/middleware.rs tests/middleware_contract.rs && git commit -m "docs: add Middleware Blueprint to ARCHITECTURE.md + contract tests for all 6 stacks"
```

---

### Task 0B: Move DynProvider to skg-turn with blanket impl

**Files:**
- Modify: `skelegent/turn/skg-turn/src/provider.rs` (add DynProvider, blanket impl, box_provider)
- Modify: `skelegent/turn/skg-turn/src/lib.rs` (re-export DynProvider, box_provider)
- Modify: `skelegent/provider/skg-provider-router/src/lib.rs` (remove DynProvider definition, import from skg-turn)

**Context:** `DynProvider` (object-safe provider wrapper) currently lives in `skg-provider-router` but logically belongs next to `Provider` in `skg-turn`. The erased companion trait pattern (Rust 2025-2026 standard) means implementing `Provider` automatically gives you `DynProvider` via blanket impl. Rig does this with dyn-compatible traits from the start.

**BREAKING CHANGE:** Users importing `DynProvider` from router will need to change imports. This is fine — we're not afraid of breaking changes right now.

- [ ] **Step 1: Read current DynProvider in skg-provider-router**

Read: `skelegent/provider/skg-provider-router/src/lib.rs` — understand the full DynProvider trait definition, all methods, and how it's used by RoutingProvider and MiddlewareProvider.

- [ ] **Step 2: Add DynProvider + blanket impl to skg-turn/src/provider.rs**

```rust
use std::pin::Pin;
use std::future::Future;

/// Object-safe wrapper for [`Provider`].
///
/// You almost never implement this directly — implement [`Provider`] instead.
/// The blanket impl automatically provides `DynProvider` for any `Provider`.
///
/// Use `DynProvider` when you need:
/// - Heterogeneous collections: `Vec<Box<dyn DynProvider>>`
/// - Middleware stacks: `MiddlewareProvider` wraps `Box<dyn DynProvider>`
/// - Runtime provider selection: match on config → different provider
pub trait DynProvider: Send + Sync {
    /// Boxed inference call.
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>>;

    /// Boxed embedding call.
    fn embed_boxed(
        &self,
        request: EmbedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EmbedResponse, ProviderError>> + Send + '_>>;
}

/// Blanket impl: any [`Provider`] is automatically a [`DynProvider`].
impl<T: Provider + Send + Sync> DynProvider for T {
    fn infer_boxed(
        &self,
        request: InferRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferResponse, ProviderError>> + Send + '_>> {
        Box::pin(self.infer(request))
    }

    fn embed_boxed(
        &self,
        request: EmbedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EmbedResponse, ProviderError>> + Send + '_>> {
        Box::pin(self.embed(request))
    }
}

/// Box any [`Provider`] for use with [`RoutingProvider`] or [`MiddlewareProvider`].
///
/// ```rust,ignore
/// use skg_turn::box_provider;
/// let boxed = box_provider(my_anthropic_provider);
/// ```
pub fn box_provider(provider: impl Provider + 'static) -> Box<dyn DynProvider> {
    Box::new(provider)
}
```

Note: Check current DynProvider for any additional methods (stream, etc.) and include them.

- [ ] **Step 3: Update skg-turn/src/lib.rs re-exports**

Add: `pub use provider::{DynProvider, box_provider};`

- [ ] **Step 4: Update skg-provider-router — remove DynProvider, import from skg-turn**

Remove the DynProvider trait definition from `skg-provider-router/src/lib.rs`. Replace with:
```rust
use skg_turn::provider::{DynProvider, box_provider};
// Or re-export for backwards compat:
pub use skg_turn::provider::{DynProvider, box_provider};
```

Update RoutingProvider and MiddlewareProvider to use the imported DynProvider.

- [ ] **Step 5: Fix all compilation errors across workspace**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo check --workspace`

Fix any import errors in downstream crates (examples, tests, extras).

- [ ] **Step 6: Run full test suite**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --workspace --all-targets`

- [ ] **Step 7: Run clippy**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 8: Fix extras if it imports DynProvider from router**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo check --workspace`

- [ ] **Step 9: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add turn/skg-turn/src/provider.rs turn/skg-turn/src/lib.rs provider/skg-provider-router/src/lib.rs && git commit -m "refactor: move DynProvider to skg-turn with blanket impl (breaking: import path changed)"
```

---

## Chunk 1: Track A — Skelegent Core Ergonomics

### Task 1: OperatorInput builder methods

**Files:**
- Modify: `skelegent/layer0/src/operator.rs` (add methods to `impl OperatorInput` after line 262)
- Test: same file, `#[cfg(test)]` module

**Context:** `OperatorInput` is the most frequently constructed type. Current API requires mutable field access after `new()`. Fields: `session: Option<SessionId>`, `config: Option<OperatorConfig>`, `metadata: serde_json::Value`, `context: Option<Vec<Message>>`.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn operator_input_builder_methods() {
    use crate::content::Content;
    use crate::id::SessionId;

    let input = OperatorInput::new(Content::text("hello"), TriggerType::User)
        .with_session(SessionId::new("sess-1"))
        .with_config(OperatorConfig { max_turns: Some(5), ..Default::default() })
        .with_metadata(serde_json::json!({"trace": "abc"}));

    assert_eq!(input.session.as_ref().unwrap().as_str(), "sess-1");
    assert_eq!(input.config.as_ref().unwrap().max_turns, Some(5));
    assert_eq!(input.metadata["trace"], "abc");
}
```

- [ ] **Step 2: Run test — should fail (methods don't exist)**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test -p layer0 operator_input_builder -- --nocapture`

- [ ] **Step 3: Implement builder methods**

Add to `impl OperatorInput` in `layer0/src/operator.rs` after the `new()` method:

```rust
/// Set the session ID for conversation continuity.
pub fn with_session(mut self, session: SessionId) -> Self {
    self.session = Some(session);
    self
}

/// Set per-invocation configuration overrides.
pub fn with_config(mut self, config: OperatorConfig) -> Self {
    self.config = Some(config);
    self
}

/// Set opaque metadata (tracing, routing, domain-specific context).
pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
    self.metadata = metadata;
    self
}

/// Set pre-assembled context from the caller.
pub fn with_context(mut self, context: Vec<Message>) -> Self {
    self.context = Some(context);
    self
}
```

- [ ] **Step 4: Run test — should pass**
- [ ] **Step 5: Run all layer0 tests**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test -p layer0 --all-targets`

- [ ] **Step 6: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add layer0/src/operator.rs && git commit -m "feat: add OperatorInput builder methods (with_session, with_config, with_metadata, with_context)"
```

---

### Task 2: OperatorConfig builder methods

**Files:**
- Modify: `skelegent/layer0/src/operator.rs` (add methods to `impl OperatorConfig`)

**Context:** `OperatorConfig` has fields: `max_turns: Option<u32>`, `max_cost: Option<Decimal>`, `max_duration: Option<DurationMs>`, `model: Option<String>`, `allowed_operators: Option<Vec<String>>`, `system_addendum: Option<String>`. Derives `Default`.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn operator_config_builder_methods() {
    let config = OperatorConfig::default()
        .with_max_turns(10)
        .with_model("claude-opus-4-6")
        .with_system_addendum("Be concise.");

    assert_eq!(config.max_turns, Some(10));
    assert_eq!(config.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(config.system_addendum.as_deref(), Some("Be concise."));
}
```

- [ ] **Step 2: Run test — fail**
- [ ] **Step 3: Implement**

```rust
impl OperatorConfig {
    /// Set the maximum number of ReAct loop iterations.
    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }

    /// Set the maximum cost budget in USD.
    pub fn with_max_cost(mut self, max_cost: Decimal) -> Self {
        self.max_cost = Some(max_cost);
        self
    }

    /// Set the maximum wall-clock duration.
    pub fn with_max_duration(mut self, max_duration: DurationMs) -> Self {
        self.max_duration = Some(max_duration);
        self
    }

    /// Override the model for this invocation.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Restrict which operators/tools this invocation can use.
    pub fn with_allowed_operators(mut self, operators: Vec<String>) -> Self {
        self.allowed_operators = Some(operators);
        self
    }

    /// Append additional system prompt content.
    pub fn with_system_addendum(mut self, addendum: impl Into<String>) -> Self {
        self.system_addendum = Some(addendum.into());
        self
    }
}
```

- [ ] **Step 4: Run test — pass**
- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add layer0/src/operator.rs && git commit -m "feat: add OperatorConfig builder methods (with_max_turns, with_model, etc.)"
```

---

### Task 3: ExitReason + TriggerType helpers and Display

**Files:**
- Modify: `skelegent/layer0/src/operator.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn exit_reason_convenience_constructors() {
    let halt = ExitReason::interceptor_halt("budget exceeded");
    assert!(matches!(halt, ExitReason::InterceptorHalt { reason } if reason == "budget exceeded"));

    let stop = ExitReason::safety_stop("content filtered");
    assert!(matches!(stop, ExitReason::SafetyStop { reason } if reason == "content filtered"));

    let custom = ExitReason::custom("my-reason");
    assert!(matches!(custom, ExitReason::Custom(s) if s == "my-reason"));
}

#[test]
fn trigger_type_convenience_constructors() {
    let custom = TriggerType::custom("webhook");
    assert!(matches!(custom, TriggerType::Custom(s) if s == "webhook"));
}

#[test]
fn exit_reason_display() {
    assert_eq!(ExitReason::Complete.to_string(), "complete");
    assert_eq!(ExitReason::MaxTurns.to_string(), "max_turns");
    assert_eq!(
        ExitReason::InterceptorHalt { reason: "budget".into() }.to_string(),
        "interceptor_halt: budget"
    );
}

#[test]
fn trigger_type_display() {
    assert_eq!(TriggerType::User.to_string(), "user");
    assert_eq!(TriggerType::Custom("webhook".into()).to_string(), "custom: webhook");
}
```

- [ ] **Step 2: Run tests — fail**
- [ ] **Step 3: Implement constructors**

```rust
impl ExitReason {
    /// Create an `InterceptorHalt` exit reason.
    pub fn interceptor_halt(reason: impl Into<String>) -> Self {
        Self::InterceptorHalt { reason: reason.into() }
    }

    /// Create a `SafetyStop` exit reason.
    pub fn safety_stop(reason: impl Into<String>) -> Self {
        Self::SafetyStop { reason: reason.into() }
    }

    /// Create a `Custom` exit reason.
    pub fn custom(reason: impl Into<String>) -> Self {
        Self::Custom(reason.into())
    }
}

impl TriggerType {
    /// Create a `Custom` trigger type.
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }
}
```

- [ ] **Step 4: Implement Display**

```rust
impl std::fmt::Display for ExitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete => write!(f, "complete"),
            Self::MaxTurns => write!(f, "max_turns"),
            Self::BudgetExhausted => write!(f, "budget_exhausted"),
            Self::CircuitBreaker => write!(f, "circuit_breaker"),
            Self::Timeout => write!(f, "timeout"),
            Self::InterceptorHalt { reason } => write!(f, "interceptor_halt: {reason}"),
            Self::Error => write!(f, "error"),
            Self::SafetyStop { reason } => write!(f, "safety_stop: {reason}"),
            Self::AwaitingApproval => write!(f, "awaiting_approval"),
            Self::Custom(s) => write!(f, "custom: {s}"),
            _ => write!(f, "unknown"),
        }
    }
}

impl std::fmt::Display for TriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Task => write!(f, "task"),
            Self::Signal => write!(f, "signal"),
            Self::Schedule => write!(f, "schedule"),
            Self::SystemEvent => write!(f, "system_event"),
            Self::Custom(s) => write!(f, "custom: {s}"),
            _ => write!(f, "unknown"),
        }
    }
}
```

- [ ] **Step 5: Run tests — pass**
- [ ] **Step 6: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add layer0/src/operator.rs && git commit -m "feat: add ExitReason/TriggerType convenience constructors and Display impls"
```

---

### Task 4: InferResponse → Content conversion

**Files:**
- Modify: `skelegent/turn/skg-turn/src/infer.rs` (add conversion methods and From impl)

**Context:** `InferResponse` has `content: Content`, `tool_calls`, `stop_reason`, `usage`, `model`, `cost`. The hot path in every operator is extracting `Content` from `InferResponse` to build `OperatorOutput`. Currently requires `.content.clone()` or manual reconstruction.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn infer_response_into_content() {
    let response = InferResponse {
        content: Content::text("hello"),
        tool_calls: vec![],
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage::default(),
        model: "test".into(),
        cost: None,
    };

    // Owned conversion
    let content: Content = response.into_content();
    assert_eq!(content.as_text(), Some("hello"));
}

#[test]
fn infer_response_text_helper() {
    let response = InferResponse {
        content: Content::text("world"),
        tool_calls: vec![],
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage::default(),
        model: "test".into(),
        cost: None,
    };

    assert_eq!(response.text(), Some("world"));
}
```

- [ ] **Step 2: Run tests — fail**
- [ ] **Step 3: Implement**

Read the existing `InferResponse` impl block first. Then add:

```rust
impl InferResponse {
    /// Extract the text content as a string slice.
    ///
    /// Convenience method equivalent to `self.content.as_text()`.
    pub fn text(&self) -> Option<&str> {
        self.content.as_text()
    }

    /// Consume the response and return its content.
    ///
    /// Use when building `OperatorOutput` from an inference result:
    /// ```rust,ignore
    /// let output = OperatorOutput::new(response.into_content(), ExitReason::Complete);
    /// ```
    pub fn into_content(self) -> Content {
        self.content
    }
}

impl From<InferResponse> for Content {
    fn from(response: InferResponse) -> Self {
        response.content
    }
}
```

Note: Check if `text()` already exists on `InferResponse`. If it does, skip adding it.

- [ ] **Step 4: Run tests — pass**
- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add turn/skg-turn/src/infer.rs && git commit -m "feat: add InferResponse::into_content(), text(), and From<InferResponse> for Content"
```

---

### Task 5: Re-export coverage

**Files:**
- Modify: `skelegent/secret/skg-secret/src/lib.rs` (re-export middleware types)
- Verify: `skelegent/hooks/skg-hook-recorder/src/lib.rs` (already good)
- Verify: `skelegent/turn/skg-turn/src/lib.rs` (already good)

- [ ] **Step 1: Add re-exports to skg-secret**

In `skelegent/secret/skg-secret/src/lib.rs`, add after `pub mod middleware;`:

```rust
pub use middleware::{SecretMiddleware, SecretNext, SecretStack, SecretStackBuilder};
```

- [ ] **Step 2: Write test verifying imports work from crate root**

```rust
#[test]
fn secret_middleware_importable_from_crate_root() {
    // This test verifies re-exports compile. If it compiles, it passes.
    fn _assert_types() {
        let _: fn() -> skg_secret::SecretStackBuilder = skg_secret::SecretStack::builder;
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test -p skg-secret -- --nocapture`

- [ ] **Step 4: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add secret/skg-secret/src/lib.rs && git commit -m "feat: re-export SecretMiddleware types at skg-secret crate root"
```

---

## Chunk 2: Track B — Extras Production Hardening

### Task 6: LLM judge retry/timeout + embedding cache

**Files:**
- Modify: `extras/eval/skg-eval/src/lib.rs`

**Context:** `LlmJudgeMetric` dispatches to a judge operator but has no retry/timeout. `SemanticSimilarityMetric` embeds both expected and actual on every eval case — no caching.

- [ ] **Step 1: Read LlmJudgeMetric and SemanticSimilarityMetric implementations**

Read: `extras/eval/skg-eval/src/lib.rs` — find LlmJudgeMetric::score() and SemanticSimilarityMetric::score()

- [ ] **Step 2: Write failing test — judge timeout**

Test: Create a dispatcher that never responds (sleep forever). Judge metric should return a score of 0.0 with a timeout error message, not hang indefinitely.

```rust
#[tokio::test]
async fn llm_judge_times_out_on_slow_dispatch() {
    // Create a dispatcher that sleeps forever
    // Create LlmJudgeMetric with a timeout (new parameter)
    // Assert score is 0.0 and error mentions "timeout"
}
```

- [ ] **Step 3: Add timeout parameter to LlmJudgeMetric**

Add `timeout: Duration` field (default 30s). Wrap the dispatch call in `tokio::time::timeout()`. On timeout, return Score { value: 0.0, reason: "judge timed out after Xs" }.

- [ ] **Step 4: Run test — pass**

- [ ] **Step 5: Write failing test — embedding cache**

Test: Create a SemanticSimilarityMetric. Evaluate 3 cases with the same expected output. Assert the provider's embed() was called only once for the expected text (cached), not 3 times.

- [ ] **Step 6: Add simple HashMap cache to SemanticSimilarityMetric**

Cache key: the input text string. Cache value: the embedding vector. Use `Arc<Mutex<HashMap<String, Vec<f32>>>>`.

- [ ] **Step 7: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo test -p skg-eval -- --nocapture`

- [ ] **Step 8: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add eval/skg-eval/src/lib.rs && git commit -m "feat: add LLM judge timeout + embedding cache for SemanticSimilarityMetric"
```

---

### Task 7: Approval audit logging + dynamic policy

**Files:**
- Modify: `extras/hooks/skg-hook-approval/src/lib.rs`

**Context:** `DispatchApprovalGuard` and `InferApprovalGuard` use pure functions for policy decisions. No audit logging of denials. No way to change policy at runtime.

- [ ] **Step 1: Read the existing approval guard implementations**

Read: `extras/hooks/skg-hook-approval/src/lib.rs` (full file)

- [ ] **Step 2: Write failing test — audit observer sees denials**

```rust
// AuditingGuard wraps any guard and logs every decision
// Test: deny a dispatch, assert audit log contains the denial with operator_id and reason
```

- [ ] **Step 3: Implement AuditingDispatchGuard**

```rust
/// Wraps a `DispatchApprovalGuard` and logs every policy decision.
pub struct AuditingDispatchGuard<L: ApprovalLogger> {
    inner: DispatchApprovalGuard,
    logger: L,
}

/// Receives approval/denial events for audit logging.
pub trait ApprovalLogger: Send + Sync {
    fn log_decision(&self, operator_id: &str, decision: &PolicyDecision);
}
```

- [ ] **Step 4: Write failing test — dynamic policy swap**

```rust
// DynamicPolicyGuard uses Arc<RwLock<Box<dyn Fn(...) -> PolicyDecision>>>
// Test: create guard, dispatch (allowed), swap policy, dispatch again (denied)
```

- [ ] **Step 5: Implement DynamicPolicyGuard**

A guard that reads its policy from `Arc<RwLock<...>>`, allowing runtime swaps.

- [ ] **Step 6: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo test -p skg-hook-approval -- --nocapture`

- [ ] **Step 7: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add hooks/skg-hook-approval/src/lib.rs && git commit -m "feat: add AuditingDispatchGuard and DynamicPolicyGuard for approval middleware"
```

---

### Task 8: Structured error context

**Files:**
- Modify: `extras/hooks/skg-hook-replay/src/lib.rs` (ReplayError)
- Modify: `extras/a2a/skg-a2a/src/client/mod.rs` or `src/lib.rs` (A2aClientError)

**Context:** `ReplayError::PayloadError(String)` loses JSON parsing context. `A2aClientError` wraps generic HTTP errors without status code.

- [ ] **Step 1: Read ReplayError and A2aClientError definitions**

- [ ] **Step 2: Enhance ReplayError::PayloadError**

Change from `PayloadError(String)` to:
```rust
PayloadError {
    /// What we were trying to deserialize.
    context: String,
    /// The underlying serde error.
    source: String,
    /// The position in the recording sequence.
    position: usize,
}
```

Update all construction sites.

- [ ] **Step 3: Enhance A2aClientError with status code**

Add a variant or field that preserves the HTTP status code:
```rust
/// Remote agent returned an HTTP error.
HttpError {
    status: u16,
    body: String,
},
```

- [ ] **Step 4: Run tests, fix any breakage**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo test --workspace --all-targets`

- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add hooks/skg-hook-replay/ a2a/skg-a2a/ && git commit -m "feat: structured error context for ReplayError and A2aClientError"
```

---

### Task 9: A2aDispatcher builder + interface fallback

**Files:**
- Modify: `extras/a2a/skg-a2a/src/client/mod.rs` (or wherever A2aDispatcher lives)

- [ ] **Step 1: Read the A2aDispatcher implementation**

- [ ] **Step 2: Write failing test — builder with custom HTTP client**

```rust
// A2aDispatcher::builder(card)
//     .with_http_client(custom_client)
//     .with_interface_selector(|interfaces| interfaces.last())
//     .build()
```

- [ ] **Step 3: Implement builder**

Add `A2aDispatcherBuilder` with fluent API. Default HTTP client is `reqwest::Client::new()`. Default interface selector picks first interface.

- [ ] **Step 4: Write test — interface fallback**

Test: AgentCard with 2 interfaces, first is unreachable, selector picks second.

- [ ] **Step 5: Run tests**
- [ ] **Step 6: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add a2a/skg-a2a/ && git commit -m "feat: A2aDispatcher builder with custom HTTP client and interface selection"
```

---

### Task 10: Complete gen_ai.* semantic conventions

**Files:**
- Modify: `extras/hooks/skg-hook-otel/src/lib.rs`

**Context:** Missing from OTel gen_ai semconv: `gen_ai.request.temperature`, `gen_ai.request.top_p`, `gen_ai.response.finish_reason`. The InferRequest may carry temperature/top_p in its `extra` field or dedicated fields.

- [ ] **Step 1: Read InferRequest fields to find temperature/top_p**

Read: `skelegent/turn/skg-turn/src/infer.rs` — check InferRequest struct fields.

- [ ] **Step 2: Add missing attributes in OtelInferMiddleware**

In the pre-call section, add:
```rust
if let Some(temp) = request.temperature {
    span.set_attribute(KeyValue::new("gen_ai.request.temperature", temp));
}
if let Some(top_p) = request.top_p {
    span.set_attribute(KeyValue::new("gen_ai.request.top_p", top_p));
}
```

In the post-call section, add:
```rust
span.set_attribute(KeyValue::new("gen_ai.response.finish_reason", format!("{:?}", response.stop_reason)));
```

Note: Check actual field names on InferRequest — may be `temperature: Option<f64>` or inside `extra: serde_json::Value`.

- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add hooks/skg-hook-otel/src/lib.rs && git commit -m "feat: complete gen_ai.* OTel semconv (temperature, top_p, finish_reason)"
```

---

## Chunk 3: Track C — Integration Tests & Cross-Crate Composition

### Task 11: Cross-crate integration tests (skelegent)

**Files:**
- Create: `skelegent/tests/middleware_composition.rs`

Tests that prove the architecture composes across crate boundaries:

- [ ] **Step 1: Write test — DispatchStack + InferStack composed**

```rust
// Build a LocalOrch with DispatchStack (counting middleware)
// Register an operator that uses MiddlewareProvider with InferStack (counting middleware)
// Dispatch, collect output
// Assert both dispatch and infer middleware fired
```

- [ ] **Step 2: Write test — SecretStack + OperatorInput pipeline**

```rust
// Build a SecretRegistry with SecretStack (policy guard)
// Wire into an Environment that injects credentials
// Dispatch an operator that needs a credential
// Assert policy guard was consulted
```

- [ ] **Step 3: Write test — Recorder captures across all boundaries**

```rust
// Build a full pipeline: LocalOrch + DispatchRecorder + StoreRecorder + InferRecorder
// Run an operator that writes state, calls inference, and dispatches a child
// Verify InMemorySink has entries for all three boundaries
```

- [ ] **Step 4: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --test middleware_composition -- --nocapture`

- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add tests/middleware_composition.rs && git commit -m "test: cross-crate integration tests proving middleware composition across boundaries"
```

---

### Task 12: Cross-crate integration tests (extras)

**Files:**
- Create: `extras/tests/cross_crate_composition.rs`

- [ ] **Step 1: Write test — eval + otel don't interfere**

```rust
// Run EvalRunner with OTel middleware active
// Verify scores are correct (OTel overhead doesn't corrupt timing/scores)
```

- [ ] **Step 2: Write test — approval + replay composition**

```rust
// Build a DispatchStack with ApprovalGuard + RecorderMiddleware
// Dispatch (approved), record, replay
// Verify replay produces same output
// Dispatch (denied), verify denial is recorded but no replay entry for it
```

- [ ] **Step 3: Run tests**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo test --test cross_crate_composition -- --nocapture`

- [ ] **Step 4: Commit**

```bash
cd /Users/bear/dev/golden-neuron/extras/ && git add tests/cross_crate_composition.rs && git commit -m "test: cross-crate composition tests (eval+otel, approval+replay)"
```

---

## Chunk 4: Track D — Middleware Documentation & Examples

### Task 13: Middleware trait doc examples

**Files:**
- Modify: `skelegent/layer0/src/middleware.rs` (add examples to DispatchMiddleware, StoreMiddleware, ExecMiddleware)
- Modify: `skelegent/turn/skg-turn/src/infer_middleware.rs` (add examples to InferMiddleware, EmbedMiddleware)
- Modify: `skelegent/secret/skg-secret/src/middleware.rs` (add examples to SecretMiddleware)

For each middleware trait, add a `# Example` doc section showing a minimal implementation:

- [ ] **Step 1: Read each middleware trait**
- [ ] **Step 2: Add doc examples to DispatchMiddleware**

```rust
/// # Example
///
/// ```rust,ignore
/// use layer0::middleware::{DispatchMiddleware, DispatchNext};
///
/// struct LoggingMiddleware;
///
/// #[async_trait]
/// impl DispatchMiddleware for LoggingMiddleware {
///     async fn dispatch(
///         &self,
///         ctx: &DispatchContext,
///         input: OperatorInput,
///         next: &dyn DispatchNext,
///     ) -> Result<DispatchHandle, OrchError> {
///         tracing::info!(operator = %ctx.operator_id, "dispatching");
///         let handle = next.dispatch(ctx, input).await?;
///         tracing::info!(operator = %ctx.operator_id, "dispatched");
///         Ok(handle)
///     }
/// }
/// ```
```

- [ ] **Step 3: Repeat for StoreMiddleware, ExecMiddleware, InferMiddleware, EmbedMiddleware, SecretMiddleware**

Each example should be a simple, complete, copy-pasteable implementation showing the continuation-passing pattern.

- [ ] **Step 4: Run doc tests to verify examples compile**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --doc -p layer0`
Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --doc -p skg-turn`
Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --doc -p skg-secret`

Note: If examples use `rust,ignore`, they won't be tested. Prefer `rust,no_run` or compilable examples where possible.

- [ ] **Step 5: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add layer0/src/middleware.rs turn/skg-turn/src/infer_middleware.rs secret/skg-secret/src/middleware.rs && git commit -m "docs: add usage examples to all 6 middleware trait doc comments"
```

---

## Chunk 5: Redundancy Cleanup

### Task 14: Deduplicate test terminals + standardize naming

**Files:**
- Modify: `skelegent/layer0/src/test_utils/mod.rs` (add shared dispatch terminals)
- Modify: `skelegent/turn/skg-turn/src/test_utils.rs` (add shared infer/embed terminals)
- Modify: `skelegent/layer0/src/middleware.rs` (replace inline test terminals with imports)
- Modify: `skelegent/turn/skg-turn/src/infer_middleware.rs` (replace inline test terminals with imports)
- Modify: `skelegent/secret/skg-secret/src/middleware.rs` (replace inline test terminals with imports)

**Context:** `EchoTerminal` is defined 6+ times across test modules with identical logic but different trait impls. Extract to shared test_utils. Examples stay self-contained (Option A for tests, Option B for examples — per user's preference).

- [ ] **Step 1: Create shared test terminals in layer0::test_utils**

Add `EchoDispatchTerminal` (impl DispatchNext), `EchoStoreWriteTerminal` (impl StoreWriteNext), `EchoStoreReadTerminal` (impl StoreReadNext), `EchoExecTerminal` (impl ExecNext) to `layer0/src/test_utils/mod.rs`.

- [ ] **Step 2: Create shared test terminals in skg-turn::test_utils**

Add `EchoInferTerminal` (impl InferNext), `EchoEmbedTerminal` (impl EmbedNext) to `turn/skg-turn/src/test_utils.rs`.

- [ ] **Step 3: Replace inline test terminals in middleware test modules**

In each `#[cfg(test)]` module, replace the locally-defined `EchoTerminal` with the shared version. Do NOT touch examples.

- [ ] **Step 4: Standardize terminal naming**

Use the pattern `Echo{Boundary}Terminal` everywhere. Remove any `StubTerminal`, `CountingTerminal` that are actually just echo variants.

- [ ] **Step 5: Run all tests**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && nix develop -c cargo test --workspace --all-targets`

- [ ] **Step 6: Commit**

```bash
cd /Users/bear/dev/golden-neuron/skelegent/ && git add layer0/src/test_utils/ turn/skg-turn/src/test_utils.rs layer0/src/middleware.rs turn/skg-turn/src/infer_middleware.rs secret/skg-secret/src/middleware.rs && git commit -m "refactor: extract shared test terminals, remove inline duplicates"
```

---

## Verification (Final)

### Task 15: Full verification

- [ ] **Step 1: Run skelegent verification**

Run: `cd /Users/bear/dev/golden-neuron/skelegent/ && ./scripts/verify.sh`

- [ ] **Step 2: Run extras verification**

Run: `cd /Users/bear/dev/golden-neuron/extras/ && nix develop -c cargo test --workspace --all-targets && nix develop -c cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 3: Final test count**

Record final numbers for both repos.

- [ ] **Step 4: Update memory with new state**

Update project memory with final test counts and what shipped.

---

## Execution Order

```
Phase 0: [Task 0A || Task 0B] (foundational, different files — parallel)
Phase 1: [Task 1 || Task 2 || Task 3 || Task 4 || Task 5] (Track A, skelegent — parallel)
          [Task 6 || Task 7 || Task 8 || Task 9 || Task 10] (Track B, extras — parallel)
          Note: Phase 1 can run in parallel with Phase 0 for extras items (different repo)
Phase 2: [Task 11 || Task 12 || Task 13 || Task 14] (Tracks C+D+E — parallel)
Phase 3: Task 15 (Final verification)
```

**Critical path:** Task 0B (DynProvider move) must complete before Task 4 (InferResponse conversion) since both touch skg-turn. Task 0A is independent of everything else.
