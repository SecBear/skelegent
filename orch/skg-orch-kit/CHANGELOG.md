# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-orch-kit-v0.4.0...skg-orch-kit-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* **layer0:** Orchestrator now requires Dispatcher supertrait. Capabilities struct removed. execute() reverted to single-param.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* dispatcher-backed tool execution in context engine (PER-162) ([a3c343d](https://github.com/SecBear/skelegent/commit/a3c343d9cd5cf3e58fc5e1ae8380c2a82b460011))
* dispatcher-first by default in CognitiveOperator (PER-159) ([98d7a09](https://github.com/SecBear/skelegent/commit/98d7a09933c511b5db73bd5d2b155e4cbe3ba0ad))
* Dispatcher::dispatch takes &DispatchContext instead of &OperatorId ([5858d49](https://github.com/SecBear/skelegent/commit/5858d4925817b9f7b81624a76fd5334f872daca6))
* **layer0, context-engine:** add EffectEmitter and CognitiveOperator ([3cb67af](https://github.com/SecBear/skelegent/commit/3cb67afa425339deec5e71dbcf983809f1cfc75c))
* **layer0:** Orchestrator extends Dispatcher — one invocation primitive ([4eafc11](https://github.com/SecBear/skelegent/commit/4eafc110dd6e798665c2c420f1e83037b24a7017))
* memory tools + EffectLog wiring + output_schema on ToolDyn ([02ead0f](https://github.com/SecBear/skelegent/commit/02ead0f60915666e750582efd9e6b2eaf2ed3e85))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* **PER-164,PER-31,PER-193:** v2 fitness checks, compaction MCP server, declarative system design ([35fc486](https://github.com/SecBear/skelegent/commit/35fc4864e8347367706b1df7e35f067b4acd2b79))
* streaming dispatch as layer0 primitive ([6117665](https://github.com/SecBear/skelegent/commit/6117665f586110bdcac64c438e63bf7631860361))
* unify effects — single emission path via EffectEmitter ([b42bd36](https://github.com/SecBear/skelegent/commit/b42bd36d95a8841a8b20342819b163b2811e09d4))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))


### Bug Fixes

* **context-engine:** reconcile durable orch merge ([d1f9a4c](https://github.com/SecBear/skelegent/commit/d1f9a4cfb8c89bda7fe47b6d67fd3301af03b91a))
* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* resolve all P1 + P2 audit findings ([b255033](https://github.com/SecBear/skelegent/commit/b255033a1bba35ee9bd4541e45b08f3fdf44a4b9))
* update all Operator::execute call sites for Capabilities parameter ([dc761cc](https://github.com/SecBear/skelegent/commit/dc761cccc2014d63e362522dd84769c0458f6dc4))
