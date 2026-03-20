# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-effects-local-v0.4.0...skg-effects-local-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* **layer0:** Orchestrator now requires Dispatcher supertrait. Capabilities struct removed. execute() reverted to single-param.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* Dispatcher::dispatch takes &DispatchContext instead of &OperatorId ([5858d49](https://github.com/SecBear/skelegent/commit/5858d4925817b9f7b81624a76fd5334f872daca6))
* implement graph ops on InMemoryStore, add graph and effect pipeline tests ([5c057c0](https://github.com/SecBear/skelegent/commit/5c057c085fdb31e0533956f180b5d8ee1148d560))
* **layer0:** Orchestrator extends Dispatcher — one invocation primitive ([4eafc11](https://github.com/SecBear/skelegent/commit/4eafc110dd6e798665c2c420f1e83037b24a7017))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* streaming dispatch as layer0 primitive ([6117665](https://github.com/SecBear/skelegent/commit/6117665f586110bdcac64c438e63bf7631860361))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))


### Bug Fixes

* **context-engine:** reconcile durable orch merge ([d1f9a4c](https://github.com/SecBear/skelegent/commit/d1f9a4cfb8c89bda7fe47b6d67fd3301af03b91a))
* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* polish primitives — wire disconnected seams, fix silent failures ([a03637d](https://github.com/SecBear/skelegent/commit/a03637d1182bb4c7b220392018fce1f119a13d3b))
* preserve structured JSON in handoff metadata instead of Null ([94feb3c](https://github.com/SecBear/skelegent/commit/94feb3cb144f965084ea6f15b07e9d6b39dfdca5))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))
* resolve all P1 + P2 audit findings ([b255033](https://github.com/SecBear/skelegent/commit/b255033a1bba35ee9bd4541e45b08f3fdf44a4b9))
