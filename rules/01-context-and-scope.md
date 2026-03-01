# Context And Scope

## One Task Per Context Window

Treat a context window as a fixed-size allocation. Once mixed, it cannot be cleanly "freed"
without starting a new session.

Rules:

1. One task per session. If you switch tasks, restart.
2. Keep the "load stack" stable: re-load `AGENTS.md`, the relevant specs, and the relevant rules.
3. If outputs get worse over time (autoregressive failure), stop and restart with a smaller context.

## What Counts As A Task

A task is any unit of work with a coherent success condition.

Examples:

1. "Add `neuron` umbrella crate"
2. "Implement the `neuron-orch-local` orchestrator"
3. "Harden coverage for hook edge cases"

Not tasks (too large, must be split):

1. "Make Neuron production-ready"
2. "Write all docs"

## Restart Protocol

When drift is detected:

1. Stop implementing.
2. Write down the current objective in one sentence.
3. Start a fresh session and re-load the required stack.
4. Re-run the backpressure command that proves correctness for the current objective.

