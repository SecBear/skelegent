# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skelegent-v0.4.0...skelegent-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* batch 1+2 consolidation ([edf5ae2](https://github.com/SecBear/skelegent/commit/edf5ae2bbe5dbb1d6f744d4463a1ccf1e96d63cb))
* **layer0, context-engine:** add EffectEmitter and CognitiveOperator ([3cb67af](https://github.com/SecBear/skelegent/commit/3cb67afa425339deec5e71dbcf983809f1cfc75c))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* Wave 1 batch 2 — handoff detection, wire protocol, builder, test utils, schema tools ([9e7f9f5](https://github.com/SecBear/skelegent/commit/9e7f9f53584598ebd00a4ea473de0e8069297214))


### Bug Fixes

* exclude autoexamples from workspace root package ([93a0139](https://github.com/SecBear/skelegent/commit/93a01397b05e5f3b4c4622b3143809113586955b))
* post-rebase consolidation ([d45208c](https://github.com/SecBear/skelegent/commit/d45208c6fdbda1c1445359242a7e50abc4c1b2bf))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))
* resolve all P1 + P2 audit findings ([b255033](https://github.com/SecBear/skelegent/commit/b255033a1bba35ee9bd4541e45b08f3fdf44a4b9))
