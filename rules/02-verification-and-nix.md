# Verification

This repo assumes Rust tooling is provided by the Nix flake via direnv.

## Setup

Run `direnv allow` once per session. After that, `cargo`, `rustc`, `clippy`,
and `rustfmt` are available directly on PATH.

## Command Policy

Preferred:

1. `cargo test --workspace --all-targets`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `nix fmt`

If a command fails, do not guess. Read the output, find root cause, and fix it
with a test-first approach when behavior is changing.

## No Claims Without Evidence

Do not claim:

1. "Tests pass"
2. "Fixed"
3. "Done"

Unless the relevant command was run in the current session and you have the
exit status and the failure count.

## Minimal Verification Sets

1. Rust code change:
   - `cargo test --workspace --all-targets`
2. Public API / protocol change:
   - `cargo test --workspace --all-targets`
   - plus any crate-specific tests touching the boundary
3. Formatting-only change:
   - `nix fmt`
