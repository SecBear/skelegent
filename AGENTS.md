# AGENTS.md

Entrypoint for any coding agent working in this repo.

`CLAUDE.md` is a symlink to this file. Both point to the same content.

## What This Project Is

Skelegent is a Rust workspace implementing a 6-layer composable agentic AI runtime.
Layer 0 defines the stability contract (protocol traits, wire types). Layers 1–5
build implementations on top. Every concern — from provider serialization to secret
management — lives in exactly one crate.

Core values (in priority order): composability over convenience, declaration separated
from execution, slim defaults with opt-in complexity. See `ARCHITECTURE.md` for full
rationale.

## Key Abstractions

You must understand these 6 types to work in this codebase:

| Type | Crate | Role |
|------|-------|------|
| `Operator` | layer0 | Object-safe trait. `execute(input, ctx) -> Result<OperatorOutput, OperatorError>`. The unit of agent behavior. |
| `DispatchContext` | layer0 | Execution metadata threaded through every boundary: dispatch ID, trace context, auth, typed extensions. Every operator, tool, and middleware receives this. |
| `Context` | skg-context-engine | Mutable conversation substrate: messages, rules, metrics, effects. All mutations go through `ctx.run(op)` which fires rules. Effects are declared here via `push_effect()` / `extend_effects()` and drained into `OperatorOutput::effects`. |
| `Effect` | layer0 | Declarative side-effects (Delegate, Handoff, Log, WriteMemory, etc.). Operators declare intent; orchestrators and environments execute. |
| `Provider` | skg-turn | NOT object-safe. Generic `<P: Provider>` everywhere, erased at the `Operator` boundary. Wraps LLM inference (Anthropic, OpenAI, Ollama, etc.). |
| `Dispatcher` | layer0 | Invokes operators by ID. The orchestration boundary. |

### How they connect

```
User message
  → Dispatcher.dispatch(ctx: &DispatchContext, input)
    → Operator.execute(input, &DispatchContext)
      → react_loop(Context, Provider, Tools, &DispatchContext, config)
        → Context.run(ops) fires Rules
        → Provider.infer(request) → response
        → Tools execute with DispatchContext
        → Context.push_effect(Effect) declares side-effects
      → OperatorOutput { content, exit_reason, effects, metadata }
    → EffectHandler processes declared effects
```

## Where to Make Changes

| Task | Where |
|------|-------|
| New protocol trait or wire type | `layer0/` |
| Change operator behavior (react loop, boundaries, rules) | `op/skg-context-engine/` |
| New simple operator | `op/skg-op-single-shot/` or new `op/` crate |
| New LLM provider | new `provider/skg-provider-*` crate implementing `Provider` |
| New effect variant | `layer0/src/effect.rs` (enum) + `effects/skg-effects-local/` (handler) |
| New middleware | layer0 defines the trait; impl goes in the relevant crate |
| New state backend | new `state/` crate implementing `StateStore` |
| New environment | new `env/` crate implementing `Environment` |
| Tool infrastructure | `turn/skg-tool/` (trait, registry) or `turn/skg-mcp/` (MCP bridge) |
| Orchestration patterns | `orch/skg-orch-kit/` (utilities) or `orch/skg-orch-local/` (local impl) |
| Durable run primitives | `orch/skg-run-core/` |
| Auth/secrets/crypto | `auth/`, `secret/`, `crypto/` |
| Hooks (lifecycle governance) | `hooks/skg-hook-*` |
| The umbrella crate | `skelegent/` |

## Where Truth Lives

| What | Where |
|------|-------|
| Architectural positions | `ARCHITECTURE.md` |
| Behavioral requirements | `specs/` (indexed by `SPECS.md`) |
| Operational constraints | `rules/` |
| Deep rationale | `docs/` |
| Crate map and key concepts | `llms.txt` |

Authority: ARCHITECTURE.md > specs > rules > agent judgment.
If specs are ambiguous, update the specs (do not invent behavior).

## Load Order

Before implementation work, load in order:

1. This file
2. `ARCHITECTURE.md`
3. `SPECS.md` then the specific spec(s) for your task
4. The relevant `rules/`

## Verification

This repo uses Nix-provided Rust tooling. All must pass before any commit:

```bash
nix develop -c nix fmt
nix develop -c cargo test --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
```

Or run all of the above via `./scripts/verify.sh`.

For layer0 test-utils: `nix develop -c cargo test --features test-utils -p layer0`

Do not claim "done" without fresh evidence from the relevant commands.

## Patterns to Know

**Effects are on Context, not a separate parameter.** Operators declare effects via
`ctx.push_effect(effect)` during execution. These are drained into
`OperatorOutput::effects` by `make_output()`. There is no `EffectEmitter` parameter
on `Operator::execute`. (`EffectEmitter` still exists for dispatch-channel wiring
of progress/artifact events, but operators never receive it.)

**CognitiveOperator is a thin adapter.** It wraps `react_loop()` as a `dyn Operator`.
It forwards the `DispatchContext` it receives — it does not fabricate one.
Its config is `ReactLoopConfig`.

**Context mutations fire rules.** Every `ctx.run(op)` call checks registered rules
(Before, After, When triggers). BudgetGuard, compaction, and overwatch agents are
all implemented as rules.

**Provider is generic, Operator is object-safe.** `react_loop<P: Provider>` is generic
over the provider. The object-safe boundary is `Operator`, which erases the provider
type. This is by design — see ARCHITECTURE.md §"The Object-Safety Decision."

## Codifying Learnings

When a failure mode repeats:

1. Fix the immediate issue.
2. Encode: behavior requirement → spec in `specs/`. Process constraint → rule in `rules/`.

## Rules Index

Rules in `rules/` are numbered by concern area. Gaps in numbering are intentional —
numbers reserve space for future rules in their domain. Currently defined:
`01` (scope), `02` (verification), `04` (TDD), `06` (worktrees), `07` (commits),
`08` (review), `11` (protocol philosophy).
