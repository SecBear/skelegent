# Composition Factory and Glue

## The Question

Where does “glue” live?

- Inside orchestrator implementations?
- As a wrapper around Neuron?

## Answer (Specification)

Composition glue that wires agents, policies, and topology belongs with orchestration implementations (Layer 2), not in `layer0`.

Reason:

- It is inherently an orchestration concern (it chooses routing/topology/policy).
- It must be shared by examples and tests to prevent drift.
- It should remain optional. `layer0` must not become a product DSL.

A separate wrapper product (outside Neuron) can exist to provide:

- YAML workflow DSL
- Slack/email delivery
- long-running job scheduling UX

That wrapper depends on Neuron and uses the composition factories.

## Required APIs

Neuron core should provide an *unopinionated wiring kit* plus (optionally) a small set of reference factories.

### `neuron-orch-kit` (Recommended)

Define a crate named `neuron-orch-kit` as the “boring glue” layer that a product like Sortie would build on.

`neuron-orch-kit` MUST:

- remain a Rust API (no workflow DSL)
- allow registering arbitrary agents/operators (not just preset flows)
- support swapping implementations (mock vs real; local vs distributed) via explicit selectors
- expose a pluggable effect runner/interpreter policy for:
  - `WriteMemory` / `DeleteMemory`
  - `Delegate`
  - `Handoff`
  - `Signal`
- allow bypassing any defaults (zero lock-in)

`neuron-orch-kit` MUST NOT:

- require a fixed topology enum as the only composition mechanism
- silently fall back when routing/policy inputs are unknown
- hardcode delivery integrations (Slack/webhooks/email)
- hardcode a particular durable engine (Temporal/Restate/etc.)

### Reference Factories (Optional)

Neuron may also provide a small set of *reference* factory entrypoints that:

- accept a declarative spec (flow/topology + runtime profile)
- return a runnable orchestrator graph
- support mock and real profiles

These are allowed to be opinionated, but they must be clearly labeled as reference flows and must be bypassable.

## Sortie Integration Rule

If writing Sortie from scratch, Sortie SHOULD depend on `neuron-orch-kit`.

If `neuron-orch-kit` becomes constraining (e.g., it encodes product-level policy or freezes topology), Sortie SHOULD bypass it and wire directly against `layer0` instead. This “escape hatch” is not a failure; it is the signal that `neuron-orch-kit` needs to become less opinionated.

## Current Implementation Status

`neuron-orch-kit` exists as the unopinionated wiring kit.

Still required for “core complete”:

- end-to-end examples/tests that exercise `neuron-orch-kit` as the shared wiring layer (to prevent drift)
- a reference effect execution story that is explicitly documented and test-proven (delegate/handoff/signal/state)
