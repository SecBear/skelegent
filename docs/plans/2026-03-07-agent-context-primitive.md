# AgentContext Primitive + Unified Observability Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make context a first-class layer0 primitive with typed methods, universal tracing, and intervention hooks — giving neuron best-in-class context engineering ergonomics.

**Architecture:** Three orthogonal systems, each doing one thing well. (1) `AgentContext` — a layer0 type owning the context window with safe mutation methods. (2) `tracing` — `#[instrument]` on every trait boundary for universal observation. (3) Hooks — lean intervention at system boundaries via `HookPayload` and expanded `HookPoint`. `AgentContext` methods serve as natural interception points for context mutations; tracing handles observation; hooks handle halt/modify at non-context boundaries.

**Tech Stack:** Rust, `tracing` 0.1, layer0 protocol types, `#[non_exhaustive]` for forward compatibility.

**Research basis:** `golden/projects/neuron/context-engineering-sdk-landscape.md` — deep analysis of OpenAI, Anthropic, LangChain/LangGraph, Google ADK, CrewAI, Vercel AI, Semantic Kernel/AutoGen, AWS Bedrock, Letta/MemGPT, OpenHands, Cursor.

**Key design principles from research:**
- Typed, versioned context segments (not raw message arrays)
- Explicit `compile()` / `to_provider_request()` — context assembly is a visible operation
- Immutable snapshots for introspection and deterministic replay
- Policy enforcement at both write and compile stages
- Per-message metadata (salience, source, compaction policy) — neuron already has this via `AnnotatedMessage`
- Observable mutations via structured events
- Ownership model for safe concurrent access (`&` = read, `&mut` = write)

---

## Phase 1: AgentContext in layer0

### Task 1: Define `AgentContext` type and `ContextWatcher` trait

**Files:**
- Create: `neuron/layer0/src/context.rs`
- Modify: `neuron/layer0/src/lib.rs` (add `pub mod context` and re-exports)

**Step 1: Write the failing test**

Create `neuron/layer0/src/context.rs` with the type definition and basic tests:

```rust
//! First-class context primitive — the agent's view of the world.
//!
//! `AgentContext` owns the message window, system prompt, and per-message
//! metadata. Every mutation method is a natural interception point:
//! registered [`ContextWatcher`]s observe and can reject mutations.
//!
//! The Rust ownership model enforces access control:
//! `&AgentContext` = read-only, `&mut AgentContext` = mutation.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::id::AgentId;
use crate::lifecycle::CompactionPolicy;

/// Per-message annotation. Carries metadata that survives the message lifecycle.
///
/// This is the layer0 protocol type. `neuron-turn`'s `AnnotatedMessage` wraps
/// a `ProviderMessage` with these annotations — we define the annotation
/// contract here so all layers share it.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMeta {
    /// Compaction policy for this message.
    #[serde(default)]
    pub policy: CompactionPolicy,
    /// Source of this message (e.g. `"user"`, `"tool:shell"`, `"mcp:github"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Importance hint (0.0–1.0). Higher = more likely to survive compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// Monotonic version. Incremented on every mutation to this message.
    #[serde(default)]
    pub version: u64,
}

impl Default for MessageMeta {
    fn default() -> Self {
        Self {
            policy: CompactionPolicy::Normal,
            source: None,
            salience: None,
            version: 0,
        }
    }
}

/// A message in the context window, with its annotation.
///
/// Generic over the message type `M` so layer0 doesn't depend on
/// `neuron-turn`'s `ProviderMessage`. The turn layer provides the concrete
/// type; layer0 defines the contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessage<M> {
    /// The underlying message payload.
    pub message: M,
    /// Per-message metadata.
    pub meta: MessageMeta,
}

/// Where to inject a message in the context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Append after the last message.
    Back,
    /// Insert before the first message.
    Front,
    /// Insert at a specific index.
    At(usize),
}

/// What a watcher decides after observing a mutation.
#[derive(Debug, Clone)]
pub enum WatcherVerdict {
    /// Allow the mutation.
    Allow,
    /// Reject the mutation with a reason.
    Reject { reason: String },
}

