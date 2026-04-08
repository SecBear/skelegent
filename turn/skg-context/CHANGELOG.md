# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/skg-context-v0.4.0...skg-context-v1.0.0) (2026-04-08)


### ⚠ BREAKING CHANGES

* all recipe types removed from public API
* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.

### Features

* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* **PER-164,PER-31,PER-193:** v2 fitness checks, compaction MCP server, declarative system design ([35fc486](https://github.com/SecBear/skelegent/commit/35fc4864e8347367706b1df7e35f067b4acd2b79))


### Code Refactoring

* delete recipe ops, skg-context crate, and compaction server ([73ec98a](https://github.com/SecBear/skelegent/commit/73ec98a46ef87f802555bbab4a318abc493e3b7a))
