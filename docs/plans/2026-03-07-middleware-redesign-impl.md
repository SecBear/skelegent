# Context & Middleware Redesign — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Hook/HookPoint with per-boundary continuation-based middleware, and unify AgentContext/AnnotatedMessage/ContextStrategy into a single concrete Context type.

**Architecture:** Three protocol-boundary middleware traits in layer0 (DispatchMiddleware, StoreMiddleware, ExecMiddleware) using the continuation pattern. Provider middleware stays in the turn layer (Layer 1). Middleware stacks preserve observer/guardrail/transformer ordering via builder API. A single concrete Context type replaces three overlapping types across two crates.

**Tech Stack:** Rust, async-trait, tracing 0.1, serde, layer0 protocol types.

**Design doc:** `docs/plans/2026-03-07-context-middleware-redesign.md` (approved).

**Verification:** `cd neuron/ && nix develop --command cargo test --workspace --all-targets` and `cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings`.

---

## Phase 1: New Types (Additive — Nothing Breaks)

Add all new types alongside the old ones. Everything compiles, all existing tests pass.

### Task 1.1: Message and Role types in layer0

**Files:**
- Modify: `neuron/layer0/src/context.rs`
- Modify: `neuron/layer0/src/lib.rs`

**Step 1: Write the failing test**

Add to `neuron/layer0/src/context.rs` tests module:

```rust
#[test]
fn message_construction_and_role_variants() {
    let msg = Message {
        role: Role::User,
        content: Content::text("hello"),
        meta: MessageMeta::default(),
    };
    assert!(matches!(msg.role, Role::User));

    let tool_msg = Message {
        role: Role::Tool { name: "shell".into(), call_id: "tc_1".into() },
        content: Content::text("output"),
        meta: MessageMeta::default(),
    };
    assert!(matches!(tool_msg.role, Role::Tool { .. }));
}

#[test]
fn message_serde_roundtrip() {
    let msg = Message {
        role: Role::Assistant,
        content: Content::text("hi"),
        meta: MessageMeta::default(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let rt: Message = serde_json::from_str(&json).unwrap();
    assert!(matches!(rt.role, Role::Assistant));
}
```

**Step 2: Run test to verify it fails**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 message_construction -- --nocapture`
Expected: FAIL — `Message` and `Role` not defined.

**Step 3: Write minimal implementation**

Add to `neuron/layer0/src/context.rs`, above the existing `AgentContext` code:

```rust
/// Role of a message in the context window.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// System instruction.
    System,
    /// Human message.
    User,
    /// Model response.
    Assistant,
    /// Tool/sub-operator result.
    Tool {
        /// Name of the tool/operator.
        name: String,
        /// Provider-specific call ID for correlation.
        call_id: String,
    },
}

/// A message in an agent's context window.
///
/// Concrete type — not generic. Every message has a role, content,
/// and per-message metadata (compaction policy, salience, source).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Who produced this message.
    pub role: Role,
    /// The message payload.
    pub content: Content,
    /// Per-message annotation (compaction policy, salience, source, version).
    pub meta: MessageMeta,
}

impl Message {
    /// Create a new message with default metadata.
    pub fn new(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta::default(),
        }
    }

    /// Create a message with `CompactionPolicy::Pinned`.
    pub fn pinned(role: Role, content: Content) -> Self {
        Self {
            role,
            content,
            meta: MessageMeta {
                policy: CompactionPolicy::Pinned,
                ..Default::default()
            },
        }
    }
}
```

Add `use crate::content::Content;` to the imports if not already present.

Add to `neuron/layer0/src/lib.rs` re-exports:

```rust
pub use context::{Message, Role};
```

**Step 4: Run test to verify it passes**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 -- message_construction message_serde`
Expected: PASS

**Step 5: Commit**

```bash
git add layer0/src/context.rs layer0/src/lib.rs
git commit -m "feat(layer0): add concrete Message and Role types"
```

---

### Task 1.2: Middleware traits and Next types

**Files:**
- Create: `neuron/layer0/src/middleware.rs`
- Modify: `neuron/layer0/src/lib.rs`

**Step 1: Write the failing test**

