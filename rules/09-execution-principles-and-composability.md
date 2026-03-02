# Rule 09 — Execution Principles and Composability

## Rationale
Neuron’s value is composability. We avoid monoliths by drawing firm boundaries between protocol, runtime policy, and technology choices.

## MUST / MUST NOT
- Layer 0 MUST stay protocol-only (object-safe traits + serde wire types). Execution policy and durability MUST NOT live in Layer 0.
- Operators MUST declare Effects; orchestrators/environments MUST execute them. Operators MUST NOT write state directly.
- Steering MUST be operator-initiated control flow and MUST NOT be implemented via hooks.
- Hooks MUST remain event-triggered observation/intervention with explicit actions; read-only streaming updates MUST NOT change control flow.
- Defaults MUST remain slim: sequential tools, no steering, no streaming, local best-effort effects. Advanced behavior MUST be opt-in via small traited components (no boolean soup).
- Turn engines SHOULD compose primitives (ContextAssembler, ToolExecutionPlanner, ConcurrencyDecider, BatchExecutor, SteeringSource, HookDispatcher, EffectSynthesizer, ExitController). Monolithic loops SHOULD be refactored.
- Tool concurrency metadata SHOULD live on the tool definition; ConcurrencyDecider SHOULD prefer metadata and MAY layer policy.
- ExitController MUST own budget/time/turn limits. Planners MAY read remaining budget/time but MUST NOT decide exits.
- LocalEffectExecutor MUST be lean (in-order, best-effort). Durable semantics (idempotency keys, retries, sagas) MUST live in durable orchestrators.
- The tool_use → tool_result pairing MUST be preserved. On steering, placeholders MUST be emitted for skipped tools.

## Process
- Behavior-preserving refactors MUST pass the full test suite before adding new capabilities via decomposed traits.
- New execution features MUST ship as traited, opt-in components and include targeted tests and docs.
- Composition conformance MUST be maintained with golden tests for: provider swap, state swap, operator swap, orchestration behaviors.

## Anti-patterns
- Encoding scheduling, steering, or effect execution inside hooks.
- Pushing execution/durability into Layer 0.
- Adding feature flags for complex behavior where a pluggable strategy/trait would suffice.
- Splitting exit authority across multiple components (e.g., planner rejecting work instead of ExitController enforcing limits).

## Examples
- Good: BarrierPlanner implements ToolExecutionPlanner; ReactOperator accepts it via with_planner(); defaults remain sequential.
- Good: SteeringSource drains messages at defined boundaries; hooks remain unchanged.
- Good: ToolDynStreaming emits chunks; operator forwards via ToolExecutionUpdate (read-only); control flow unchanged.
- Good: LocalEffectExecutor applies Write/Delete/Delegate/Handoff/Signal in order; durable executor deferred.
- Bad: A hook that injects steering messages or reorders tool execution.
- Bad: Adding idempotency keys to Layer 0 or LocalEffectExecutor.
