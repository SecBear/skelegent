# Introduction

**skelegent** is a composable agentic AI architecture implemented as a Rust workspace. It provides the building blocks for constructing agentic systems -- from a single LLM call with tool use, to multi-agent orchestration with durable execution, state persistence, and environment isolation.

## What skelegent is

skelegent is a set of Rust crates organized into six architectural layers:

- **Layer 0** defines the stability contract: four protocol traits and two cross-cutting interfaces that every other layer builds on. These traits almost never change.
- **Layers 1--5** provide swappable implementations of those protocols: providers (Anthropic, OpenAI, Ollama), operators (ReAct loops, single-shot), orchestration, state persistence, environment isolation, and hook-based observation.

The result is a system where you pick the implementations you need and compose them. A local development setup and a globally distributed production deployment use the same trait boundaries -- the only difference is which implementations back each protocol.

## What skelegent is not

skelegent is not a framework. There is no runtime you boot, no configuration DSL, no workflow engine. It is a collection of crates with well-defined trait boundaries. You compose them in your own application code.

skelegent is not an LLM wrapper library. While it includes provider implementations for making LLM calls, the architecture is designed around the full lifecycle of agentic systems: reasoning loops, tool execution, state management, multi-agent composition, security hooks, and environment isolation.

## Key properties

- **Provider-agnostic.** The `Provider` trait abstracts over Anthropic, OpenAI, and Ollama. Adding a new provider means implementing one trait.
- **Object-safe protocol boundaries.** All Layer 0 traits work behind `Box<dyn Trait>` and are `Send + Sync`. You can compose implementations at runtime without generics leaking through your entire application.
- **Trait-based composition.** Every protocol (operator execution, orchestration, state, environment) is a trait. Swap implementations without changing calling code.
- **Precise cost tracking.** All monetary values use `rust_decimal::Decimal`, avoiding floating-point accumulation errors across thousands of LLM calls.
- **Serializable boundaries.** All protocol messages (`OperatorInput`, `OperatorOutput`, effects, signals) implement `Serialize + Deserialize`. An in-process function call and a cross-network RPC use the same types.

## License

skelegent is dual-licensed under MIT and Apache-2.0, following the Rust ecosystem convention.

## Source code

The source code is hosted at [github.com/secbear/skelegent](https://github.com/secbear/skelegent).
