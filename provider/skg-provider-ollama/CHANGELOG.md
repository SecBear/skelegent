# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-provider-ollama-v0.4.0...skg-provider-ollama-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* all recipe types removed from public API
* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* dispatcher-backed tool execution in context engine (PER-162) ([a3c343d](https://github.com/SecBear/skelegent/commit/a3c343d9cd5cf3e58fc5e1ae8380c2a82b460011))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* surface reasoning_tokens in TokenUsage + update aspirational backlog ([d7456af](https://github.com/SecBear/skelegent/commit/d7456afaa2df99f382d19080cf6e495414a85828))
* tool result formatting hooks + ToolSchema.extra field ([de3b0f8](https://github.com/SecBear/skelegent/commit/de3b0f817cfe2d4eedd0488d4cb3ec568de785b3))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))
* Wave 1 design roadmap implementation ([cb15f7b](https://github.com/SecBear/skelegent/commit/cb15f7b5e2d1bf88e22ebbb71ebabf5fdf1cf37b))
* Wave 2 — breaking InferRequest bundle + orchestration + builder + OpenAPI ([285a89d](https://github.com/SecBear/skelegent/commit/285a89daf0f88d394cf4dfb68f56e111e9f99570))


### Bug Fixes

* error propagation, observability, and structured error responses ([d188005](https://github.com/SecBear/skelegent/commit/d18800532a051a9377be3da1def15de1b3dc733c))
* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))


### Code Refactoring

* delete recipe ops, skg-context crate, and compaction server ([73ec98a](https://github.com/SecBear/skelegent/commit/73ec98a46ef87f802555bbab4a318abc493e3b7a))
