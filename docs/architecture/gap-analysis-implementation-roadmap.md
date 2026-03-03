# Gap Analysis → Code: Implementation Roadmap

> Maps the March 2026 decision map gap analysis to concrete Neuron code changes.
> This document is the spec for the implementation branch.

---

## Overview

The gap analysis identified 1 new decision (D3D: Output Shape), expansions to
4 existing decisions (D3A, D5, D4B, C1), and several implementation-level gaps.
This document maps each to specific files, types, and changes in the Neuron
codebase.

---

## Phase 1: Protocol-Level Changes (Do First — Hardest to Change Later)

### 1A. Add `output_schema` to `ProviderRequest`

**File:** `turn/neuron-turn/src/types.rs` (~line 95)

**Current:**
```rust
pub struct ProviderRequest {
    pub model: Option<String>,
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub system: Option<String>,
    pub extra: serde_json::Value,
}
```

**Change:** Add `output_schema: Option<serde_json::Value>` field.

**Rationale:** Output shape enforcement (D3D) is the one genuinely new decision.
Every provider that supports constrained decoding (Anthropic tool_use, OpenAI
Structured Outputs) can use this field. Providers that don't can ignore it.
Putting it in `extra` makes it convention; putting it in the struct makes it
protocol.

**Downstream:** `neuron-provider-anthropic` should map this to the appropriate
API parameter. The `ReactOperator` in `neuron-op-react` should pass it through
when building the `ProviderRequest`.

**Tests:**
- `ProviderRequest` round-trips through serde with and without `output_schema`
- Provider implementations handle `Some(schema)` and `None` correctly

---

### 1B. Add exit reason variants to `ExitReason`

**File:** `layer0/src/operator.rs` (~line 96)

**Current:**
```rust
#[non_exhaustive]
pub enum ExitReason {
    Complete,
    MaxTurns,
    BudgetExhausted,
    CircuitBreaker,
    Timeout,
    ObserverHalt { reason: String },
    Error,
    Custom(String),
}
```

**Change:** Add three variants:
```rust
    /// Model determined the task is impossible.
    Infeasible,
    /// Loop/repetition detection triggered.
    StuckDetected,
    /// Routed to human instead of completing autonomously.
    HumanEscalation,
```

**Rationale:** The enum is `#[non_exhaustive]`, so this is additive and
non-breaking. Named variants are better than `Custom("infeasible")` for
pattern matching, observability dashboards, and metrics.

**Tests:**
- All new variants round-trip through serde_json
- Existing match arms still compile (non_exhaustive guarantees this)

---

## Phase 2: Turn Engine Improvements (High Impact, Layer 1)

### 2A. Loop/stuck detection in ReactOperator

**File:** `op/neuron-op-react/src/lib.rs` (~line 1100, exit condition checks)

**Current exit logic:** Checks MaxTurns, BudgetExhausted, Timeout, then
dispatches ExitCheck hook. No loop detection.

**Change:** Before the ExitCheck hook dispatch, add:
1. Track last N tool call sequences (tool name + input hash)
2. If the same sequence repeats K times consecutively, exit with
   `ExitReason::StuckDetected`
3. Make N and K configurable via `ReactConfig`

**Design consideration:** Semantic similarity (comparing model output
embeddings) is expensive and requires an embedding model. Start with the
cheaper heuristic: repeated identical tool calls. This catches the most
common failure mode (model calling the same tool with same args in a loop).

**Tests:**
- Unit test: feed ReactOperator a provider that always returns the same tool
  call → exits with StuckDetected after K repetitions
- Unit test: legitimate repeated calls with different args don't trigger

---

### 2B. Tool search and lazy loading

**Files:**
- `turn/neuron-tool/src/lib.rs` — `ToolRegistry`
- `op/neuron-op-react/src/lib.rs` — `ProviderRequest` building

**Current:** `ToolRegistry` has `register()` and `get()`. All registered tools'
schemas are sent to the provider every turn via `ProviderRequest.tools`.