/// Observes and can intervene on `AgentContext` mutations.
///
/// Watchers fire synchronously during mutations. Keep them fast.
/// Default implementations return `Allow` — implement only what you need.
pub trait ContextWatcher: Send + Sync {
    /// Before a message is injected.
    fn on_inject<M>(&self, _msg: &ContextMessage<M>, _pos: Position) -> WatcherVerdict
    where M: std::fmt::Debug {
        WatcherVerdict::Allow
    }

    /// Before messages are removed.
    fn on_remove(&self, _count: usize) -> WatcherVerdict {
        WatcherVerdict::Allow
    }

    /// Before compaction runs. Opportunity to flush state.
    fn on_pre_compact(&self, _message_count: usize) -> WatcherVerdict {
        WatcherVerdict::Allow
    }

    /// After compaction completes.
    fn on_post_compact(&self, _removed: usize, _remaining: usize) {}
}

/// Read-only snapshot of context state for introspection.
///
/// Cheap to create (clones message metadata, not full messages).
/// Satisfies L5 context introspection requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// Number of messages in the window.
    pub message_count: usize,
    /// Per-message metadata (without the message payloads).
    pub message_metas: Vec<MessageMeta>,
    /// System prompt (if set).
    pub has_system: bool,
    /// Agent identity.
    pub agent_id: AgentId,
    /// Total estimated tokens.
    pub estimated_tokens: usize,
}

/// Errors from context operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ContextError {
    /// A watcher rejected the mutation.
    #[error("context mutation rejected: {reason}")]
    Rejected { reason: String },
    /// Index out of bounds.
    #[error("position {index} out of bounds (len={len})")]
    OutOfBounds { index: usize, len: usize },
}

/// The agent's context window — first-class, observable, safe.
///
/// Every mutation method fires registered [`ContextWatcher`]s before
/// applying the change. The Rust type system enforces access:
/// `&AgentContext<M>` for reads, `&mut AgentContext<M>` for writes.
///
/// Generic over message type `M` so layer0 stays independent of
/// `neuron-turn`'s `ProviderMessage`.
pub struct AgentContext<M> {
    agent_id: AgentId,
    messages: Vec<ContextMessage<M>>,
    system: Option<String>,
    watchers: Vec<Arc<dyn ContextWatcher>>,
}