Create `neuron/layer0/src/middleware.rs` with tests at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_next_calls_through() {
        // A DispatchMiddleware that adds a tag, then calls next
        struct TagMiddleware;

        #[async_trait]
        impl DispatchMiddleware for TagMiddleware {
            async fn dispatch(
                &self,
                operator: &OperatorId,
                mut input: OperatorInput,
                next: &dyn DispatchNext,
            ) -> Result<OperatorOutput, OrchError> {
                input.metadata = serde_json::json!({"tagged": true});
                next.dispatch(operator, input).await
            }
        }

        // Verify the middleware can be constructed and the trait is object-safe
        let _mw: Box<dyn DispatchMiddleware> = Box::new(TagMiddleware);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 dispatch_next_calls_through`
Expected: FAIL — module doesn't exist.

**Step 3: Write minimal implementation**

Write `neuron/layer0/src/middleware.rs`:

```rust
//! Per-boundary middleware traits using the continuation pattern.
//!
//! Three middleware traits — one per layer0 protocol boundary:
//! - [`DispatchMiddleware`] wraps [`Orchestrator::dispatch`]
//! - [`StoreMiddleware`] wraps [`StateStore`] read/write
//! - [`ExecMiddleware`] wraps [`Environment::run`]
//!
//! Provider middleware is NOT here — it lives in the turn layer (Layer 1)
//! because Provider is RPITIT, not object-safe.

use crate::effect::Scope;
use crate::environment::EnvironmentSpec;
use crate::error::{EnvError, OrchError, StateError};
use crate::id::OperatorId;
use crate::operator::{OperatorInput, OperatorOutput};
use async_trait::async_trait;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DISPATCH MIDDLEWARE (wraps Orchestrator::dispatch)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in a dispatch middleware chain.
///
/// Call `dispatch()` to pass control to the inner layer.
/// Don't call it to short-circuit (guardrail halt).
#[async_trait]
pub trait DispatchNext: Send + Sync {
    /// Forward the dispatch to the next layer.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError>;
}

/// Middleware wrapping `Orchestrator::dispatch`.
///
/// Code before `next.dispatch()` = pre-processing (input mutation, logging).
/// Code after `next.dispatch()` = post-processing (output mutation, metrics).
/// Not calling `next.dispatch()` = short-circuit (guardrail halt, cached response).
#[async_trait]
pub trait DispatchMiddleware: Send + Sync {
    /// Intercept a dispatch call.
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError>;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STORE MIDDLEWARE (wraps StateStore read/write)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in a store-write middleware chain.
#[async_trait]
pub trait StoreWriteNext: Send + Sync {
    /// Forward the write to the next layer.
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError>;
}

/// The next layer in a store-read middleware chain.
#[async_trait]
pub trait StoreReadNext: Send + Sync {
    /// Forward the read to the next layer.
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError>;
}

/// Middleware wrapping `StateStore` read and write operations.
///
/// Use for: encryption-at-rest, audit trails, caching, access control.
#[async_trait]
pub trait StoreMiddleware: Send + Sync {
    /// Intercept a state write.
    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
        next: &dyn StoreWriteNext,
    ) -> Result<(), StateError>;

    /// Intercept a state read. Default: pass through.
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
        next: &dyn StoreReadNext,
    ) -> Result<Option<serde_json::Value>, StateError> {
        next.read(scope, key).await
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EXEC MIDDLEWARE (wraps Environment::run)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The next layer in an exec middleware chain.
#[async_trait]
pub trait ExecNext: Send + Sync {
    /// Forward the execution to the next layer.
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
    ) -> Result<OperatorOutput, EnvError>;
}

/// Middleware wrapping `Environment::run`.
///
/// Use for: credential injection, isolation upgrades, resource enforcement.
#[async_trait]
pub trait ExecMiddleware: Send + Sync {
    /// Intercept an environment execution.
    async fn run(
        &self,
        input: OperatorInput,
        spec: &EnvironmentSpec,
        next: &dyn ExecNext,
    ) -> Result<OperatorOutput, EnvError>;
}
```

Add `pub mod middleware;` to `neuron/layer0/src/lib.rs`.

Add re-exports:

```rust
pub use middleware::{
    DispatchMiddleware, DispatchNext,
    StoreMiddleware, StoreReadNext, StoreWriteNext,
    ExecMiddleware, ExecNext,
};
```

**Step 4: Run test to verify it passes**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 dispatch_next_calls_through`
Expected: PASS

**Step 5: Commit**

```bash
git add layer0/src/middleware.rs layer0/src/lib.rs
git commit -m "feat(layer0): add per-boundary middleware traits"
```

---

### Task 1.3: DispatchStack builder

**Files:**
- Create: `neuron/layer0/src/middleware/dispatch_stack.rs`
- Modify: `neuron/layer0/src/middleware.rs` (make it `mod` directory or include)

Note: If the middleware module is a single file, refactor it into a directory with `mod.rs` + `dispatch_stack.rs`. Alternatively, keep it in one file if the stack builder is small enough.

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn dispatch_stack_observer_always_runs() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicU32::new(0));

    struct CountObserver(Arc<AtomicU32>);

    #[async_trait]
    impl DispatchMiddleware for CountObserver {
        async fn dispatch(
            &self,
            operator: &OperatorId,
            input: OperatorInput,
            next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            next.dispatch(operator, input).await
        }
    }

    struct HaltGuard;

    #[async_trait]
    impl DispatchMiddleware for HaltGuard {
        async fn dispatch(
            &self,
            _operator: &OperatorId,
            _input: OperatorInput,
            _next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            // Short-circuit: don't call next
            Err(OrchError::DispatchFailed {
                agent: "halted".into(),
                message: "budget exceeded".into(),
            })
        }
    }

    let stack = DispatchStack::builder()
        .observe(Arc::new(CountObserver(counter.clone())))
        .guard(Arc::new(HaltGuard))
        .build();

    // Even though the guard halts, the observer must have run
    let input = OperatorInput::new(Content::text("test"), TriggerType::User);
    let result = stack.dispatch(&OperatorId::from("a"), input).await;
    assert!(result.is_err());
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dispatch_stack_transform_then_guard() {
    struct Uppercaser;

    #[async_trait]
    impl DispatchMiddleware for Uppercaser {
        async fn dispatch(
            &self,
            operator: &OperatorId,
            mut input: OperatorInput,
            next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            // Transform: uppercase the metadata tag
            input.metadata = serde_json::json!({"transformed": true});
            next.dispatch(operator, input).await
        }
    }

    struct PassGuard;

    #[async_trait]
    impl DispatchMiddleware for PassGuard {
        async fn dispatch(
            &self,
            operator: &OperatorId,
            input: OperatorInput,
            next: &dyn DispatchNext,
        ) -> Result<OperatorOutput, OrchError> {
            // Guard passes — call next
            next.dispatch(operator, input).await
        }
    }

    // Inner: the actual orchestrator (as a DispatchNext)
    struct EchoNext;

    #[async_trait]
    impl DispatchNext for EchoNext {
        async fn dispatch(
            &self,
            _operator: &OperatorId,
            input: OperatorInput,
        ) -> Result<OperatorOutput, OrchError> {
            Ok(OperatorOutput::new(input.message, ExitReason::Complete))
        }
    }

    let stack = DispatchStack::builder()
        .transform(Arc::new(Uppercaser))
        .guard(Arc::new(PassGuard))
        .build();

    let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
    let result = stack.dispatch_with(&OperatorId::from("a"), input, &EchoNext).await;
    assert!(result.is_ok());
}
```

**Step 2: Run test to verify it fails**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 dispatch_stack`
Expected: FAIL — `DispatchStack` not defined.

**Step 3: Write minimal implementation**

The `DispatchStack` builder wraps middleware in the correct order:
- Observers (outermost) — always run, always call next
- Transformers — mutate, always call next
- Guards (innermost) — may short-circuit

