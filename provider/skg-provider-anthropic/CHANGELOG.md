# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-provider-anthropic-v0.4.0...skg-provider-anthropic-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* batch 1+2 consolidation ([edf5ae2](https://github.com/SecBear/skelegent/commit/edf5ae2bbe5dbb1d6f744d4463a1ccf1e96d63cb))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* streaming dispatch as layer0 primitive ([6117665](https://github.com/SecBear/skelegent/commit/6117665f586110bdcac64c438e63bf7631860361))
* surface reasoning_tokens in TokenUsage + update aspirational backlog ([d7456af](https://github.com/SecBear/skelegent/commit/d7456afaa2df99f382d19080cf6e495414a85828))
* tool result formatting hooks + ToolSchema.extra field ([de3b0f8](https://github.com/SecBear/skelegent/commit/de3b0f817cfe2d4eedd0488d4cb3ec568de785b3))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))
* Wave 1 design roadmap implementation ([cb15f7b](https://github.com/SecBear/skelegent/commit/cb15f7b5e2d1bf88e22ebbb71ebabf5fdf1cf37b))
* Wave 2 — breaking InferRequest bundle + orchestration + builder + OpenAPI ([285a89d](https://github.com/SecBear/skelegent/commit/285a89daf0f88d394cf4dfb68f56e111e9f99570))


### Bug Fixes

* error propagation, observability, and structured error responses ([d188005](https://github.com/SecBear/skelegent/commit/d18800532a051a9377be3da1def15de1b3dc733c))
* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* **provider-anthropic:** add Claude Code identity headers for subscription OAuth ([0a7dfa9](https://github.com/SecBear/skelegent/commit/0a7dfa922a2da4fb0d1a4679fc6deba977c06813))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))