impl<M: Clone + std::fmt::Debug> AgentContext<M> {
    /// Create a new empty context for an agent.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            messages: Vec::new(),
            system: None,
            watchers: Vec::new(),
        }
    }

    /// Register a context watcher.
    pub fn add_watcher(&mut self, watcher: Arc<dyn ContextWatcher>) {
        self.watchers.push(watcher);
    }

    // ── Reads ──

    /// Current messages in the context window.
    pub fn messages(&self) -> &[ContextMessage<M>] {
        &self.messages
    }

    /// Number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the context is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Current system prompt.
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    /// Agent identity.
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Create a read-only snapshot for introspection.
    pub fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot {
            message_count: self.messages.len(),
            message_metas: self.messages.iter().map(|m| m.meta.clone()).collect(),
            has_system: self.system.is_some(),
            agent_id: self.agent_id.clone(),
            estimated_tokens: 0, // caller provides via token estimator
        }
    }

    // ── Mutations ──

    /// Set the system prompt.
    pub fn set_system(&mut self, system: impl Into<String>) {
        self.system = Some(system.into());
    }

    /// Clear the system prompt.
    pub fn clear_system(&mut self) {
        self.system = None;
    }

    /// Inject a message at a position. Fires watchers.
    pub fn inject(&mut self, msg: ContextMessage<M>, pos: Position) -> Result<(), ContextError> {
        // Fire watchers
        for w in &self.watchers {
            if let WatcherVerdict::Reject { reason } = w.on_inject(&msg, pos) {
                return Err(ContextError::Rejected { reason });
            }
        }

        match pos {
            Position::Back => self.messages.push(msg),
            Position::Front => self.messages.insert(0, msg),
            Position::At(idx) => {
                if idx > self.messages.len() {
                    return Err(ContextError::OutOfBounds {
                        index: idx,
                        len: self.messages.len(),
                    });
                }
                self.messages.insert(idx, msg);
            }
        }
        Ok(())
    }

    /// Remove the last N messages. Fires watchers.
    pub fn truncate_back(&mut self, count: usize) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let actual = count.min(self.messages.len());
        for w in &self.watchers {
            if let WatcherVerdict::Reject { reason } = w.on_remove(actual) {
                return Err(ContextError::Rejected { reason });
            }
        }
        let start = self.messages.len() - actual;
        Ok(self.messages.drain(start..).collect())
    }

    /// Remove the first N messages. Fires watchers.
    pub fn truncate_front(&mut self, count: usize) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let actual = count.min(self.messages.len());
        for w in &self.watchers {
            if let WatcherVerdict::Reject { reason } = w.on_remove(actual) {
                return Err(ContextError::Rejected { reason });
            }
        }
        Ok(self.messages.drain(..actual).collect())
    }

    /// Remove messages matching a predicate. Fires watchers.
    pub fn remove_where(&mut self, pred: impl Fn(&ContextMessage<M>) -> bool) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let count = self.messages.iter().filter(|m| pred(m)).count();
        if count > 0 {
            for w in &self.watchers {
                if let WatcherVerdict::Reject { reason } = w.on_remove(count) {
                    return Err(ContextError::Rejected { reason });
                }
            }
        }
        let mut removed = Vec::new();
        self.messages.retain(|m| {
            if pred(m) {
                removed.push(m.clone());
                false
            } else {
                true
            }
        });
        Ok(removed)
    }

    /// Transform messages in place.
    pub fn transform(&mut self, mut f: impl FnMut(&mut ContextMessage<M>)) {
        for msg in &mut self.messages {
            f(msg);
            msg.meta.version += 1;
        }
    }

    /// Extract messages matching a predicate (non-destructive).
    pub fn extract(&self, pred: impl Fn(&ContextMessage<M>) -> bool) -> Vec<&ContextMessage<M>> {
        self.messages.iter().filter(|m| pred(m)).collect()
    }

    /// Direct mutable access to messages (escape hatch).
    /// Prefer typed methods above — this exists for compaction strategies
    /// that need full control.
    pub fn messages_mut(&mut self) -> &mut Vec<ContextMessage<M>> {
        &mut self.messages
    }

    /// Replace all messages (used by compaction). Fires watchers.
    pub fn replace_messages(&mut self, new_messages: Vec<ContextMessage<M>>) -> Result<Vec<ContextMessage<M>>, ContextError> {
        let old_count = self.messages.len();
        let new_count = new_messages.len();
        let removed = old_count.saturating_sub(new_count);

        // Fire pre-compact watchers
        for w in &self.watchers {
            if let WatcherVerdict::Reject { reason } = w.on_pre_compact(old_count) {
                return Err(ContextError::Rejected { reason });
            }
        }

        let old = std::mem::replace(&mut self.messages, new_messages);

        // Fire post-compact watchers
        for w in &self.watchers {
            w.on_post_compact(removed, self.messages.len());
        }

        Ok(old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestMsg = String;

    fn msg(s: &str) -> ContextMessage<TestMsg> {
        ContextMessage {
            message: s.to_string(),
            meta: MessageMeta::default(),
        }
    }

    #[test]
    fn new_context_is_empty() {
        let ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);
        assert!(ctx.system().is_none());
    }

    #[test]
    fn inject_back() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("hello"), Position::Back).unwrap();
        ctx.inject(msg("world"), Position::Back).unwrap();
        assert_eq!(ctx.len(), 2);
        assert_eq!(ctx.messages()[0].message, "hello");
        assert_eq!(ctx.messages()[1].message, "world");
    }

    #[test]
    fn inject_front() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("first"), Position::Back).unwrap();
        ctx.inject(msg("before"), Position::Front).unwrap();
        assert_eq!(ctx.messages()[0].message, "before");
    }

    #[test]
    fn inject_at_index() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("a"), Position::Back).unwrap();
        ctx.inject(msg("c"), Position::Back).unwrap();
        ctx.inject(msg("b"), Position::At(1)).unwrap();
        assert_eq!(ctx.messages()[1].message, "b");
    }

    #[test]
    fn inject_out_of_bounds() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        let result = ctx.inject(msg("x"), Position::At(5));
        assert!(result.is_err());
    }

    #[test]
    fn truncate_back() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        for i in 0..5 {
            ctx.inject(msg(&format!("msg{i}")), Position::Back).unwrap();
        }
        let removed = ctx.truncate_back(2).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn truncate_front() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        for i in 0..5 {
            ctx.inject(msg(&format!("msg{i}")), Position::Back).unwrap();
        }
        let removed = ctx.truncate_front(2).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(ctx.len(), 3);
        assert_eq!(ctx.messages()[0].message, "msg2");
    }

    #[test]
    fn watcher_can_reject_inject() {
        struct RejectAll;
        impl ContextWatcher for RejectAll {
            fn on_inject<M: std::fmt::Debug>(&self, _msg: &ContextMessage<M>, _pos: Position) -> WatcherVerdict {
                WatcherVerdict::Reject { reason: "nope".into() }
            }
        }

        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.add_watcher(Arc::new(RejectAll));
        let result = ctx.inject(msg("blocked"), Position::Back);
        assert!(result.is_err());
        assert!(ctx.is_empty());
    }

    #[test]
    fn snapshot_captures_state() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("agent-1".into());
        ctx.set_system("You are a test agent.");
        ctx.inject(msg("hello"), Position::Back).unwrap();
        ctx.inject(msg("world"), Position::Back).unwrap();

        let snap = ctx.snapshot();
        assert_eq!(snap.message_count, 2);
        assert!(snap.has_system);
        assert_eq!(snap.agent_id.as_str(), "agent-1");
    }

    #[test]
    fn transform_increments_version() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("hello"), Position::Back).unwrap();
        assert_eq!(ctx.messages()[0].meta.version, 0);

        ctx.transform(|m| m.message = m.message.to_uppercase());
        assert_eq!(ctx.messages()[0].message, "HELLO");
        assert_eq!(ctx.messages()[0].meta.version, 1);
    }

    #[test]
    fn replace_messages_fires_compact_watchers() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct CompactWatcher {
            fired: AtomicBool,
        }
        impl ContextWatcher for CompactWatcher {
            fn on_post_compact(&self, _removed: usize, _remaining: usize) {
                self.fired.store(true, Ordering::SeqCst);
            }
        }

        let watcher = Arc::new(CompactWatcher {
            fired: AtomicBool::new(false),
        });

        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.add_watcher(watcher.clone());
        for i in 0..5 {
            ctx.inject(msg(&format!("msg{i}")), Position::Back).unwrap();
        }

        let _old = ctx.replace_messages(vec![msg("summary")]).unwrap();
        assert!(watcher.fired.load(Ordering::SeqCst));
        assert_eq!(ctx.len(), 1);
    }

    #[test]
    fn remove_where_filters() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("keep"), Position::Back).unwrap();
        ctx.inject(msg("drop"), Position::Back).unwrap();
        ctx.inject(msg("keep2"), Position::Back).unwrap();

        let removed = ctx.remove_where(|m| m.message.starts_with("drop")).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(ctx.len(), 2);
    }

    #[test]
    fn extract_non_destructive() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        ctx.inject(msg("a"), Position::Back).unwrap();
        ctx.inject(msg("b"), Position::Back).unwrap();

        let found = ctx.extract(|m| m.message == "b");
        assert_eq!(found.len(), 1);
        assert_eq!(ctx.len(), 2); // not removed
    }

    #[test]
    fn system_prompt_lifecycle() {
        let mut ctx: AgentContext<TestMsg> = AgentContext::new("test".into());
        assert!(ctx.system().is_none());
        ctx.set_system("hello");
        assert_eq!(ctx.system(), Some("hello"));
        ctx.clear_system();
        assert!(ctx.system().is_none());
    }
}
```

**Step 2: Register the module in `lib.rs`**

Add to `neuron/layer0/src/lib.rs`:
```rust
pub mod context;
```

And add re-exports:
```rust
pub use context::{
    AgentContext, ContextError, ContextMessage, ContextSnapshot, ContextWatcher,
    MessageMeta, Position, WatcherVerdict,
};
```

**Step 3: Run tests to verify**

Run: `cd neuron && nix develop -c cargo test -p layer0 -- context`
Expected: All tests pass.

**Step 4: Run full layer0 verification**

Run: `cd neuron && nix develop -c cargo clippy -p layer0 -- -D warnings`
Expected: Clean.

**Step 5: Commit**

```bash
git add layer0/src/context.rs layer0/src/lib.rs
git commit -m "feat(layer0): add AgentContext primitive with ContextWatcher

