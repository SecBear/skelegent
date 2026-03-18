# Crate Map

All crates in the skelegent workspace, organized by architectural layer.

## Layer 0 -- Protocol Traits

| Crate | Description |
|-------|-------------|
| `layer0` | Protocol traits (`Operator`, `Dispatcher`, `StateStore`, `Environment`), middleware traits (`DispatchMiddleware`, `StoreMiddleware`, `ExecMiddleware`), message types, and error types. The stability contract. |

## Layer 1 -- Operator Implementations

| Crate | Description |
|-------|-------------|
| `skg-turn` | Shared toolkit: `Provider` trait, `InferRequest`, `InferResponse`, `TokenUsage`, provider request/response types, content conversions. |
| `skg-provider-anthropic` | Anthropic Claude API provider. Implements `Provider` for the Messages API. |
| `skg-provider-openai` | OpenAI API provider. Implements `Provider` for the Chat Completions API. |
| `skg-provider-ollama` | Ollama local model provider. Implements `Provider` for the Ollama API. |
| `skg-provider-codex` | OpenAI Codex (Responses API) provider. Implements `Provider` for the Responses API. |
| `skg-provider-router` | Provider router. Selects an underlying provider per request using pluggable routing policy. |
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
| `skg-orch-local` | In-process orchestrator. Implements `Dispatcher` (layer0), `Signalable`, and `Queryable` (skg-effects-core) with tokio tasks. |
| `skg-orch-kit` | Shared utilities for orchestrator implementations. |
| `skg-orch-env` | Environment-aware orchestrator. Routes operators through `Environment::run`. |
| `skg-run-core` | Portable durable run/control primitives and kernel above Layer 0. |
| `skg-effects-core` | Effect handler trait (`EffectHandler`), `Signalable`, `Queryable`, errors, and policy — no implementations. |
| `skg-effects-local` | Local in-process `EffectHandler` implementation (in-order, best-effort). |
| `skg-runner` | Runner binary for containerized/operator-hosted execution with gRPC + healthcheck endpoints. |

## Layer 3 -- State

| Crate | Description |
|-------|-------------|
| `skg-state-memory` | In-memory state store. Implements `StateStore` with `HashMap`. Ephemeral. |
| `skg-state-fs` | Filesystem state store. Implements `StateStore` with file-backed persistence. |
| `skg-state-proxy` | gRPC proxy for `StateStore`, enabling cross-container state access. |

## Layer 4 -- Environment and Credentials

| Crate | Description |
|-------|-------------|
| `skg-env-local` | Local environment. Implements `Environment` with no isolation (passthrough). |
| `skg-env-docker` | Docker-backed environment implementation for isolated operator execution. |
| `skg-secret` | Secret resolution trait. Defines the interface for secret backends. |
| `skg-secret-vault` | HashiCorp Vault secret backend. |
| `skg-crypto` | Cryptographic utilities and primitives. |
| `skg-auth` | Authentication and authorization abstractions. |
| `skg-auth-omp` | OMP credential provider that reads Oh My Pi OAuth tokens from `agent.db`. |

## Layer 5 -- Cross-Cutting

| Crate | Description |
|-------|-------------|
| `skg-hook-security` | Security middleware: `RedactionMiddleware` (pattern-based content redaction) and `ExfilGuardMiddleware` (data-loss-prevention guardrails). |
| `skg-hook-recorder` | Universal operation recorder middleware. Captures dispatch events for testing and debugging. |
| `skg-hook-retry` | Retry middleware with configurable backoff and deadline-aware dispatch retries. |

## Umbrella

| Crate | Description |
|-------|-------------|
| `skelegent` | Umbrella crate. Feature-gated re-exports of all layers. |

## A2A

| Crate | Description |
|-------|-------------|
| `skg-a2a-core` | A2A protocol wire types and conversions. |


## Examples

| Crate | Description |
|-------|-------------|
| `custom-operator-barrier` | Example custom operator with barrier scheduling and steering (workspace member at `examples/custom_operator_barrier`). |
| `hello-claude` | Minimal example binary that wires OMP auth plus a single-shot Claude operator. |
| `middleware_approval` | Example demonstrating approval-gated middleware. |
| `middleware_echo` | Example demonstrating echo middleware. |
| `middleware_recorder` | Example demonstrating recorder middleware. |
## Summary

| Layer | Crates |
|-------|--------|
| 0 | 1 |
| 1 | 13 |
| 2 | 7 |
| 3 | 3 |
| 4 | 7 |
| 5 | 3 |
| Umbrella | 1 |
| A2A | 1 |
| Examples | 5 |
| **Total** | **41** |
