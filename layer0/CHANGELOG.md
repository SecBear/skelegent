# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/layer0-v0.4.0...layer0-v1.0.0) (2026-03-13)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* Protocol-level identity type renamed from AgentId to OperatorId. 'Agent' in neuron is an implementation (inference loop + provider), not a protocol concept. The protocol type is Operator.

### Features

* add InferRequest/InferResponse + Provider::infer() method ([f8b6b33](https://github.com/SecBear/skelegent/commit/f8b6b336d491dc8f11e47896b47c29e7a0da63bd))
* DIY-first primitive decomposition for context-engine ([4764e7f](https://github.com/SecBear/skelegent/commit/4764e7f77f4e252a967a3dcb325d9d1a0b9c04f3))
* dynamic tool availability + tool approval (AD-23, AD-24) ([de4cd16](https://github.com/SecBear/skelegent/commit/de4cd16772af4624b460256ca06f6d1b78183c3f))
* **layer0:** add concrete Context type (Phase 2, Task 2.1) ([2afc4b2](https://github.com/SecBear/skelegent/commit/2afc4b2c780a0447fe9765ac88509e7771a7bbc8))
* **layer0:** add DispatchStack, StoreStack, ExecStack middleware builders ([b2a369c](https://github.com/SecBear/skelegent/commit/b2a369c2adf8eae7504781a1ff6502044cec532a))
* **layer0:** add Message, Role types and per-boundary middleware traits ([20234d6](https://github.com/SecBear/skelegent/commit/20234d625345a58349a30a8140379e32ade6f4de))
* **layer0:** add Message::estimated_tokens() and text_content() helpers ([6d3a68c](https://github.com/SecBear/skelegent/commit/6d3a68cabd54e6bcf1573535b6e1fd930cbe37c7))
* migrate ContextAssembler to Message type + add MessageMeta builders (Phase R.4) ([6edcf02](https://github.com/SecBear/skelegent/commit/6edcf022b2a67b4087bd4d7e7201ba1130121469))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* Phase R — reshape reusable code for middleware era ([7a5cc90](https://github.com/SecBear/skelegent/commit/7a5cc9000c4e926337c4bb41403bb0d641eb12bb))
* StopReason #[non_exhaustive] + ProviderMessage removal + proc macro + doc polish ([3b22a6e](https://github.com/SecBear/skelegent/commit/3b22a6eeefc4e2b9c2d0d813db9d3d4094d27ce8))
* **turn:** ToolOperator adapter, DispatchPlanner rename, ReactOperator orchestrator dispatch ([78fea8f](https://github.com/SecBear/skelegent/commit/78fea8fee9d1c7279e3993d25ad457ba91c25f14))
* v3.2 sweep system, hook enhancements, and extras extraction ([17d9823](https://github.com/SecBear/skelegent/commit/17d98234004632bdf19dde985518844d95dc25fd))


### Bug Fixes

* repair pre-existing clippy lints ([278017e](https://github.com/SecBear/skelegent/commit/278017e1a23f6ec526beadad8b68acd00214f375))


### Code Refactoring

* AgentId → OperatorId across all crates ([e033586](https://github.com/SecBear/skelegent/commit/e03358693a81998e12b250abe7b533305ab911ab))