**Change:**
1. Add `ToolRegistry::search(query: &str) -> Vec<&dyn ToolDyn>` method
   (keyword match on name + description; optionally embeddings later)
2. Add a built-in `ToolSearchTool` that wraps `ToolRegistry::search` and
   returns tool schemas as JSON
3. Add `ReactConfig::tool_loading: ToolLoading` enum:
   ```rust
   pub enum ToolLoading {
       /// Send all tool schemas every turn (current behavior).
       Eager,
       /// Send only tool names + descriptions; full schema loaded on demand.
       Lazy,
       /// Send no tools initially; agent uses ToolSearchTool to discover.
       SearchFirst,
   }
   ```
4. In `ReactOperator`, when building `ProviderRequest.tools`:
   - `Eager`: current behavior (all schemas)
   - `Lazy`: send `ToolSchema` with `input_schema: {}` (empty); when model
     calls a tool, if schema validation fails, inject full schema and retry
   - `SearchFirst`: send only `ToolSearchTool`; other tools' schemas added
     to context only when search returns them

**Impact:** 85-98% token reduction for large tool surfaces (measured by
Anthropic and Braintrust).

**Tests:**
- `Eager` mode behaves identically to current behavior (regression)
- `SearchFirst` mode: model can discover and call tools via search
- `Lazy` mode: tool schemas are loaded on demand

---

## Phase 3: Composition Improvements (When Building Real Orchestrator)

### 3A. Permission context on Effect::Delegate

**File:** `layer0/src/effect.rs` (~line 50)

**Current:**
```rust
Delegate {
    agent: AgentId,
    input: Box<OperatorInput>,
}
```

**Change:** Add optional delegation context:
```rust
Delegate {
    agent: AgentId,
    input: Box<OperatorInput>,
    /// Optional permission/credential scope for the delegated agent.
    /// When None, the orchestrator uses the child's default configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delegation_context: Option<DelegationContext>,
}
```

Where `DelegationContext` carries:
```rust
pub struct DelegationContext {
    /// Credential references the child should have access to.
    pub credentials: Vec<CredentialRef>,
    /// Tools the child is allowed to use (subset of parent's tools).
    pub allowed_tools: Option<Vec<String>>,
    /// Output schema the child's response must conform to.
    pub output_schema: Option<serde_json::Value>,
}
```

**Rationale:** This makes C1 (child context) and trust propagation explicit
in the protocol. The orchestrator can enforce scoped delegation without the
child having to know about the parent's full permission set.

**Tests:**
- `Effect::Delegate` round-trips with and without `delegation_context`
- Orchestrator respects `allowed_tools` when present

---

## Phase 4: Don't Do Yet

### Reasoning effort as named field
`extra` is the correct home until providers converge on a common API.
Anthropic uses token-count budgets; OpenAI uses low/medium/high enums.
Making it a named field means picking a representation that won't fit all
providers.

### Workload identity in auth traits
The auth crates are stubs (84 lines average). Design identity when
implementing a real backend (OIDC, k8s).

### OTel semantic conventions in hooks
The hook system maps cleanly to `gen_ai.*` attributes. This is a new
crate (`neuron-otel` or similar) that implements the `Hook` trait and
emits OTel spans. Design when observability becomes a priority.

---

## File Index

| File | Phase | Change |
|------|-------|--------|
| `turn/neuron-turn/src/types.rs` | 1A | Add `output_schema` field |
| `layer0/src/operator.rs` | 1B | Add ExitReason variants |
| `op/neuron-op-react/src/lib.rs` | 2A, 2B | Loop detection, tool loading modes |
| `turn/neuron-tool/src/lib.rs` | 2B | Add `search()` to ToolRegistry |
| `layer0/src/effect.rs` | 3A | Add DelegationContext to Delegate |

---

*Source: `decision-map-gap-analysis-2026.md`, `decision-map-new-decision-audit.md`*
*Created: 2026-03-03*
