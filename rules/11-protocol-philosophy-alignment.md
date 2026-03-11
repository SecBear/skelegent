# Rule 11 — Protocol Trait Implementation Checklist

Before implementing any protocol trait, read (in order):

1. The trait definition in `layer0/src/` — every doc comment and default method.
2. The governing spec in `specs/`.
3. The relevant section of `ARCHITECTURE.md`.
4. At least one existing implementation of the same trait.

An agent MUST NOT begin writing code until it can answer these questions:

- What are the default methods and what do they mean? (Capabilities the backend MAY opt into.)
- What graceful degradation pattern does this trait use? (Empty vec? No-op? Error?)
- What is advisory vs enforced? (Hints that backends MAY ignore vs contracts they MUST honor.)
- What is the extension pattern? (How do new capabilities get added without breaking existing backends?)
- What composition contracts does this trait participate in? (Who calls it? What do they assume?)

## Checklist

For each protocol trait, the philosophy is captured in this checklist. Implementations MUST satisfy all applicable items:

#### StateStore

- [ ] `search()` MUST return empty `Vec` (not error) when the backend has no search capability.
- [ ] `read_hinted()` / `write_hinted()` MUST delegate to `read()` / `write()` by default. Backends MAY override to use hints but MUST NOT require them.
- [ ] `clear_transient()` MUST be a no-op by default. Backends that support transient storage MAY override.
- [ ] `link()` / `unlink()` / `traverse()` / `search_hinted()` MUST have default no-op/empty implementations. Backends that support graph/vector capabilities MAY override.
- [ ] All `StoreOptions` fields are advisory. Backends MUST store data correctly even if they ignore every hint.
- [ ] Scope isolation MUST be enforced. Operations on one scope MUST NOT affect another.
- [ ] Errors MUST use `StateError` variants. Backends MUST NOT panic on missing keys, empty queries, or unsupported operations.

#### Operator

- [ ] Operators MUST declare effects; they MUST NOT execute them (write state, call APIs, perform I/O directly).
- [ ] Operators MUST receive external services via dependency-injected traits (`Box<dyn Trait>`), never via concrete imports.
- [ ] Operator output MUST be deterministic given the same input and provider responses.

#### Dispatcher, Signalable, Queryable



- [ ] `Dispatcher::dispatch()` MUST be transport-agnostic. Local function call, HTTP, Temporal — the Dispatcher trait MUST NOT assume mechanism.

- [ ] `Signalable::signal()` MUST be fire-and-forget. The caller MUST NOT assume the signal was processed.

- [ ] `Queryable::query()` MUST be read-only and MUST NOT mutate state.


#### Environment

- [ ] `run()` MUST execute the operator within the specified isolation boundary.
- [ ] Credential injection MUST be mediated by the environment, never by the operator itself.
- [ ] Error messages MUST NOT contain secret material.

#### Middleware

- [ ] Middleware MUST NOT change control flow outside its boundary (that's Steering's job). Middleware intercepts at defined boundaries (dispatch, store, exec).
- [ ] Middleware that fails MUST be treated as a pass-through (logged, not fatal).
- [ ] Observer middleware MUST all run. Guardrail middleware MUST short-circuit on rejection. Transformer middleware MUST chain.

## When adding new default methods to a protocol trait

- The new method MUST have a default implementation that preserves backward compatibility.
- The default MUST follow the trait's established degradation pattern (empty vec, no-op, delegate to basic method).
- The method MUST be added to the philosophy checklist above.
- The governing spec MUST be updated before the code change (Rule 03).

## When implementing a new backend crate

- The crate MUST include a doc comment on its `impl` block stating which optional capabilities it supports and which it degrades on.
- The crate MUST include a trait compliance test (e.g., `fn _assert_state_store<T: StateStore>() {}`).
- The crate SHOULD include tests that verify graceful degradation for capabilities it does NOT support.

## Anti-patterns

- Implementing a trait based on the method signatures alone, without reading doc comments or existing implementations.
- Adding a method to a protocol trait without a default implementation (breaks all existing backends).
- Returning errors for unsupported capabilities instead of graceful degradation (empty vec, no-op).
- Downcasting `&dyn StateStore` to a concrete type to access backend-specific features (violates composition).
- Storing operator logic in effect executors or orchestrators (violates declaration/execution boundary).
- Adding boolean flags to protocol traits instead of traited opt-in components.

## Examples

- Good: `skg-state-sqlite` implements `search()` via FTS5, stores metadata from `write_hinted()`, returns empty vec when FTS5 query matches nothing.
- Good: `skg-state-memory` implements `search()` as empty vec (no search capability), `write_hinted()` routes transient entries to separate table.
- Good: `SweepOperator` takes `Box<dyn ResearchProvider>` — never imports `SweepProvider` directly.
- Bad: A state store that throws `StateError::NotSupported` when `search()` is called (should return empty vec).
- Bad: An operator that `use reqwest::Client` to call an API directly (should go through injected provider trait).
- Bad: Adding `fn supports_graph(&self) -> bool` to StateStore (capability detection via boolean = anti-pattern; use default methods instead).