```rust
/// A composed middleware stack for dispatch operations.
///
/// Built via [`DispatchStack::builder()`]. Stacking order:
/// Observers (outermost) → Transformers → Guards (innermost).
///
/// Observers always run (even if a guard halts) because they're
/// the outermost layer. Guards see transformed input because
/// transformers are between observers and guards.
pub struct DispatchStack {
    /// Middleware layers in call order (outermost first).
    /// Built from: observers, then transformers, then guards.
    layers: Vec<Arc<dyn DispatchMiddleware>>,
}

pub struct DispatchStackBuilder {
    observers: Vec<Arc<dyn DispatchMiddleware>>,
    transformers: Vec<Arc<dyn DispatchMiddleware>>,
    guards: Vec<Arc<dyn DispatchMiddleware>>,
}

impl DispatchStack {
    /// Start building a dispatch middleware stack.
    pub fn builder() -> DispatchStackBuilder {
        DispatchStackBuilder {
            observers: Vec::new(),
            transformers: Vec::new(),
            guards: Vec::new(),
        }
    }

    /// Dispatch through the middleware chain, ending at `terminal`.
    pub async fn dispatch_with(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        terminal: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        if self.layers.is_empty() {
            return terminal.dispatch(operator, input).await;
        }
        let chain = MiddlewareChain {
            layers: &self.layers,
            index: 0,
            terminal,
        };
        chain.dispatch(operator, input).await
    }
}

impl DispatchStackBuilder {
    /// Add an observer middleware (outermost — always runs, always calls next).
    pub fn observe(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.observers.push(mw);
        self
    }

    /// Add a transformer middleware (mutates input/output, always calls next).
    pub fn transform(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.transformers.push(mw);
        self
    }

    /// Add a guard middleware (innermost — may short-circuit by not calling next).
    pub fn guard(mut self, mw: Arc<dyn DispatchMiddleware>) -> Self {
        self.guards.push(mw);
        self
    }

    /// Build the stack. Order: observers → transformers → guards.
    pub fn build(self) -> DispatchStack {
        let mut layers = Vec::new();
        layers.extend(self.observers);
        layers.extend(self.transformers);
        layers.extend(self.guards);
        DispatchStack { layers }
    }
}

/// Internal chain that threads dispatch through middleware layers.
struct MiddlewareChain<'a> {
    layers: &'a [Arc<dyn DispatchMiddleware>],
    index: usize,
    terminal: &'a dyn DispatchNext,
}

#[async_trait]
impl DispatchNext for MiddlewareChain<'_> {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
    ) -> Result<OperatorOutput, OrchError> {
        if self.index >= self.layers.len() {
            return self.terminal.dispatch(operator, input).await;
        }
        let next = MiddlewareChain {
            layers: self.layers,
            index: self.index + 1,
            terminal: self.terminal,
        };
        self.layers[self.index].dispatch(operator, input, &next).await
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 dispatch_stack`
Expected: PASS

**Step 5: Commit**

```bash
git add layer0/
git commit -m "feat(layer0): add DispatchStack builder with observer/transform/guard ordering"
```

---

### Task 1.4: StoreStack and ExecStack builders

**Files:**
- Modify: `neuron/layer0/src/middleware.rs` (or directory)

Follow the same pattern as Task 1.3 for `StoreStack` and `ExecStack`. Each gets:
- A `*Stack` struct with `layers: Vec<Arc<dyn *Middleware>>`
- A `*StackBuilder` with `observe/transform/guard` methods
- A `*_with()` method that threads through a terminal `*Next`
- Internal `*Chain` struct implementing `*Next`

Tests:
- `store_stack_audit_write` — observer logs all writes, guard may reject
- `exec_stack_credential_inject` — transformer adds credentials before `next.run()`

**Step 5: Commit**

```bash
git add layer0/
git commit -m "feat(layer0): add StoreStack and ExecStack middleware builders"
```

---

### Task 1.5: Full workspace verification (Phase 1)

Run:

```bash
cd neuron/ && nix develop --command cargo test --workspace --all-targets
cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings
```

Expected: All existing tests pass. All new tests pass. Zero clippy warnings. Nothing was removed — only added.

**Commit:**

```bash
git commit --allow-empty -m "chore: Phase 1 verification — all additive, nothing removed"
```

---

## Phase 2: Context Migration

Replace the generic `AgentContext<M>` with the concrete `Context` type and migrate `AnnotatedMessage`/`ContextStrategy` consumers.

### Task 2.1: Replace AgentContext<M> with Context

**Files:**
- Modify: `neuron/layer0/src/context.rs`
- Modify: `neuron/layer0/src/lib.rs`

`AgentContext<M>` is defined but has ZERO consumers in skelegent/ or extras/. Replace it in-place with `Context` that uses the concrete `Message` type from Task 1.1.

**Step 1: Write the failing test**

```rust
#[test]
fn context_push_and_read() {
    let mut ctx = Context::new(OperatorId::from("agent-1"));
    ctx.push(Message::new(Role::User, Content::text("hello"))).unwrap();
    ctx.push(Message::new(Role::Assistant, Content::text("hi"))).unwrap();
    assert_eq!(ctx.len(), 2);
    assert!(matches!(ctx.messages()[0].role, Role::User));
    assert!(matches!(ctx.messages()[1].role, Role::Assistant));
}

#[test]
fn context_compact_truncate() {
    let mut ctx = Context::new(OperatorId::from("a"));
    for i in 0..10 {
        ctx.push(Message::new(Role::User, Content::text(format!("msg {i}")))).unwrap();
    }
    let removed = ctx.compact_truncate(3);
    assert_eq!(removed.len(), 7);
    assert_eq!(ctx.len(), 3);
}

#[test]
fn context_compact_by_policy_preserves_pinned() {
    let mut ctx = Context::new(OperatorId::from("a"));
    ctx.push(Message::pinned(Role::System, Content::text("you are helpful"))).unwrap();
    for i in 0..5 {
        ctx.push(Message::new(Role::User, Content::text(format!("msg {i}")))).unwrap();
    }
    let removed = ctx.compact_by_policy();
    // Pinned message survives, Normal messages removed
    assert_eq!(ctx.len(), 1);
    assert!(matches!(ctx.messages()[0].role, Role::System));
    assert_eq!(removed.len(), 5);
}

#[test]
fn context_compact_with_closure() {
    let mut ctx = Context::new(OperatorId::from("a"));
    for i in 0..6 {
        ctx.push(Message::new(Role::User, Content::text(format!("msg {i}")))).unwrap();
    }
    // Custom: keep only even-indexed messages
    let removed = ctx.compact_with(|msgs| {
        msgs.iter().enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, m)| m.clone())
            .collect()
    });
    assert_eq!(ctx.len(), 3);
    assert_eq!(removed.len(), 3);
}
```

**Step 2: Run test to verify it fails**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 context_push_and_read`
Expected: FAIL — `Context` not defined.

**Step 3: Implement Context**

Replace the existing `AgentContext<M>` struct and all its `impl` blocks with a new `Context` struct that uses `Message` directly. Keep `ContextWatcher`, `WatcherVerdict`, `ContextSnapshot`, `ContextError`, `Position`, `MessageMeta`, `ContextMessage<M>` temporarily (mark deprecated).

The `Context` type:
- Same watcher-guarded mutation API as `AgentContext<M>` but using `Message`
- Adds `compact_truncate`, `compact_by_policy`, `compact_with` methods
- System prompt becomes a `Message` with `Role::System`, not a separate field
- `snapshot()` returns `ContextSnapshot` with concrete types

Update `lib.rs` re-exports: add `Context`, keep old types with `#[deprecated]`.

**Step 4: Run tests**

Run: `cd neuron/ && nix develop --command cargo test -p layer0 context_`
Expected: All new context tests PASS. Old `AgentContext` tests may be removed or adapted.

**Step 5: Commit**

