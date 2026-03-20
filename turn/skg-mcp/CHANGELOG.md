# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-mcp-v0.4.0...skg-mcp-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* **layer0:** Orchestrator now requires Dispatcher supertrait. Capabilities struct removed. execute() reverted to single-param.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* **layer0, context-engine:** add EffectEmitter and CognitiveOperator ([3cb67af](https://github.com/SecBear/skelegent/commit/3cb67afa425339deec5e71dbcf983809f1cfc75c))
* **layer0:** Orchestrator extends Dispatcher — one invocation primitive ([4eafc11](https://github.com/SecBear/skelegent/commit/4eafc110dd6e798665c2c420f1e83037b24a7017))
* memory tools + EffectLog wiring + output_schema on ToolDyn ([02ead0f](https://github.com/SecBear/skelegent/commit/02ead0f60915666e750582efd9e6b2eaf2ed3e85))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* thread DispatchContext through MCP tool calls and server ([15de639](https://github.com/SecBear/skelegent/commit/15de639b9c0ca9541e056807ddac8d345650c215))
* Wave 1 batch 2 — handoff detection, wire protocol, builder, test utils, schema tools ([9e7f9f5](https://github.com/SecBear/skelegent/commit/9e7f9f53584598ebd00a4ea473de0e8069297214))


### Bug Fixes

* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* update all Operator::execute call sites for Capabilities parameter ([dc761cc](https://github.com/SecBear/skelegent/commit/dc761cccc2014d63e362522dd84769c0458f6dc4))
