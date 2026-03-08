# Design Decisions

This page summarizes the key architectural decisions in neuron and the reasoning behind each one.

## Why `#[async_trait]` instead of native async traits

**Decision:** All Layer 0 protocol traits use `#[async_trait]` (heap-allocated futures). Internal traits in Layer 1 (like `Provider`) use RPITIT (native async, zero-cost).

**Reasoning:** Rust stabilized `async fn` in traits, but `async fn` in `dyn Trait` is still not available natively. Layer 0 traits *must* be object-safe because the entire composition model depends on `Box<dyn Operator>`, `Arc<dyn StateStore>`, etc. The `async_trait` macro provides this by boxing the returned future.

Internal traits like `Provider` are never used behind `dyn` -- they appear as generic type parameters in operator wrappers around `react_loop` (e.g., `struct MyOperator<P: Provider>`). These can use RPITIT for zero-cost abstraction. The object-safe boundary is the `Operator` trait, which is the protocol boundary.

**Future:** When Rust stabilizes `async fn in dyn Trait` with `Send` bounds, the Layer 0 traits will migrate to native async. This will be a breaking change in a minor version before v1.0.

## Why `serde_json::Value` for state values

**Decision:** `StateStore` stores `serde_json::Value`, not generic `T: Serialize`.

**Reasoning:** A generic `T` would destroy object safety. `StateStore` must work as `dyn StateStore` because orchestrators, environments, and operators all share a state store through trait objects. Making the trait generic over the value type would require callers to agree on concrete types at compile time, defeating the purpose of dynamic composition.

`serde_json::Value` is the universal interchange format for agentic systems. Every LLM API speaks JSON. Every tool accepts and returns JSON. The cost (no compile-time schema checking) is acceptable because state data crosses process boundaries, is persisted to disk, and may be read by different versions of the code.

## Why `rust_decimal::Decimal` for cost tracking

**Decision:** All monetary values (`OperatorMetadata.cost`, `OperatorConfig.max_cost`) use `rust_decimal::Decimal`.

**Reasoning:** Floating-point accumulation errors are real when tracking spend across thousands of LLM calls. `f64` introduces rounding errors that compound over time. A system that runs 10,000 model calls per day, each costing fractions of a cent, needs exact arithmetic to produce accurate cost reports and enforce budgets precisely.

`Decimal` adds one dependency to Layer 0 but eliminates an entire class of bugs.

## Why four protocols plus two interfaces

**Decision:** The architecture has four protocol traits (`Operator`, `Orchestrator`, `StateStore`, `Environment`) and two cross-cutting interfaces (per-boundary middleware, lifecycle events).

**Reasoning:** The four protocols are orthogonal concerns that compose independently:

1. **Operator** -- What happens in a single agent cycle (reasoning + acting).
2. **Orchestrator** -- How multiple agents compose (topology + durability).
3. **State** -- How data persists (storage backend).
4. **Environment** -- Where code runs (isolation + credentials).

These were derived from analyzing 23 architectural decisions that every agentic system must make. The four protocols cover all 23 decisions without overlap. Reducing to three protocols (by merging state into environment, or orchestration into operator) creates coupling where orthogonal concerns should be independent. Expanding to five or more protocols creates distinctions without meaningful boundaries.

The two interfaces (middleware and lifecycle events) are *cross-cutting* -- they span multiple protocols and cannot be owned by any single one. A budget event involves the operator (which tracks cost), the middleware (which observes it), and the orchestrator (which reacts to it). Making this a method on any single trait would couple unrelated protocols.

## Why edition 2024

**Decision:** The workspace uses Rust edition 2024.

**Reasoning:** Edition 2024 is the latest stable edition and provides native support for RPITIT (return position impl trait in traits) and other modern language features. This allows traits like `Provider` to use zero-cost async abstractions without workarounds like the `async_trait` macro. The Rust ecosystem has fully adopted 2024, providing excellent compatibility with all core dependencies.

## Why `#[non_exhaustive]` on all enums and structs

**Decision:** All public enums (`ExitReason`, `TriggerType`, etc.) and structs (`OperatorInput`, `OperatorOutput`, `OperatorConfig`, etc.) in Layer 0 are marked `#[non_exhaustive]`.

**Reasoning:** Layer 0 is the stability contract. Adding a variant to an enum or a field to a struct should not be a breaking change. `#[non_exhaustive]` forces downstream code to handle unknown variants (with `_ =>` arms) and prevents struct literal construction (forcing use of constructors or builder methods). This gives Layer 0 the freedom to evolve without breaking every implementation.

## Why operators declare effects instead of executing them

**Decision:** `OperatorOutput.effects` contains `Vec<Effect>` -- the operator declares side-effects but does not execute them.

**Reasoning:** The same operator code must work in radically different execution contexts. An operator running in-process has its effects executed immediately by the caller. An operator running inside a Temporal activity has its effects serialized and executed by the workflow engine. If the operator executed effects directly, it would be coupled to its execution context.

The effect declaration pattern makes operators pure functions over data: input in, output + effects out. The calling layer decides execution semantics.

## Why the Provider trait is not object-safe

**Decision:** The `Provider` trait (in `neuron-turn`) uses RPITIT and is not object-safe. It is never used behind `dyn Provider`.

**Reasoning:** Provider implementations are performance-critical -- they make HTTP calls to LLM APIs. The zero-cost abstraction of RPITIT (no heap allocation for the future) is worth the restriction of not using `dyn Provider`. The object-safe boundary is one layer up: a concrete operator wrapper (generic over `P: Provider`) implements `dyn Operator`. The generic type parameter is erased at the protocol boundary.

This is the general pattern: internal implementation traits can be generic and non-object-safe for performance. Protocol traits must be object-safe for composition. The bridge between them is a concrete type that is generic internally but implements an object-safe trait externally.