```bash
git add layer0/
git commit -m "feat(layer0): add concrete Context type replacing AgentContext<M>"
```

---

### Task 2.2: Migrate AnnotatedMessage → Message

**Files:**
- Modify: `neuron/turn/skg-turn/src/context.rs`
- Modify: `neuron/turn/skg-turn/src/lib.rs`
- Modify: `neuron/turn/skg-context/src/lib.rs` (SlidingWindow, TieredStrategy)
- Modify: `neuron/turn/skg-context/src/context_assembly.rs` (ContextAssembler)

`AnnotatedMessage` has these fields:
- `message: ProviderMessage` (role + content)
- `policy: CompactionPolicy`
- `source: Option<String>`
- `salience: Option<f64>`
- `session_label: Option<String>`

These map 1:1 to `Message`:
- `message` → `Message.role` + `Message.content`
- `policy` → `Message.meta.policy`
- `source` → `Message.meta.source`
- `salience` → `Message.meta.salience`

**Step 1:** Update `ContextStrategy` trait to use `Vec<Message>` instead of `Vec<AnnotatedMessage>`. Keep `AnnotatedMessage` temporarily with a `From<AnnotatedMessage> for Message` conversion.

**Step 2:** Update `SlidingWindow` and `TieredStrategy` in `skg-context` to work with `Message`.

**Step 3:** Update `ContextAssembler` to produce `Vec<Message>`.

**Step 4:** Run tests. Fix compile errors.

**Step 5: Commit**

```bash
git add turn/ 
git commit -m "refactor(turn): migrate AnnotatedMessage to layer0::Message"
```

---

### Task 2.3: Migrate ContextStrategy → Context methods

**Files:**
- Modify: `neuron/turn/skg-turn/src/context.rs`
- Modify: `neuron/turn/skg-context/src/lib.rs`
- Modify: `neuron/op/skg-op-react/src/lib.rs`

The `ContextStrategy` trait has:
- `token_estimate(&self, messages: &[AnnotatedMessage]) -> usize`
- `should_compact(&self, estimate: usize) -> bool`
- `compact(&self, messages: Vec<AnnotatedMessage>) -> Result<Vec<AnnotatedMessage>, CompactionError>`

These become:
- `Context::estimated_tokens()` — already on Context
- Compaction trigger logic — moves into ReactOperator (operator-local)
- `Context::compact_with(closure)` — the strategy provides the closure

`SlidingWindow` becomes a function:

```rust
pub fn sliding_window_compactor(keep: usize) -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs| {
        let pinned: Vec<_> = msgs.iter()
            .filter(|m| m.meta.policy == CompactionPolicy::Pinned)
            .cloned()
            .collect();
        let unpinned: Vec<_> = msgs.iter()
            .filter(|m| m.meta.policy != CompactionPolicy::Pinned)
            .collect();
        let keep_from = unpinned.len().saturating_sub(keep);
        let mut result = pinned;
        result.extend(unpinned[keep_from..].iter().cloned());
        result
    }
}
```

ReactOperator changes from `self.context_strategy.compact(messages)` to `self.context.compact_with(sliding_window_compactor(keep))`.

**Step 1:** Convert SlidingWindow to a function returning a closure.
**Step 2:** Update ReactOperator to use `Context` with `compact_with`.
**Step 3:** Delete `ContextStrategy` trait.
**Step 4:** Run full workspace tests.

**Step 5: Commit**

```bash
git add turn/ op/
git commit -m "refactor: replace ContextStrategy trait with Context::compact_with + closures"
```

---

### Task 2.4: Delete deprecated types

**Files:**
- Modify: `neuron/layer0/src/context.rs` — remove `AgentContext<M>`, `ContextMessage<M>`
- Modify: `neuron/turn/skg-turn/src/context.rs` — remove `AnnotatedMessage`, `ContextStrategy`
- Modify: `neuron/layer0/src/lib.rs` — remove deprecated re-exports

**Step 1:** Delete old types. Fix any remaining compile errors.
**Step 2:** Run full workspace tests.
**Step 3: Commit**

```bash
git add layer0/ turn/
git commit -m "cleanup: remove AgentContext<M>, AnnotatedMessage, ContextStrategy"
```

---

### Task 2.5: Full workspace verification (Phase 2)

```bash
cd neuron/ && nix develop --command cargo test --workspace --all-targets
cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings
```

Expected: All tests pass. No clippy warnings. Context is now a single concrete type.

---

## Phase 3: Middleware Migration

Convert existing Hook consumers to use the middleware stack.

### Task 3.1: Migrate LocalOrchestrator to DispatchStack

**Files:**
- Modify: `neuron/orch/skg-orch-local/src/lib.rs`
- Modify: `neuron/orch/skg-orch-local/tests/orch.rs`

LocalOrch currently has an `Option<HookRegistry>` and fires `PreDispatch`/`PostDispatch` hooks. Replace with `DispatchStack`:

```rust
// Before:
pub struct LocalOrch {
    // ...
    hooks: Option<HookRegistry>,
}

// After:
pub struct LocalOrch {
    // ...
    middleware: Option<DispatchStack>,
}
```

The `dispatch()` method changes from manually creating HookContext + calling `hooks.dispatch()` to using `self.middleware.dispatch_with(agent, input, &inner)`.

The `CountHook` test in `tests/orch.rs` becomes a `CountMiddleware` implementing `DispatchMiddleware`.

**Commit:**

```bash
git commit -m "refactor(orch-local): replace HookRegistry with DispatchStack"
```

---

### Task 3.2: Migrate effect handlers to StoreMiddleware

**Files:**
- Modify: `neuron/effects/skg-effects-local/src/lib.rs`
- Modify: `neuron/effects/skg-effects-local/tests/hooks.rs`
- Modify: `neuron/orch/skg-orch-kit/src/runner.rs`

These currently fire `PreMemoryWrite` hooks before state writes. Replace with `StoreStack` wrapping the `StateStore`.

The test hooks (`HaltHook`, `RecordingObserver`, `ModifyTransformer`, `LifetimeGuardrail`) become `StoreMiddleware` implementations.

**Commit:**

```bash
git commit -m "refactor(effects): replace PreMemoryWrite hooks with StoreMiddleware"
```

---

### Task 3.3: Migrate security hooks to middleware

**Files:**
- Modify: `neuron/hooks/skg-hook-security/src/lib.rs`

`RedactionHook` (PostSubDispatch) and `ExfilGuardHook` (PreSubDispatch) are ReactOperator-internal concerns. They move to `skg-op-react` as operator-local middleware or become `DispatchMiddleware` implementations.

Decision: Since these operate on sub-dispatch input/output, they're most naturally `DispatchMiddleware`:

```rust
// RedactionHook → RedactionMiddleware
pub struct RedactionMiddleware { /* ... */ }

#[async_trait]
impl DispatchMiddleware for RedactionMiddleware {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        let mut output = next.dispatch(operator, input).await?;
        // Redact secrets from output
        self.redact(&mut output);
        Ok(output)
    }
}
```

**Commit:**

```bash
git commit -m "refactor(security): migrate RedactionHook/ExfilGuardHook to DispatchMiddleware"
```

---

### Task 3.4: Migrate ReactOperator internal hook points

**Files:**
- Modify: `neuron/op/skg-op-react/src/lib.rs`
- (Optional) Create: `neuron/op/skg-op-react/src/intercept.rs`

ReactOperator fires hooks at: PreInference, PostInference, PreSubDispatch, PostSubDispatch, ExitCheck, SubDispatchUpdate, PreSteeringInject, PostSteeringSkip, PreCompaction, PostCompaction.

These are operator-local. Replace with:
1. **Sub-dispatch hooks** → Use `DispatchStack` for sub-operator dispatch (the operator already dispatches via `Arc<dyn Orchestrator>`)
2. **Inference hooks** → tracing spans (observation only) or operator-local callback
3. **Steering hooks** → operator-local extension point
4. **Compaction hooks** → `ContextWatcher` on `Context` (already exists)
5. **Exit check** → operator-local logic, not a hook

Define a `ReactInterceptor` trait local to `skg-op-react` for operator-specific extension points that don't belong in layer0:

```rust
// In skg-op-react, NOT layer0
#[async_trait]
pub trait ReactInterceptor: Send + Sync {
    async fn pre_inference(&self, _messages: &[Message]) -> ReactAction { ReactAction::Continue }
    async fn post_inference(&self, _response: &Content) -> ReactAction { ReactAction::Continue }
    async fn exit_check(&self, _output: &OperatorOutput) -> ReactAction { ReactAction::Continue }
}
```

**Commit:**

```bash
git commit -m "refactor(react): extract operator-local ReactInterceptor, remove HookPoint dependency"
```

---

### Task 3.5: Full workspace verification (Phase 3)

```bash
cd neuron/ && nix develop --command cargo test --workspace --all-targets
cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings
```

Expected: All tests pass. No clippy warnings.

---

## Phase 4: Delete Old Types

### Task 4.1: Remove Hook system from layer0

**Files:**
- Modify: `neuron/layer0/src/hook.rs` — delete everything (entire file or gut it)
- Modify: `neuron/layer0/src/lib.rs` — remove `pub mod hook` and all Hook re-exports
- Delete: `neuron/hooks/skg-hooks/src/lib.rs` (HookRegistry)
- Delete: `neuron/layer0/src/test_utils/logging_hook.rs`

Before deleting, verify with `cargo check --workspace` that no crate imports Hook types.

**Commit:**

```bash
git commit -m "cleanup: remove Hook trait, HookPoint, HookRegistry from layer0"
```

---

### Task 4.2: Remove ObservableEvent and EventSource

**Files:**
- Modify: `neuron/layer0/src/lifecycle.rs` — remove `ObservableEvent`, `EventSource`
- Modify: `neuron/layer0/src/lib.rs` — remove re-exports

These are replaced by tracing spans and middleware. Verify no consumers remain.

**Commit:**

```bash
git commit -m "cleanup: remove ObservableEvent and EventSource (replaced by tracing)"
```

---

### Task 4.3: Clean up skg-hooks crate

**Files:**
- Modify: `neuron/hooks/skg-hooks/Cargo.toml` — if crate is now empty, remove it
- Modify: `neuron/Cargo.toml` workspace members — remove if crate deleted

If `skg-hook-security` was migrated to middleware in Task 3.3, it may also be removable or renamed.

Also update `neuron/neuron/Cargo.toml` facade crate:
- Remove `hooks` feature gate from `default`
- Remove `hooks = ["core", "dep:skg-hooks"]` feature
- Change `op-react` and `op-single-shot` features to no longer depend on `hooks`
  (`skg-op-single-shot` has zero hook usage — the dependency was a false gate)
- Remove `Hook`, `HookAction`, `HookContext`, `HookPoint` from the prelude re-exports
- Add `DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`, `DispatchStack` to prelude

**Commit:**

```bash
git commit -m "cleanup: remove skg-hooks crate (HookRegistry replaced by DispatchStack)"
```

---

## Phase 5: Documentation & Final Verification

### Task 5.1: Update ARCHITECTURE.md

**Files:**
- Modify: `neuron/ARCHITECTURE.md`

Update the "Three-Primitive Operator Composition" section. The old text about hooks/steering/planner being structurally different and must not be unified needs to be replaced with the middleware model.

Update the Lifecycle section to reference `DispatchStack`, `StoreStack`, `ExecStack`.

Update the layer0 protocol table: remove ⑤ Hooks and ⑥ Lifecycle rows. Add middleware documentation.

**Commit:**

```bash
git commit -m "docs: update ARCHITECTURE.md for middleware redesign"
```

---

### Task 5.2: Update book documentation

**Files:**
- Modify: `neuron/docs/book/src/guides/hooks.md` — rewrite for middleware
- Modify: `neuron/docs/book/src/guides/custom-operator.md` — update examples
- Modify: `neuron/docs/book/src/architecture/protocol-traits.md` — update
- Modify: `neuron/docs/book/src/SUMMARY.md` — update navigation

Replace Hook examples with middleware examples. The `DenyToolHook` becomes a `DispatchMiddleware` guard. The `StripSecretTransformer` becomes a `DispatchMiddleware` transformer.

**Commit:**

```bash
git commit -m "docs: update book for middleware redesign"
```

---

### Task 5.3: Final full verification

```bash
cd neuron/ && nix develop --command cargo test --workspace --all-targets
cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings
cd extras/ && nix develop --command cargo test --workspace --all-targets
cd extras/ && nix develop --command cargo clippy --workspace -- -D warnings
```

Expected: All tests pass in both workspaces. Zero clippy warnings.

Verify no external decision vocabulary in source:

```bash
cd neuron/ && grep -rn '\b[DLC][1-5][A-E]\?\b' --include='*.rs' --include='*.md' | grep -v 'target/' | grep -v 'plans/'
```

Expected: No matches outside plans/ directory.

**Commit:**

```bash
git commit --allow-empty -m "chore: final verification — middleware redesign complete"
```

---

## Summary

