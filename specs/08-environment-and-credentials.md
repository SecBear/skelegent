# Environment and Credentials

## Purpose

Environment defines how operator work runs under isolation and credential constraints.

This is where “local vs docker vs k8s” lives. The protocol should not change across these.

## Protocol

`layer0::Environment` defines:

- `run(input, spec) -> output`

`EnvironmentSpec` defines:

- isolation boundaries
- credential references + injection strategy
- resource limits
- network policy

## Credentials Integration

Credential *delivery* is an environment concern (env var, mounted file, sidecar).

Credential *source backend* is a secret/auth/crypto concern.

## Credential Injection Pattern

From `ARCHITECTURE.md §Tool Execution`:

> Boundary injection preferred — credentials added at the edge, stripped from
> context. Tests must prove no secret leakage.

This means:

1. **Add at the edge**: credentials are resolved and injected at the `Environment`
   boundary, not inside the operator or turn. The operator never sees raw secrets.
2. **Strip from context**: after a tool executes with a credential, the raw credential
   value MUST NOT appear in the tool result that enters the context window.
3. **Test requirement**: every credential injection path MUST have a test that asserts
   no raw secret value appears in `OperatorOutput.message` or in any effect payload.

The `CredentialRef` in `EnvironmentSpec` names the credential; the `Environment`
implementation resolves it at execution time and injects it into the tool call without
exposing the value to the operator.

## Current Implementation Status

- `neuron-env-local` exists.

Stubs are acceptable for docker/k8s implementations right now.

Still required for “core complete”:

- documentation and tests proving that credentials are represented consistently end-to-end (source + injection)
- a reference “credential resolution + injection” pipeline for local mode (even if backend sources are stubbed)