First-class context type with typed messages, per-message metadata,
positional injection, truncation, filtering, transformation, and
snapshot introspection. ContextWatcher trait enables observation and
rejection of mutations. Rust ownership enforces read/write access."
```

---

### Task 2: Expand HookPoint and add HookPayload for system boundaries

**Files:**
- Modify: `neuron/layer0/src/hook.rs`

**Step 1: Add new HookPoint variants**

Add to the `HookPoint` enum (after `PreMemoryWrite`):

```rust
    // ── Provider layer ──

    /// Before a provider API call. Payload: `HookPayload::Provider`.
    PreProviderCall,
    /// After a provider API call returns. Payload: `HookPayload::Provider`.
    PostProviderCall,

    // ── Orchestration layer ──

    /// Before an operator is dispatched via orchestrator. Payload: `HookPayload::Dispatch`.
    PreDispatch,
    /// After an operator dispatch completes. Payload: `HookPayload::Dispatch`.
    PostDispatch,

    // ── State layer ──

    /// Before a state store write. Payload: `HookPayload::State`.
    PreStateWrite,
    /// After a state store read. Payload: `HookPayload::State`.
    PostStateRead,

    // ── Lifecycle layer ──

    /// Before compaction destroys messages. Payload: `HookPayload::Compaction`.
    PreCompaction,
    /// After compaction completes. Payload: `HookPayload::Compaction`.
    PostCompaction,
    /// Before Temporal continue-as-new resets event history.
    PreContinueAsNew,