| Phase | Tasks | What Changes |
|-------|-------|-------------|
| 1 | 1.1–1.5 | Add Message, Role, middleware traits, stack builders (additive) |
| R | R.1–R.6 | Reshape reusable code onto new types (depends on Phase 1) |
| 2 | 2.1–2.5 | Replace AgentContext/AnnotatedMessage/ContextStrategy with Context |
| 3 | 3.1–3.5 | Migrate Hook consumers to middleware |
| 4 | 4.1–4.3 | Delete Hook, HookPoint, HookRegistry, ObservableEvent |
| 5 | 5.1–5.3 | Update docs, final verification |

Total: ~24 tasks across 6 phases. Each phase ends with full workspace verification.

---

## Phase R: Reshape Reusable Code (depends on Phase 1)

After Phase 1 lands `Message`, `Role`, `Content`, and the middleware traits, reshape
all reusable algorithms onto the new types. Each task is one agent, one crate. The old
code stays alive until Phase 2–4 deletes it — these tasks produce NEW code that will
replace the old code when consumers are migrated.

### Conversion bridge (created in Task 1.1, used by all R tasks)

Phase 1 adds these conversions to `layer0/src/context.rs`:

```rust
// ProviderMessage (turn-layer) → Message (layer0)
// This lives in skg-turn because it knows both types.
// layer0::Message does NOT depend on ProviderMessage.

impl From<ProviderMessage> for Message {
    fn from(pm: ProviderMessage) -> Self {
        let role = match pm.role {
            turn::Role::System => Role::System,
            turn::Role::User => Role::User,
            turn::Role::Assistant => Role::Assistant,
        };
        let content = Content::Blocks(
            pm.content.into_iter().map(|cp| match cp {
                ContentPart::Text { text } => ContentBlock::Text { text },
                ContentPart::ToolUse { id, name, input } => ContentBlock::ToolUse { id, name, input },
                ContentPart::ToolResult { tool_use_id, content, is_error } => ContentBlock::ToolResult { tool_use_id, content, is_error },
                ContentPart::Image { source, media_type } => {
                    let src = match source {
                        turn::ImageSource::Base64 { data } => layer0::ImageSource::Base64 { data },
                        turn::ImageSource::Url { url } => layer0::ImageSource::Url { url },
                    };
                    ContentBlock::Image { source: src, media_type }
                }
            }).collect()
        );
        Message::new(role, content)
    }
}

// AnnotatedMessage → Message with metadata
impl From<AnnotatedMessage> for Message {
    fn from(am: AnnotatedMessage) -> Self {
        let mut msg = Message::from(am.message);
        if let Some(policy) = am.policy {
            msg.meta.policy = policy;
        }
        msg.meta.source = am.source;
        msg.meta.salience = am.salience;
        msg
    }
}
```

Note: This `From` impl lives in `skg-turn/src/convert.rs` (or `context.rs`),
NOT in layer0 — because layer0 must not depend on skg-turn. The turn crate
sees both types and provides the bridge.

### Token estimation helper (shared by all compaction tasks)

Every compaction algorithm needs token estimation. Extract the shared logic:

```rust
// In layer0/src/context.rs (on Message)
impl Message {
    /// Rough token estimate: chars/4 for text, 1000 for images, +4 overhead per message.
    pub fn estimated_tokens(&self) -> usize {
        let content_tokens = match &self.content {
            Content::Text(s) => s.len() / 4,
            Content::Blocks(blocks) => blocks.iter().map(|b| match b {
                ContentBlock::Text { text } => text.len() / 4,
                ContentBlock::ToolUse { input, .. } => input.to_string().len() / 4,
                ContentBlock::ToolResult { content, .. } => content.len() / 4,
                ContentBlock::Image { .. } => 1000,
                ContentBlock::Custom { data, .. } => data.to_string().len() / 4,
            }).sum(),
        };
        content_tokens + 4 // per-message overhead
    }

    /// Extract all text content for similarity computation.
    pub fn text_content(&self) -> String {
        match &self.content {
            Content::Text(s) => s.clone(),
            Content::Blocks(blocks) => blocks.iter().filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            }).collect::<Vec<_>>().join(" "),
        }
    }
}
```

---

### Task R.1: SlidingWindow → `sliding_window_compactor()` closure

**Crate:** `skg-context`
**Files:** Modify `neuron/turn/skg-context/src/lib.rs`
**Depends on:** Task 1.1 (Message type exists)
**One agent. One file.**

**Current code** (in `skg-context/src/lib.rs`, lines 25–129):

```rust
// SlidingWindow implements ContextStrategy for AnnotatedMessage.
// Algorithm: partition pinned/normal → keep first message + recent by token budget.
// Token estimation: sum(content_chars / chars_per_token + 4_overhead).
```

**Target code:**

```rust
/// Sliding window compactor: drops oldest messages, keeps first + recent by token budget.
///
/// The returned closure is passed to `Context::compact_with()`.
/// Pinned messages always survive. The first non-pinned message is preserved
/// (typically the initial user message). Remaining budget is filled from the end.
pub fn sliding_window_compactor() -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        let (pinned, normal): (Vec<_>, Vec<_>) = msgs
            .iter()
            .partition(|m| matches!(m.meta.policy, CompactionPolicy::Pinned));

        let compacted_normal = if normal.len() <= 2 {
            normal.into_iter().cloned().collect()
        } else {
            let first = normal[0].clone();
            let rest = &normal[1..];

            let total_tokens: usize = std::iter::once(&first)
                .chain(rest.iter().copied())
                .map(|m| m.estimated_tokens())
                .sum();
            let target = total_tokens / 2;

            let mut kept = Vec::new();
            let mut current_tokens = first.estimated_tokens();

            for msg in rest.iter().rev() {
                let msg_tokens = msg.estimated_tokens();
                if current_tokens + msg_tokens > target && !kept.is_empty() {
                    break;
                }
                kept.push((*msg).clone());
                current_tokens += msg_tokens;
            }

            kept.reverse();
            let mut result = vec![first];
            result.extend(kept);
            result
        };

        let mut result: Vec<Message> = pinned.into_iter().cloned().collect();
        result.extend(compacted_normal);
        result
    }
}
```

**Tests to rewrite:**
- `sliding_window_estimates_tokens` → `Message::estimated_tokens()` unit test (moves to layer0)
- `sliding_window_should_compact` → removed (trigger logic moves to Context/ReactOperator)
- `sliding_window_compact_*` → rewritten using `Message` constructors
- `sliding_window_pinned_messages_survive_compaction` → same logic, new types

**Keep the old `impl ContextStrategy` alive** — it will be deleted in Phase 2 Task 2.4.
The new function is additive.

---

### Task R.2: SaliencePackingStrategy → `salience_packing_compactor()` closure

