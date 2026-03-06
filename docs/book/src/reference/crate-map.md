# Crate Map

All crates in the neuron workspace, organized by architectural layer.

## Layer 0 -- Protocol Traits

| Crate | Description |
|-------|-------------|
| `layer0` | Protocol traits (`Operator`, `Orchestrator`, `StateStore`, `Environment`, `Hook`), message types, and error types. The stability contract. |

## Layer 1 -- Operator Implementations

| Crate | Description |
|-------|-------------|
| `neuron-turn` | Shared toolkit: `Provider` trait, `ContextStrategy`, provider request/response types, content conversions. |
| `neuron-provider-anthropic` | Anthropic Claude API provider. Implements `Provider` for the Messages API. |
| `neuron-provider-openai` | OpenAI API provider. Implements `Provider` for the Chat Completions API. |
| `neuron-provider-ollama` | Ollama local model provider. Implements `Provider` for the Ollama API. |
| `neuron-tool` | `ToolDyn` trait, `ToolRegistry`, `AliasedTool`. Object-safe tool abstraction. |
| `neuron-context` | Conversation context assembly and compaction strategies. |
| `neuron-mcp` | MCP (Model Context Protocol) client. Wraps MCP server tools as `ToolDyn` implementations. |
| `neuron-op-react` | ReAct operator. Implements `Operator` with the reason-act-observe loop and tool execution. |
| `neuron-op-single-shot` | Single-shot operator. Implements `Operator` with one model call and no tools. |
| `neuron-turn-kit` | Turn engine primitives: `DispatchPlanner`, `ConcurrencyDecider`, `BatchExecutor` (execution-only), `SteeringSource`. |

## Layer 2 -- Orchestration

| Crate | Description |
|-------|-------------|
| `neuron-orch-local` | In-process orchestrator. Implements `Orchestrator` with tokio tasks. |
| `neuron-orch-kit` | Shared utilities for orchestrator implementations. |
| `neuron-effects-core` | Effect execution trait (`EffectExecutor`), errors, and policy — no implementations. |
| `neuron-effects-local` | Local in-process `EffectExecutor` implementation (in-order, best-effort). |

## Layer 3 -- State

| Crate | Description |
|-------|-------------|
| `neuron-state-memory` | In-memory state store. Implements `StateStore` with `HashMap`. Ephemeral. |
| `neuron-state-fs` | Filesystem state store. Implements `StateStore` with file-backed persistence. |

## Layer 4 -- Environment and Credentials

| Crate | Description |
|-------|-------------|
| `neuron-env-local` | Local environment. Implements `Environment` with no isolation (passthrough). |
| `neuron-secret` | Secret resolution trait. Defines the interface for secret backends. |
| `neuron-secret-vault` | HashiCorp Vault secret backend. |
| `neuron-crypto` | Cryptographic utilities and primitives. |
| `neuron-auth` | Authentication and authorization abstractions. |

## Layer 5 -- Cross-Cutting

| Crate | Description |
|-------|-------------|
| `neuron-hooks` | `HookRegistry` for ordered hook pipeline dispatch. Collects and dispatches `Hook` events. |
| `neuron-hook-security` | Security-focused hooks: guardrails, policy enforcement, secret redaction. |

## Umbrella

| Crate | Description |
|-------|-------------|
| `neuron` | Umbrella crate. Feature-gated re-exports of all layers. |


## Examples

| Crate | Description |
|-------|-------------|
| `custom-operator-barrier` | Example custom operator with barrier scheduling and steering (workspace member at `examples/custom_operator_barrier`). |
## Summary

| Layer | Crates |
|-------|--------|
| 0 | 1 |
| 1 | 10 |
| 2 | 4 |
| 3 | 2 |
| 4 | 5 |
| 5 | 2 |
| Umbrella | 1 |
| **Total** | **25** |
