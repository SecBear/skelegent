# Architecture and Layering

## Canonical Layers

Skelegent uses six conceptual layers. These are governance and dependency boundaries, not a claim about physical deployment.

- Layer 0: Protocol contract (`layer0`) — traits, middleware surfaces, and message-level hints
- Layer 1: Turn implementations (operator runtimes + providers + tools + context)
- Layer 2: Orchestration implementations (composition, immediate dispatch, and durable run/control above Layer 0)
- Layer 3: State implementations (persistence backends)
- Layer 4: Environment implementations (isolation + credentials + resource/network)
- Layer 5: Cross-cutting governance (hooks + observation/intervention + lifecycle coordination)

## Runtime Mental Model (Teaching Order)

At runtime, it’s often clearer to think in this order:

1. A single operator cycle runs (`Operator::execute`).
2. Orchestrator implementations call `Environment::run(operator, input)` to execute that cycle within an isolation boundary (credentials, resource limits, sandboxing).
3. It reads/writes state via declared effects (outer execution decides when/how).
4. Orchestration coordinates many cycles (routing, signals, retries, durable run/control).
5. Hooks/lifecycle above Layer 0 provide intervention and cross-layer coordination vocabulary.

This teaching order must not contradict canonical layer numbering.

`Environment::run()` is called by **orchestrator implementations** (Layer 2), not by operators or composition code directly. Operators receive dispatch capability via `Arc<dyn Dispatcher>` injected at construction time; the orchestrator mediates the environment boundary transparently.

## Dependency Rules

- `layer0` must remain minimal and stable.
- Implementation crates can depend on `layer0`.
- Higher layers must not force lower layers to depend on them.

## Where Composition/Glue Belongs

Composition/glue is an opinionated layer above the protocol contract.

- The *protocol* should not encode product-level routing policy.
- Composition factories should live with orchestrator implementations (Layer 2) because they define wiring/topology and are inherently orchestration concerns.
- A separate wrapper product can exist (outside this workspace) to add YAML workflow DSLs, Slack delivery, etc. That wrapper depends on Skelegent; it does not belong inside `layer0`.

