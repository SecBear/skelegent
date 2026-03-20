# Changelog

## [1.0.0](https://github.com/SecBear/skelegent/compare/layer0-v0.4.0...layer0-v1.0.0) (2026-03-20)


### ⚠ BREAKING CHANGES

* **orch:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* **layer0:** Orchestrator now requires Dispatcher supertrait. Capabilities struct removed. execute() reverted to single-param.
* **layer0:** Operator::execute now takes &Capabilities as second parameter. All Operator implementations must be updated.
* Protocol-level identity type renamed from AgentId to OperatorId. 'Agent' in neuron is an implementation (inference loop + provider), not a protocol concept. The protocol type is Operator.

### Features

* add builder methods, Display impls, InferResponse conversions, and re-exports for API ergonomics ([c76ab46](https://github.com/SecBear/skelegent/commit/c76ab46c9d8cd530e9394d40c28724afeaf49119))
* add DispatchContext and thread through all trait signatures ([37a1766](https://github.com/SecBear/skelegent/commit/37a1766c45dabc29399223735cb4d69ba4d16aaf))
* add InferRequest/InferResponse + Provider::infer() method ([f8b6b33](https://github.com/SecBear/skelegent/commit/f8b6b336d491dc8f11e47896b47c29e7a0da63bd))
* **approval:** HITL approval protocol — typed request/response contract ([a7f0cc4](https://github.com/SecBear/skelegent/commit/a7f0cc43c3be57c15895ef43886f525e909d4ace))
* Dispatcher::dispatch takes &DispatchContext instead of &OperatorId ([5858d49](https://github.com/SecBear/skelegent/commit/5858d4925817b9f7b81624a76fd5334f872daca6))
* DispatchHandle event interception + StreamProvider middleware ([aaa6fa0](https://github.com/SecBear/skelegent/commit/aaa6fa07d39e3912bb2551d18b0785716fad50f7))
* DIY-first primitive decomposition for context-engine ([4764e7f](https://github.com/SecBear/skelegent/commit/4764e7f77f4e252a967a3dcb325d9d1a0b9c04f3))
* dynamic tool availability + tool approval (AD-23, AD-24) ([de4cd16](https://github.com/SecBear/skelegent/commit/de4cd16772af4624b460256ca06f6d1b78183c3f))
* EffectLog + EffectMiddleware + FromContext extractors + TokenCounter ([4c4bf44](https://github.com/SecBear/skelegent/commit/4c4bf44e855bd299dcdccd587c872586dd9cf691))
* implement graph ops on InMemoryStore, add graph and effect pipeline tests ([5c057c0](https://github.com/SecBear/skelegent/commit/5c057c085fdb31e0533956f180b5d8ee1148d560))
* **layer0, context-engine:** add EffectEmitter and CognitiveOperator ([3cb67af](https://github.com/SecBear/skelegent/commit/3cb67afa425339deec5e71dbcf983809f1cfc75c))
* **layer0:** add Capabilities to Operator::execute — composition primitive ([e4cdc57](https://github.com/SecBear/skelegent/commit/e4cdc57b8d96053d2446f0fd187b88b1b3c3d00e))
* **layer0:** add concrete Context type (Phase 2, Task 2.1) ([2afc4b2](https://github.com/SecBear/skelegent/commit/2afc4b2c780a0447fe9765ac88509e7771a7bbc8))
* **layer0:** add DispatchStack, StoreStack, ExecStack middleware builders ([b2a369c](https://github.com/SecBear/skelegent/commit/b2a369c2adf8eae7504781a1ff6502044cec532a))
* **layer0:** add Message, Role types and per-boundary middleware traits ([20234d6](https://github.com/SecBear/skelegent/commit/20234d625345a58349a30a8140379e32ade6f4de))
* **layer0:** add Message::estimated_tokens() and text_content() helpers ([6d3a68c](https://github.com/SecBear/skelegent/commit/6d3a68cabd54e6bcf1573535b6e1fd930cbe37c7))
* **layer0:** Orchestrator extends Dispatcher — one invocation primitive ([4eafc11](https://github.com/SecBear/skelegent/commit/4eafc110dd6e798665c2c420f1e83037b24a7017))
* memory tools + EffectLog wiring + output_schema on ToolDyn ([02ead0f](https://github.com/SecBear/skelegent/commit/02ead0f60915666e750582efd9e6b2eaf2ed3e85))
* migrate ContextAssembler to Message type + add MessageMeta builders (Phase R.4) ([6edcf02](https://github.com/SecBear/skelegent/commit/6edcf022b2a67b4087bd4d7e7201ba1130121469))
* **orch:** durable orchestration core — portable run/control contracts ([#65](https://github.com/SecBear/skelegent/issues/65)) ([adc9ada](https://github.com/SecBear/skelegent/commit/adc9adaadd0a9d45d134c8e2377735af74686b31))
* Phase R — reshape reusable code for middleware era ([7a5cc90](https://github.com/SecBear/skelegent/commit/7a5cc9000c4e926337c4bb41403bb0d641eb12bb))
* RetryMiddleware crate + OperatorMeta trait ([dafaef7](https://github.com/SecBear/skelegent/commit/dafaef721c9bc59cc8803c391c852be80c4dce0f))
* StopReason #[non_exhaustive] + ProviderMessage removal + proc macro + doc polish ([3b22a6e](https://github.com/SecBear/skelegent/commit/3b22a6eeefc4e2b9c2d0d813db9d3d4094d27ce8))
* streaming dispatch as layer0 primitive ([6117665](https://github.com/SecBear/skelegent/commit/6117665f586110bdcac64c438e63bf7631860361))
* **turn:** ToolOperator adapter, DispatchPlanner rename, ReactOperator orchestrator dispatch ([78fea8f](https://github.com/SecBear/skelegent/commit/78fea8fee9d1c7279e3993d25ad457ba91c25f14))
* unify effects — single emission path via EffectEmitter ([b42bd36](https://github.com/SecBear/skelegent/commit/b42bd36d95a8841a8b20342819b163b2811e09d4))
* universal middleware — InferMiddleware, recorder, MiddlewareProvider ([08a015f](https://github.com/SecBear/skelegent/commit/08a015f88f2b7fedf480faeff5c8b73e709d00b8))
* v0.5 breaking bundle — streaming + effects + memory scoping ([96ffec9](https://github.com/SecBear/skelegent/commit/96ffec955db60afb6851ff612266dee018ff0eb0))
* v3.2 sweep system, hook enhancements, and extras extraction ([17d9823](https://github.com/SecBear/skelegent/commit/17d98234004632bdf19dde985518844d95dc25fd))
* Wave 1 batch 2 — handoff detection, wire protocol, builder, test utils, schema tools ([9e7f9f5](https://github.com/SecBear/skelegent/commit/9e7f9f53584598ebd00a4ea473de0e8069297214))
* Wave 1 design roadmap implementation ([cb15f7b](https://github.com/SecBear/skelegent/commit/cb15f7b5e2d1bf88e22ebbb71ebabf5fdf1cf37b))
* Wave 2 — breaking InferRequest bundle + orchestration + builder + OpenAPI ([285a89d](https://github.com/SecBear/skelegent/commit/285a89daf0f88d394cf4dfb68f56e111e9f99570))


### Bug Fixes

* **context-engine:** reconcile durable orch merge ([d1f9a4c](https://github.com/SecBear/skelegent/commit/d1f9a4cfb8c89bda7fe47b6d67fd3301af03b91a))
* error cascade fixes (3, 5, 6, 1) ([1930688](https://github.com/SecBear/skelegent/commit/1930688c95974eac971e902b7c616cc632a7e4f6))
* error propagation, observability, and structured error responses ([d188005](https://github.com/SecBear/skelegent/commit/d18800532a051a9377be3da1def15de1b3dc733c))
* polish primitives — wire disconnected seams, fix silent failures ([a03637d](https://github.com/SecBear/skelegent/commit/a03637d1182bb4c7b220392018fce1f119a13d3b))
* repair pre-existing clippy lints ([278017e](https://github.com/SecBear/skelegent/commit/278017e1a23f6ec526beadad8b68acd00214f375))
* resolve all blocking PR review issues ([aeab2c2](https://github.com/SecBear/skelegent/commit/aeab2c2b7cacafaed67a570ad46985aed4618f20))
* resolve all P1 + P2 audit findings ([b255033](https://github.com/SecBear/skelegent/commit/b255033a1bba35ee9bd4541e45b08f3fdf44a4b9))


### Code Refactoring

* AgentId → OperatorId across all crates ([e033586](https://github.com/SecBear/skelegent/commit/e03358693a81998e12b250abe7b533305ab911ab))