**Crate:** `skg-context`
**Files:** Modify `neuron/turn/skg-context/src/salience_packing.rs`
**Depends on:** Task 1.1 (Message type exists)
**One agent. One file.**

**Current code** (salience_packing.rs, 610 lines):

The algorithm is the most complex compaction strategy. It MUST be preserved exactly:
1. Partition pinned vs candidates
2. Budget calculation (remaining = token_budget - pinned_tokens)
3. Iterative MMR selection: `λ * salience - (1-λ) * max_similarity(candidate, selected)`
4. Optional "lost in the middle" reordering
5. Emit `[pinned] ++ [selected]`

The `term_jaccard()` helper is a pure function — reuse as-is.

**Target code:**

```rust
/// Salience-aware context packing via iterative MMR selection.
///
/// The returned closure is passed to `Context::compact_with()`.
/// See module docs for algorithm details.
pub fn salience_packing_compactor(config: SaliencePackingConfig) -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        // Phase 1: Partition
        let (pinned, mut candidates): (Vec<_>, Vec<_>) = msgs
            .iter()
            .partition(|m| matches!(m.meta.policy, CompactionPolicy::Pinned));

        // Phase 2: Budget
        let pinned_tokens: usize = pinned.iter().map(|m| m.estimated_tokens()).sum();
        if pinned_tokens >= config.token_budget {
            return pinned.into_iter().cloned().collect();
        }
        let mut remaining = config.token_budget - pinned_tokens;

        // Phase 3: Iterative MMR selection
        let mut selected: Vec<Message> = Vec::new();
        let mut selected_texts: Vec<String> = Vec::new();

        while !candidates.is_empty() && remaining > 0 {
            let mut best_idx: Option<usize> = None;
            let mut best_mmr = f64::NEG_INFINITY;

            for (i, candidate) in candidates.iter().enumerate() {
                let sim1 = candidate.meta.salience.unwrap_or(config.default_salience);
                let sim2 = if selected_texts.is_empty() {
                    0.0
                } else {
                    let cand_text = candidate.text_content();
                    selected_texts.iter()
                        .map(|s| term_jaccard(&cand_text, s))
                        .fold(0.0_f64, f64::max)
                };
                let mmr = config.lambda * sim1 - (1.0 - config.lambda) * sim2;
                if mmr > best_mmr {
                    best_mmr = mmr;
                    best_idx = Some(i);
                }
            }

            let idx = best_idx.expect("candidates non-empty");
            let best = candidates.remove(idx);
            let tokens = best.estimated_tokens();
            if tokens <= remaining {
                remaining -= tokens;
                selected_texts.push(best.text_content());
                selected.push(best.clone());
            }
        }

        // Phase 4: Optional reordering (same algorithm)
        if config.reorder_for_recall && selected.len() > 2 {
            // ... identical reorder logic ...
        }

        // Phase 5: Emit
        let mut result: Vec<Message> = pinned.into_iter().cloned().collect();
        result.extend(selected);
        result
    }
}

// term_jaccard() is unchanged — pure function, no type dependencies.
```

**Key change:** `AnnotatedMessage.salience` → `Message.meta.salience`.
`Self::text_of(msg)` → `msg.text_content()` (method on Message from Task 1.1).
`self.estimate_single(msg)` → `msg.estimated_tokens()`.

**Tests:** All 15 salience packing tests rewritten with `Message` constructors.
The `term_jaccard` tests are pure and unchanged.

**Keep old `impl ContextStrategy` alive** — deleted in Phase 2 Task 2.4.

---

### Task R.3: TieredStrategy → `tiered_compactor()` closure

**Crate:** `skg-turn` (the `tiered.rs` module)
**Files:** Modify `neuron/turn/skg-turn/src/tiered.rs`
**Depends on:** Task 1.1 (Message type exists)
**One agent. One file.**

**Current code** (tiered.rs, 308 lines):

Algorithm: partition into 4 zones (Pinned, Active, Summary, Noise).
- Pinned: `CompactionPolicy::Pinned` → always kept
- Active: last `active_zone_size` Normal messages → always kept
- Summary: older Normal messages → summarised (or discarded if no summariser)
- Noise: `DiscardWhenDone` / `CompressFirst` → discarded

The `Summariser` trait takes `&[ProviderMessage]`. It needs to change to `&[Message]`.

**Target code:**

```rust
/// Summariser trait — takes &[Message] instead of &[ProviderMessage].
pub trait Summariser: Send + Sync {
    fn summarise(&self, messages: &[Message]) -> Result<Message, CompactionError>;
}

/// Tiered compactor: zone-partitioned compaction preventing recursive degradation.
pub fn tiered_compactor(
    config: TieredConfig,
    summariser: Option<Box<dyn Summariser>>,
) -> impl FnMut(&[Message]) -> Vec<Message> {
    move |msgs: &[Message]| {
        let mut pinned = Vec::new();
        let mut noise = Vec::new();
        let mut normal = Vec::new();

        for msg in msgs {
            match msg.meta.policy {
                CompactionPolicy::Pinned => pinned.push(msg.clone()),
                CompactionPolicy::DiscardWhenDone | CompactionPolicy::CompressFirst => {
                    noise.push(msg.clone())
                }
                CompactionPolicy::Normal => normal.push(msg.clone()),
            }
        }
        let _ = noise; // discarded

        let active_size = config.active_zone_size.min(normal.len());
        let split_point = normal.len().saturating_sub(active_size);
        let summary_candidates: Vec<Message> = normal.drain(..split_point).collect();

        let mut result = pinned;
        if !summary_candidates.is_empty() {
            if let Some(ref summariser) = summariser {
                if let Ok(summary_msg) = summariser.summarise(&summary_candidates) {
                    let mut s = summary_msg;
                    s.meta.policy = CompactionPolicy::Normal;
                    s.meta.source = Some("compaction:summary".into());
                    result.push(s);
                }
            }
        }
        result.extend(normal); // active zone
        result
    }
}
```

**Tests:** 5 tests rewritten with `Message` constructors. `TestSummariser` updated
to return `Message` instead of `ProviderMessage`.

---

### Task R.4: ContextAssembler → produce `Vec<Message>`

**Crate:** `skg-context`
**Files:** Modify `neuron/turn/skg-context/src/context_assembly.rs`
**Depends on:** Task 1.1 (Message type exists), conversion bridge in skg-turn
**One agent. One file.**

**Current code** (context_assembly.rs, 380 lines):

Reads state store data (decision cards, deltas, FTS hits) and assembles
`Vec<AnnotatedMessage>` with salience/source metadata.

Pure helpers (`recency_score`, `normalize_bm25_scores`, `text_msg`, `value_to_text`,
`parse_delta_timestamp`, `now_micros`) are algorithmically unchanged.