```

**Step 2: Add HookPayload enum**

```rust
/// Typed data available at a hook point.
///
/// Each variant carries the actual data flowing through the boundary,
/// not just metadata. Hooks can observe the data and (for Transformer
/// hooks) return `ModifyPayload` to change it.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookPayload {
    /// At PreDispatch/PostDispatch — operator invocation.
    Dispatch {
        agent_id: String,
        input: Option<serde_json::Value>,
        output: Option<serde_json::Value>,
    },

    /// At PreStateWrite/PostStateRead — state I/O.
    State {
        scope: String,
        key: String,
        value: Option<serde_json::Value>,
    },

    /// At PreCompaction/PostCompaction — context about to be compacted.
    Compaction {
        message_count_before: usize,
        message_count_after: Option<usize>,
    },
}
```

**Step 3: Add payload, identity, and replay fields to HookContext**

```rust
    /// Typed payload specific to this hook point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<HookPayload>,

    /// Agent identity in scope. Enables per-agent hook filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<crate::id::AgentId>,

    /// Workflow identity. Enables per-workflow hook filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<crate::id::WorkflowId>,

    /// Whether this event is a replay (durable execution).
    /// Hooks with side effects should check this.
    #[serde(default)]
    pub replaying: bool,
```

**Step 4: Add ModifyPayload to HookAction**

```rust
    /// Replace the hook payload with modified data.
    /// Only valid from Transformer hooks at Pre* points.
    ModifyPayload {
        /// The replacement payload.
        payload: HookPayload,
    },
```

**Step 5: Update HookContext::new() to initialize new fields**

**Step 6: Add serde roundtrip tests for new variants**

**Step 7: Run tests and clippy**

Run: `cd neuron && nix develop -c cargo test -p layer0 && nix develop -c cargo clippy -p layer0 -- -D warnings`

**Step 8: Commit**

```bash
git commit -m "feat(layer0): expand HookPoint + add HookPayload for typed intervention

