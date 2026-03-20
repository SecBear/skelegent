# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-effects-core-v0.4.0...skg-effects-core-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))


### Bug Fixes

* **context-engine:** reconcile durable orch merge ([d1f9a4c](https://github.com/SecBear/skelegent/commit/d1f9a4cfb8c89bda7fe47b6d67fd3301af03b91a))
