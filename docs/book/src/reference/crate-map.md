# Crate Map

All crates in the skelegent workspace, organized by architectural layer.

## Layer 0 -- Protocol Traits

| Crate | Description |
|-------|-------------|
| `layer0` | Protocol traits (`Operator`, `Orchestrator`, `StateStore`, `Environment`), middleware traits (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`), message types, and error types. The stability contract. |

## Layer 1 -- Operator Implementations

| Crate | Description |
|-------|-------------|
| `skg-turn` | Shared toolkit: `Provider` trait, `InferRequest`, `InferResponse`, `TokenUsage`, provider request/response types, content conversions. |
| `skg-provider-anthropic` | Anthropic Claude API provider. Implements `Provider` for the Messages API. |
| `skg-provider-openai` | OpenAI API provider. Implements `Provider` for the Chat Completions API. |
| `skg-provider-ollama` | Ollama local model provider. Implements `Provider` for the Ollama API. |
| `skg-tool` | `ToolDyn` trait, `ToolRegistry`, `AliasedTool`. Object-safe tool abstraction. |
| `skg-context` | Conversation context assembly and compaction strategies. |
| `skg-mcp` | MCP (Model Context Protocol) client. Wraps MCP server tools as `ToolDyn` implementations. |
| `skg-context-engine` | Composable three-phase context engine (assembly, inference, reaction). Implements `Operator` with tool execution. |
| `skg-tool-macro` | Proc macro for `#[skg_tool]` attribute. Generates `ToolDyn` implementations from async functions. |
| `skg-op-single-shot` | Single-shot operator. Implements `Operator` with one model call and no tools. |
| `skg-turn-kit` | Turn engine primitives: `DispatchPlanner`, `ConcurrencyDecider`, `BatchExecutor` (execution-only), `SteeringSource`. |

## Layer 2 -- Orchestration

| Crate | Description |
|-------|-------------|
| `skg-orch-local` | In-process orchestrator. Implements `Orchestrator` with tokio tasks. |
| `skg-orch-kit` | Shared utilities for orchestrator implementations. |
| `skg-effects-core` | Effect execution trait (`EffectExecutor`), errors, and policy — no implementations. |
| `skg-effects-local` | Local in-process `EffectExecutor` implementation (in-order, best-effort). |

## Layer 3 -- State

| Crate | Description |
|-------|-------------|
| `skg-state-memory` | In-memory state store. Implements `StateStore` with `HashMap`. Ephemeral. |
| `skg-state-fs` | Filesystem state store. Implements `StateStore` with file-backed persistence. |

## Layer 4 -- Environment and Credentials

| Crate | Description |
|-------|-------------|
| `skg-env-local` | Local environment. Implements `Environment` with no isolation (passthrough). |
| `skg-secret` | Secret resolution trait. Defines the interface for secret backends. |
| `skg-secret-vault` | HashiCorp Vault secret backend. |
| `skg-crypto` | Cryptographic utilities and primitives. |
| `skg-auth` | Authentication and authorization abstractions. |

## Layer 5 -- Cross-Cutting

| Crate | Description |
|-------|-------------|
| `skg-hook-security` | Security middleware: `RedactionMiddleware` (pattern-based content redaction) and `ExfilGuardMiddleware` (data-loss-prevention guardrails). |

## Umbrella

| Crate | Description |
|-------|-------------|
| `skelegent` | Umbrella crate. Feature-gated re-exports of all layers. |


## Examples

| Crate | Description |
|-------|-------------|
| `custom-operator-barrier` | Example custom operator with barrier scheduling and steering (workspace member at `examples/custom_operator_barrier`). |
## Summary

| Layer | Crates |
|-------|--------|
| 0 | 1 |
| 1 | 11 |
| 2 | 4 |
| 3 | 2 |
| 4 | 5 |
| 5 | 1 |
| Umbrella | 1 |
| **Total** | **25** |