New hook points: PreProviderCall, PostProviderCall, PreDispatch,
PostDispatch, PreStateWrite, PostStateRead, PreCompaction,
PostCompaction, PreContinueAsNew.

HookPayload enum carries typed data at each boundary.
HookContext gains agent_id, workflow_id, replaying, payload fields.
HookAction gains ModifyPayload variant for typed intervention."
```

---

## Phase 2: Universal tracing instrumentation

### Task 3: Add `tracing` dep and `#[instrument]` to neuron core crates

**Files:**
- Modify: `neuron/turn/neuron-turn/Cargo.toml` (add `tracing = "0.1"`)
- Modify: `neuron/turn/neuron-turn/src/provider.rs` — NOTE: Provider is RPITIT, not `#[async_trait]`. Use `tracing::info_span!` manually inside impl, not `#[instrument]`.
- Modify: `neuron/orch/neuron-orch-local/Cargo.toml` (add `tracing = "0.1"`)
- Modify: `neuron/orch/neuron-orch-local/src/lib.rs` — `dispatch()` and `dispatch_many()`
- Modify: `neuron/hooks/neuron-hooks/Cargo.toml` (add `tracing = "0.1"`)
- Modify: `neuron/hooks/neuron-hooks/src/lib.rs` — `HookRegistry::dispatch()`
- Modify: `neuron/op/neuron-op-single-shot/Cargo.toml` (add `tracing = "0.1"`)
- Modify: `neuron/op/neuron-op-single-shot/src/lib.rs` — `execute()`
- Modify: `neuron/op/neuron-op-react/Cargo.toml` (add `tracing = "0.1"`)
- Modify: `neuron/op/neuron-op-react/src/lib.rs` — `execute()`, key loop points

**Pattern for each crate:**

1. Add `tracing = "0.1"` to `[dependencies]` in Cargo.toml
2. Add `use tracing::{info, debug, info_span, Instrument};` where needed
3. Add `#[instrument(skip_all, fields(...))]` on trait impl methods
4. For RPITIT (Provider): use `async { ... }.instrument(info_span!(...))` pattern

**Key span fields per boundary:**

| Boundary | Span fields |
|---|---|
| `Provider::complete` | `model`, `max_tokens` |
| `Orchestrator::dispatch` | `agent_id` |
| `Operator::execute` | `trigger` |
| `HookRegistry::dispatch` | `point`, `hook_count` |

**Step 1: Add tracing dep to all crates**

**Step 2: Instrument each boundary**

**Step 3: Run full workspace verification**

Run: `cd neuron && ./scripts/verify.sh`
Expected: 573 tests pass, clippy clean.

**Step 4: Commit**

```bash
git commit -m "feat: universal tracing instrumentation across neuron core

#[instrument] on Provider::complete, Orchestrator::dispatch,
Operator::execute, HookRegistry::dispatch. Every trait boundary
emits structured spans. RUST_LOG configures per-layer verbosity."
```

---

### Task 4: Add `tracing` to extras crates

**Files:**
- Modify: Multiple Cargo.toml files in extras/ (state, orch, auth, client, effects)
- Modify: Corresponding src/lib.rs files

**Same pattern as Task 3, applied to:**
- `neuron-state-sqlite` — `StateStore::read`, `write`, `delete`
- `neuron-orch-kit` — `EffectInterpreter::execute_effect`
- `neuron-auth-pi`, `neuron-auth-omp` — auth flows
- `neuron-client-parallel` — HTTP calls
- `neuron-effects-git` — git operations

**Step 1: Add deps and instrument**

**Step 2: Run verification**

Run: `cd extras && nix develop -c cargo test --workspace --all-targets && nix develop -c cargo clippy --workspace -- -D warnings`

**Step 3: Commit**

```bash
git commit -m "feat: tracing instrumentation across extras crates

Structured spans on StateStore, EffectInterpreter, AuthProvider,
ParallelClient, GitHubClient. Full observability stack."
```

---

## Phase 3: Wire hooks at system boundaries

### Task 5: Wire PreDispatch/PostDispatch in LocalOrch