**Target change:**

```rust
// Before:
pub async fn assemble(...) -> Result<Vec<AnnotatedMessage>, StateError>

// After:
pub async fn assemble(...) -> Result<Vec<Message>, StateError>
```

Every `AnnotatedMessage { message, policy, source, salience }` construction becomes:

```rust
// Before:
AnnotatedMessage {
    message: text_msg(Role::User, &text),
    policy: Some(CompactionPolicy::Normal),
    source: Some("sweep:delta".into()),
    salience: Some(salience),
}

// After:
Message {
    role: layer0::Role::User,
    content: Content::text(&text),
    meta: MessageMeta {
        policy: CompactionPolicy::Normal,
        source: Some("sweep:delta".into()),
        salience: Some(salience),
        version: 0,
    },
}
```

The `text_msg()` helper is deleted — replaced by `Message::new(role, Content::text(s))`.
`AnnotatedMessage::pinned(...)` → `Message::pinned(role, Content::text(s))`.

**Pure helpers unchanged:** `recency_score`, `normalize_bm25_scores`, `parse_delta_timestamp`,
`now_micros`, `value_to_text`.

**Tests:** 8 tests updated to assert on `Message` fields instead of `AnnotatedMessage` fields.

---

### Task R.5: RedactionHook → `RedactionMiddleware`

**Crate:** `skg-hook-security`
**Files:** Modify `neuron/hooks/skg-hook-security/src/lib.rs`
**Depends on:** Task 1.2 (DispatchMiddleware trait exists)
**One agent. One file.**

**Current code** (lib.rs lines 17–80):

Pure logic: `patterns: Vec<Regex>`, `new()` builds 3 default patterns (AWS/Vault/GitHub),
`with_pattern()` adds custom. `on_event()` scans `ctx.operator_result` string and replaces
matches with `[REDACTED]`. Returns `ModifyDispatchOutput` if any match, else `Continue`.

**Target code:**

```rust
pub struct RedactionMiddleware {
    patterns: Vec<Regex>,
}

impl RedactionMiddleware {
    pub fn new() -> Self { /* identical to RedactionHook::new() */ }
    pub fn with_pattern(mut self, pattern: Regex) -> Self { /* identical */ }

    fn redact(&self, text: &str) -> Option<String> {
        let mut redacted = text.to_owned();
        let mut found = false;
        for pattern in &self.patterns {
            if pattern.is_match(&redacted) {
                found = true;
                redacted = pattern.replace_all(&redacted, "[REDACTED]").into_owned();
            }
        }
        found.then_some(redacted)
    }
}

#[async_trait]
impl DispatchMiddleware for RedactionMiddleware {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        let mut output = next.dispatch(operator, input).await?;
        // Scan output message content for secrets
        if let Some(text) = output.message.as_text() {
            if let Some(redacted) = self.redact(text) {
                output.message = Content::text(redacted);
            }
        }
        Ok(output)
    }
}
```

**Key difference from Hook version:** Middleware wraps the call (post-processing),
so it calls `next.dispatch()` first, then scans the output. The Hook version matched
on `HookPoint::PostSubDispatch` — same semantic, different mechanics.

**Tests:** All 8 redaction tests rewritten against `DispatchMiddleware`. The test creates
a mock `DispatchNext` that returns a fixed output, then asserts the middleware redacts it.
No more `HookContext` construction.

---

### Task R.6: ExfilGuardHook → `ExfilGuardMiddleware`

**Crate:** `skg-hook-security`
**Files:** Modify `neuron/hooks/skg-hook-security/src/lib.rs` (same file as R.5)
**Depends on:** Task 1.2 (DispatchMiddleware trait exists)
**One agent. Same file as R.5 — schedule AFTER R.5.**

**Current code** (lib.rs lines 91–234):

Pure logic: `detect_generic_exfil()`, `detect_shell_exfil()`, `detect_base64_exfil()`
— three detection methods checking URL+secret, curl/wget+env, base64+URL patterns.
These methods are pure `&self, &str -> bool`. They don't change at all.

**Target code:**

```rust
pub struct ExfilGuardMiddleware {
    /* identical fields to ExfilGuardHook */
}

impl ExfilGuardMiddleware {
    pub fn new() -> Self { /* identical to ExfilGuardHook::new() */ }
    pub fn with_url_pattern(mut self, pattern: Regex) -> Self { /* identical */ }

    // These three methods are UNCHANGED — pure logic, no type dependencies:
    fn detect_generic_exfil(&self, input: &str) -> bool { /* identical */ }
    fn detect_shell_exfil(&self, input: &str) -> bool { /* identical */ }
    fn detect_base64_exfil(&self, input: &str) -> bool { /* identical */ }
}

#[async_trait]
impl DispatchMiddleware for ExfilGuardMiddleware {
    async fn dispatch(
        &self,
        operator: &OperatorId,
        input: OperatorInput,
        next: &dyn DispatchNext,
    ) -> Result<OperatorOutput, OrchError> {
        // Pre-processing: check input BEFORE calling next
        let input_str = serde_json::to_string(&input.message).unwrap_or_default();

        if self.detect_generic_exfil(&input_str) {
            return Err(OrchError::DispatchFailed {
                agent: agent.to_string(),
                message: "Potential exfiltration: tool input contains URL and sensitive data".into(),
            });
        }
        if self.detect_shell_exfil(&input_str) {
            return Err(OrchError::DispatchFailed {
                agent: agent.to_string(),
                message: "Potential exfiltration: shell command pipes secret/env data to network tool".into(),
            });
        }
        if self.detect_base64_exfil(&input_str) {
            return Err(OrchError::DispatchFailed {
                agent: agent.to_string(),
                message: "Potential exfiltration: large base64 blob sent alongside URL".into(),
            });
        }

        // Input is clean — proceed
        next.dispatch(operator, input).await
    }
}
```

**Key difference from Hook version:** Middleware short-circuits by returning `Err`
instead of `HookAction::Halt`. This is the guard pattern — don't call `next` to halt.

**Tests:** All 10 exfil guard tests rewritten. Test creates a mock `DispatchNext`,
asserts the middleware returns `Err(OrchError::DispatchFailed { .. })` for exfil
and `Ok(...)` for clean input.

---

### Phase R verification

After all R tasks complete:

```bash
cd neuron/ && nix develop --command cargo test --workspace --all-targets
cd neuron/ && nix develop --command cargo clippy --workspace -- -D warnings
```

Expected: All old tests still pass (old code untouched). All new tests pass.
Both old and new code coexist — the old code is deleted in Phases 2–4.
