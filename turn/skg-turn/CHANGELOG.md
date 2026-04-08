# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-turn-v0.4.0...skg-turn-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* move DynProvider to skg-turn with blanket impl
* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* add builder methods, Display impls, InferResponse conversions, and re-exports for API ergonomics ([c76ab46](https://github.com/SecBear/skelegent/commit/c76ab46c9d8cd530e9394d40c28724afeaf49119))
* dispatcher-backed tool execution in context engine (PER-162) ([a3c343d](https://github.com/SecBear/skelegent/commit/a3c343d9cd5cf3e58fc5e1ae8380c2a82b460011))
* EffectLog + EffectMiddleware + FromContext extractors + TokenCounter ([4c4bf44](https://github.com/SecBear/skelegent/commit/4c4bf44e855bd299dcdccd587c872586dd9cf691))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* Provider::embed(), Checkpoint primitive, embedding types ([dd6e2bb](https://github.com/SecBear/skelegent/commit/dd6e2bb03b27e04f4011f65ce680cea2deda700b))
* surface reasoning_tokens in TokenUsage + update aspirational backlog ([d7456af](https://github.com/SecBear/skelegent/commit/d7456afaa2df99f382d19080cf6e495414a85828))
* tool result formatting hooks + ToolSchema.extra field ([de3b0f8](https://github.com/SecBear/skelegent/commit/de3b0f817cfe2d4eedd0488d4cb3ec568de785b3))
* universal middleware — InferMiddleware, recorder, MiddlewareProvider ([08a015f](https://github.com/SecBear/skelegent/commit/08a015f88f2b7fedf480faeff5c8b73e709d00b8))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))
* Wave 1 batch 2 — handoff detection, wire protocol, builder, test utils, schema tools ([9e7f9f5](https://github.com/SecBear/skelegent/commit/9e7f9f53584598ebd00a4ea473de0e8069297214))
* Wave 1 design roadmap implementation ([cb15f7b](https://github.com/SecBear/skelegent/commit/cb15f7b5e2d1bf88e22ebbb71ebabf5fdf1cf37b))
* Wave 2 — breaking InferRequest bundle + orchestration + builder + OpenAPI ([285a89d](https://github.com/SecBear/skelegent/commit/285a89daf0f88d394cf4dfb68f56e111e9f99570))


### Bug Fixes

* error propagation, observability, and structured error responses ([d188005](https://github.com/SecBear/skelegent/commit/d18800532a051a9377be3da1def15de1b3dc733c))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))


### Code Refactoring

* move DynProvider to skg-turn with blanket impl ([b2d72c1](https://github.com/SecBear/skelegent/commit/b2d72c16065f36e9aea8cd1b6b746f3a5079de95))
