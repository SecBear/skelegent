#![deny(missing_docs)]
//! # skelegent
//!
//! Composable, async-first agentic AI framework for Rust.
//!
//! Skelegent provides a layered architecture: a minimal set of protocol traits in
//! [`layer0`] (Operator, Orchestrator, StateStore, Environment), operator
//! implementations (ReAct loop, single-shot), provider adapters (Anthropic, OpenAI,
//! Ollama), state backends, orchestration primitives, and a high-level
//! [`agent()`](crate::agent) builder for quick-start use. All pieces are opt-in via
//! Cargo feature flags — pull in exactly what you need.
//!
//! ## Quick Start
//!
//! Add the `agent` feature and a provider to `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! skelegent = { version = "0.4", features = ["agent", "provider-anthropic"] }
//! tokio = { version = "1", features = ["full"] }
//! ```
//!
//! Then run an agent (requires `ANTHROPIC_API_KEY` in the environment):
//!
//! ```rust,ignore
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let output = skelegent::agent("claude-sonnet-4-20250514")
//!         .system("You are a helpful assistant.")
//!         .max_turns(5)
//!         .build()?
//!         .run("What is the capital of France?")
//!         .await?;
//!
//!     if let Some(text) = output.message.as_text() {
//!         println!("{text}");
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | What it enables |
//! |---------|----------------|
//! | `core` *(default)* | [`layer0`] protocols + [`skg_context`] + [`skg_tool`] + [`skg_turn`] |
//! | `agent` | [`agent()`](crate::agent) builder (implies `context-engine` + `state-memory`) |
//! | `context-engine` | [`skg_context_engine::Context`] — composable context engine |
//! | `op-single-shot` | [`skg_op_single_shot::SingleShotOperator`] — one-shot operator |
//! | `provider-anthropic` | Anthropic Claude (reads `ANTHROPIC_API_KEY`) |
//! | `provider-openai` | OpenAI / o-series models (reads `OPENAI_API_KEY`) |
//! | `provider-ollama` | Ollama local models (no key required) |
//! | `providers-all` | All three providers combined |
//! | `state-memory` | [`skg_state_memory::MemoryStore`] — in-process store |
//! | `state-fs` | [`skg_state_fs::FsStore`] — filesystem-backed store |
//! | `orch-kit` | [`skg_orch_kit::Kit`] orchestration assembly primitives |
//! | `orch-local` | [`skg_orch_kit::OrchestratedRunner`] — local runner |
//! | `mcp` | [`skg_mcp`] MCP server/client integration |
//! | `env-local` | [`skg_env_local`] local process environment |
//! | `agent` | High-level agent API (implies `context-engine` + `state-memory`) |
//! | `macros` | Proc-macro support for deriving `ToolDyn` |
//!
//! ## Key Types (via [`prelude`])
//!
//! Import `skelegent::prelude::*` for the most common types:
//!
//! | Type | Description |
//! |------|-------------|
//! | [`layer0::Operator`] | Core async-turn trait — one agent, one cycle |
//! | [`layer0::StateStore`] | Persistent state backend trait |
//! | [`layer0::OperatorInput`] | Input envelope (content + trigger type) |
//! | [`layer0::OperatorOutput`] | Output envelope (content + exit reason + effects) |
//! | [`prelude::Context`] | Composable context engine (requires `context-engine`) |
//! | [`prelude::ToolRegistry`] | Registry of tools available to an agent |
//! | [`prelude::MemoryStore`] | In-process state store (requires `state-memory`) |
//! | [`prelude::Provider`] | LLM provider trait (RPITIT, not object-safe) |

#[cfg(feature = "core")]
pub use layer0;
#[cfg(feature = "core")]
pub use skg_context;
#[cfg(feature = "context-engine")]
pub use skg_context_engine;
#[cfg(feature = "env-local")]
pub use skg_env_local;
#[cfg(feature = "mcp")]
pub use skg_mcp;
#[cfg(feature = "op-single-shot")]
pub use skg_op_single_shot;
#[cfg(feature = "orch-kit")]
pub use skg_orch_kit;
#[cfg(feature = "orch-local")]
pub use skg_orch_local;
#[cfg(feature = "provider-anthropic")]
pub use skg_provider_anthropic;
#[cfg(feature = "provider-ollama")]
pub use skg_provider_ollama;
#[cfg(feature = "provider-openai")]
pub use skg_provider_openai;
#[cfg(feature = "state-fs")]
pub use skg_state_fs;
#[cfg(feature = "state-memory")]
pub use skg_state_memory;
#[cfg(feature = "core")]
pub use skg_tool;
#[cfg(feature = "core")]
pub use skg_turn;

#[cfg(feature = "agent")]
mod agent;
#[cfg(feature = "agent")]
pub use agent::{AgentBuildError, AgentBuilder, BuiltAgent, agent};

/// Happy-path imports for composing Skelegent systems.
pub mod prelude {
    #[cfg(feature = "core")]
    pub use layer0::{
        Content, ContentBlock, Effect, Environment, ExitReason, Operator, OperatorConfig,
        OperatorId, OperatorInput, OperatorOutput, Scope, SessionId, StateReader, StateStore,
        WorkflowId,
    };

    #[cfg(feature = "core")]
    pub use layer0::middleware::{
        DispatchMiddleware, DispatchNext, DispatchStack, ExecMiddleware, ExecNext, ExecStack,
        StoreMiddleware, StoreStack, StoreWriteNext,
    };

    #[cfg(feature = "core")]
    pub use skg_tool::{ToolDyn, ToolError, ToolRegistry};

    #[cfg(feature = "core")]
    pub use skg_turn::provider::{Provider, ProviderError};

    #[cfg(feature = "context-engine")]
    pub use skg_context_engine::{AssemblyExt, Context, ReactLoopConfig, react_loop};

    #[cfg(feature = "op-single-shot")]
    pub use skg_op_single_shot::SingleShotOperator;

    #[cfg(feature = "orch-kit")]
    pub use skg_orch_kit::{Kit, OrchestratedRunner};

    #[cfg(feature = "state-memory")]
    pub use skg_state_memory::MemoryStore;

    #[cfg(feature = "state-fs")]
    pub use skg_state_fs::FsStore;

    #[cfg(feature = "agent")]
    pub use crate::{AgentBuildError, AgentBuilder, BuiltAgent, agent};
}
