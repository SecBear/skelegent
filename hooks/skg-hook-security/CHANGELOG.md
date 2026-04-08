# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-hook-security-v0.4.0...skg-hook-security-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* batch 1+2 consolidation ([edf5ae2](https://github.com/SecBear/skelegent/commit/edf5ae2bbe5dbb1d6f744d4463a1ccf1e96d63cb))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* **PER-164,PER-31,PER-193:** v2 fitness checks, compaction MCP server, declarative system design ([35fc486](https://github.com/SecBear/skelegent/commit/35fc4864e8347367706b1df7e35f067b4acd2b79))
* streaming dispatch as layer0 primitive ([6117665](https://github.com/SecBear/skelegent/commit/6117665f586110bdcac64c438e63bf7631860361))
* Wave 1 design roadmap implementation ([cb15f7b](https://github.com/SecBear/skelegent/commit/cb15f7b5e2d1bf88e22ebbb71ebabf5fdf1cf37b))


### Bug Fixes

* correctness and security fixes from PR review (C1, H1, H2, H4, H5, H9) ([31eaeed](https://github.com/SecBear/skelegent/commit/31eaeed06bbbdf4a020d8f225d276270395f64c2))
