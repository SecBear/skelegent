# Skelegent OMP Entrypoint

This file exists so OMP can discover repo-local guidance without depending on the author's global harness.

`../AGENTS.md` is the authoritative repo entrypoint. Do not maintain a second rule plane here.

## Load Order
1. `../AGENTS.md`
2. `../ARCHITECTURE.md`
3. `../SPECS.md`, then the task-specific spec(s) under `../specs/`
4. The relevant numbered rule files under `../rules/`

## Authority
`ARCHITECTURE.md` > `specs/` > `rules/` > agent judgment.

## Working Notes
- Use this file as a map only. Put durable policy in `../AGENTS.md`, `../ARCHITECTURE.md`, `../specs/`, or `../rules/`.
- `../AGENTS.md` already maps task types to owning crates and verification commands.
- For meaningful code changes, verify with the targeted Nix commands from `../AGENTS.md` or `../scripts/verify.sh`.
