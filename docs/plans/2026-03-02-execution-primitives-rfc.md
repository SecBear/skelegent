# RFC: Execution Primitives for Rho‑Class Operators (Planner, Steering, Streaming, Effects)

## Summary
We just added three opt‑in capabilities to support Rho‑like systems:
- ToolExecutionPlanner + ConcurrencyDecider (with BarrierPlanner)
- SteeringSource (mid‑loop message injection with batch skip semantics)
- Streaming tools + ToolExecutionUpdate hook (read‑only)
And we added a LocalEffectExecutor under neuron‑orch‑kit to execute Effects deterministically.

This RFC proposes where these concepts live long‑term, why they are runtime concerns (not LLM concerns), and the composable traits we should expose so React and other operators can share a common "turn engine" without monolithic loops.

## Effect execution: placement and contract
Effects are the operator’s declared side‑effects; outer layers decide when and how to execute them.

Options:
- A) Keep executor in neuron‑orch‑kit (current): provides a reference in‑process interpreter for local orchestration. Pros: simple, keeps orchestration ownership. Cons: trait lives in kit; reuse by other orchestrators may depend on kit.
- B) Split into dedicated crates: `neuron-effects-core` (trait + error + policy) and `neuron-effects-local` (in‑process impl). Orchestrators import the core trait and provide/choose implementations. Pros: cleaner separation, easier to add durable/remote executors. Cons: a new crate split and migration.
- C) Move executor trait into layer0 (protocol): Reject. It would over-constrain implementations; execution is a policy/technology choice, not a protocol obligation.

Recommendation: Adopt B. Create `neuron-effects-core` (L2) with the `EffectExecutor` trait and policy; move the current interpreter into `neuron-effects-local`. Re‑export from `neuron-orch-kit` for back‑compat in the short term.


### Effect idempotency and durability
- Keep `LocalEffectExecutor` lean: in-order, best-effort application, no idempotency keys or saga semantics.
- Durable execution (idempotency keys, retries, compensation) belongs in a future `DurableEffectExecutor` backed by a workflow engine (e.g., Temporal), not in the core trait or local impl.
## Planning and scheduling: why the runtime owns it
The LLM picks tools and their order; the runtime must enforce safety and constraints:
- Shared vs Exclusive tools (global locks, side‑effects, resource contention)
- Concurrency and batching under budget/time limits
- Steering interrupts and control over where to pause/replan

ToolExecutionPlanner is a runtime policy: how to schedule the model’s requested tool calls inside the turn. The LLM cannot reliably enforce concurrency guarantees or guard rails; the runtime must.

Recommendation:
- Keep `ToolExecutionPlanner` and `ConcurrencyDecider` as composable primitives and make them available outside `neuron-op-react`.
- Add a small crate `neuron-turn-kit` (or extend `neuron-turn`) to host these traits and common strategies (BarrierPlanner), so other operators can reuse them without importing op-react.
- Add a `BatchExecutor` trait (separate from planning) so batch execution (parallel shared + hook semantics + streaming) can be reused across operators.

## Steering and streaming: composable, read-only taps
- SteeringSource is an explicit, opt‑in source of mid-loop messages. Poll boundaries are part of the engine contract; skipping preserves tool_use→tool_result pairing with placeholders. Keep this out of hooks.
- Streaming is a pure observation channel: `ToolDynStreaming` emits chunks; the operator forwards them via `HookPoint::ToolExecutionUpdate` (read-only). Keep hook actions ignored here.

Recommendation:

### Steering vs Hooks boundary
- Hooks are event-triggered observation/intervention at defined points (pre/post inference/tool, exit). Steering is operator-initiated control flow: the runtime decides when to poll and may skip remaining tools.
- Both can inject content mid-loop, but for different reasons: hooks may `ModifyToolInput/Output` or `Halt` at a point; steering injects new messages and restarts the loop. Keep steering out of hooks to avoid overloading hook semantics.

### BatchExecutor scope
- To avoid scope creep, keep `BatchExecutor` focused on concurrent execution. Streaming forwarding and hook dispatch remain separate, composable concerns wired by the operator/engine. Provide helper adapters in `neuron-turn-kit`, but do not bake hooks/streaming into the executor trait.
- Keep both as engine primitives. Add light adapters in `neuron-turn-kit` so non-React operators can adopt them.

## Operator composability: turn engine approach
Today ReactOperator wires: context assembly → provider call → planning → execution → hooks → effects → exit.
To avoid monolithic loops, define an internal "turn engine" via traits:
- `ContextAssembler`
- `Planner` (ToolExecutionPlanner) + `ConcurrencyDecider`
- `BatchExecutor` (streaming-aware) + `SteeringSource`
- `HookDispatcher` (already in neuron-hooks)
- `EffectSynthesizer` (maps tool calls to Effects, e.g., write_memory)
- `ExitController` (budget/turn/time limits)

React becomes a thin composition of these pieces; other operator styles (e.g., planner‑first, tool‑first, or chain‑of‑thought variants) can reuse the same engine parts.

## Naming, minimalism, and defaults
- Keep defaults slim: sequential planner; no steering; no streaming; local effects optional.
- Expose strategies (BarrierPlanner) as library code, not hardwired behaviors.
- Make the new crates additive; re-export from umbrella where helpful.

## Migration plan
1) Split effects into `neuron-effects-core` + `neuron-effects-local`; re-export from `neuron-orch-kit` for back‑compat.
2) Create `neuron-turn-kit` (or extend `neuron-turn`) with `ToolExecutionPlanner`, `ConcurrencyDecider`, `BatchExecutor`, `SteeringSource` traits and baseline implementations (BarrierPlanner).
3) Gradually refactor `neuron-op-react` to depend on `neuron-turn-kit` instead of keeping these traits internally. Keep public builders (`with_planner`, `with_concurrency_decider`, `with_steering`).
4) Add examples: swapping planners and executors without touching operator logic.

## Closed questions (resolved)
- Budget/time aware planning: keep limits in `ExitController`. Give the planner read-only access to remaining budget/time for informative decisions, but do not let it decide exits (single authority prevents divergence).
- Effect idempotency: keep local executor lean; durable semantics belong to a durable executor at orchestration layer.
- Tool concurrency metadata: formalize Shared/Exclusive at tool registration (metadata on the tool definition). The `ConcurrencyDecider` reads this first, and may layer policy overrides.
## Acceptance for this RFC
- Green-light crate splits and create tracking tasks in `ralph_queue.md`.
- Keep defaults unchanged; all new pieces are opt-in and additive.
- For step 3 (op-react refactor to turn-kit), add a hard constraint: the refactor MUST pass the existing test suite with zero behavioral changes before adding any new capabilities through the decomposed traits.
