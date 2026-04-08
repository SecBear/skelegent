# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-secret-v0.4.0...skg-secret-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* add builder methods, Display impls, InferResponse conversions, and re-exports for API ergonomics ([c76ab46](https://github.com/SecBear/skelegent/commit/c76ab46c9d8cd530e9394d40c28724afeaf49119))
* add SecretMiddleware with continuation-passing stack for policy, audit, and caching ([91527d0](https://github.com/SecBear/skelegent/commit/91527d0c110d5d1b1567b4c7af20fd6c56494bd2))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
