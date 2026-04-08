# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-op-single-shot-v0.4.0...skg-op-single-shot-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* **layer0:** Orchestrator now requires Dispatcher supertrait. Capabilities struct removed. execute() reverted to single-param.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* **layer0, context-engine:** add EffectEmitter and CognitiveOperator ([3cb67af](https://github.com/SecBear/skelegent/commit/3cb67afa425339deec5e71dbcf983809f1cfc75c))
* **layer0:** Orchestrator extends Dispatcher — one invocation primitive ([4eafc11](https://github.com/SecBear/skelegent/commit/4eafc110dd6e798665c2c420f1e83037b24a7017))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* **PER-164,PER-31,PER-193:** v2 fitness checks, compaction MCP server, declarative system design ([35fc486](https://github.com/SecBear/skelegent/commit/35fc4864e8347367706b1df7e35f067b4acd2b79))
* surface reasoning_tokens in TokenUsage + update aspirational backlog ([d7456af](https://github.com/SecBear/skelegent/commit/d7456afaa2df99f382d19080cf6e495414a85828))


### Bug Fixes

* error cascade fixes (3, 5, 6, 1) ([1930688](https://github.com/SecBear/skelegent/commit/1930688c95974eac971e902b7c616cc632a7e4f6))
* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* final review cleanup — remove redundant effects vec, fix stale doc refs ([64291ab](https://github.com/SecBear/skelegent/commit/64291ab9377cd2677df578be24c5eba3bdc9679d))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))
* update all Operator::execute call sites for Capabilities parameter ([dc761cc](https://github.com/SecBear/skelegent/commit/dc761cccc2014d63e362522dd84769c0458f6dc4))