**Files:**
- Modify: `neuron/orch/neuron-orch-local/Cargo.toml` (add `neuron-hooks` dep)
- Modify: `neuron/orch/neuron-orch-local/src/lib.rs`

**Step 1: Add optional HookRegistry to LocalOrch**

```rust
pub struct LocalOrch {
    agents: HashMap<String, Arc<dyn Operator>>,
    hooks: Option<Arc<HookRegistry>>,
}
```

**Step 2: Fire PreDispatch before execute, PostDispatch after**

**Step 3: Write test with a mock hook that observes dispatch**

**Step 4: Run tests and clippy**

**Step 5: Commit**

---

### Task 6: Wire PreStateWrite/PostStateRead in neuron-state-sqlite

**Files:**
- Modify: `extras/state/neuron-state-sqlite/Cargo.toml`
- Modify: `extras/state/neuron-state-sqlite/src/lib.rs`

**Same pattern: optional HookRegistry, fire at read/write boundaries.**

---

### Task 7: Wire PreCompaction/PostCompaction in ReactOperator

**Files:**
- Modify: `neuron/op/neuron-op-react/src/lib.rs`

**Fire PreCompaction before the compaction strategy runs, PostCompaction after.
Include message count before/after in HookPayload::Compaction.**

---

## Phase 4: Remove manual tracing from sweep operators

### Task 8: Remove manual info!/debug! from CompareOperator, ResearchOperator, SynthesisOperator

**Files:**
- Modify: `extras/op/neuron-op-sweep/src/compare.rs`
- Modify: `extras/op/neuron-op-sweep/src/research_operator.rs`
- Modify: `extras/op/neuron-op-sweep/src/synthesis_operator.rs`

**Keep:** `OperatorMetadata` population (that's data, not logging).
**Remove:** Manual `info!`, `debug!` calls that duplicate what `#[instrument]` provides.
**Keep:** Domain-specific debug logging (full prompt text at debug level) that tracing spans can't capture.

---

## Phase 5: Integration verification

### Task 9: End-to-end verification

**Step 1: Run neuron verification**

```bash
cd neuron && ./scripts/verify.sh
```
Expected: 573+ tests pass, clippy clean.

**Step 2: Run extras verification**

```bash
cd extras && nix develop -c cargo test --workspace --all-targets
cd extras && nix develop -c cargo clippy --workspace -- -D warnings
```
Expected: 301+ tests pass, clippy clean.

**Step 3: Run golden sweep runner**

```bash
cd golden/sweep-runner
nix develop -c cargo build
RUST_LOG=info nix develop -c cargo run -- --decisions-dir ../../golden/decisions --decisions D1 --research-mode search
```
Expected: Structured tracing output, real cost/duration in verdict.

**Step 4: Test tracing visibility**

```bash
RUST_LOG=debug nix develop -c cargo run -- --decisions-dir ../../golden/decisions --decisions D1 --research-mode search
```
Expected: Full span tree visible — orchestrator dispatch → operator execute → provider complete.

**Step 5: Commit and tag**

---

## Summary

| Phase | What | Crates touched | Tests added |
|---|---|---|---|
| 1 | `AgentContext` in layer0, `HookPayload`/expanded `HookPoint` | layer0 | ~15 |
| 2 | `#[instrument]` on neuron core | 5 neuron crates | 0 (tracing is passive) |
| 3 | `#[instrument]` on extras | 6 extras crates | 0 |
| 4 | Wire hooks at orchestrator, state, compaction | 3 crates | ~5 |
| 5 | Clean up sweep operator logging | 3 files | 0 |
| 6 | E2E verification | - | - |

**What neuron gets that no other SDK has:**
- First-class `AgentContext` with typed, versioned messages and per-message metadata
- `ContextWatcher` trait for safe, synchronous intervention on context mutations
- `ContextSnapshot` for L5 context introspection
- Universal tracing with zero boilerplate (`RUST_LOG` controls everything)
- Typed `HookPayload` at system boundaries for intervention with actual data
- Rust ownership model enforcing `&`=read, `&mut`=write access control
- Replay awareness (`replaying` field) for durable execution correctness
- Everything composable: watchers on context, hooks at boundaries, tracing everywhere
