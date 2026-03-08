#![deny(missing_docs)]
//! # neuron — umbrella crate
//!
//! Provides a single import surface for Neuron.
//! Re-exports protocol and key implementations behind feature flags, plus a
//! `prelude` for the happy path.

#[cfg(feature = "core")]
pub use layer0;
#[cfg(feature = "core")]
pub use neuron_context;
#[cfg(feature = "env-local")]
pub use neuron_env_local;
#[cfg(feature = "mcp")]
pub use neuron_mcp;
#[cfg(feature = "op-react")]
pub use neuron_op_react;
#[cfg(feature = "op-single-shot")]
pub use neuron_op_single_shot;
#[cfg(feature = "orch-kit")]
pub use neuron_orch_kit;
#[cfg(feature = "orch-local")]
pub use neuron_orch_local;
#[cfg(feature = "provider-anthropic")]
pub use neuron_provider_anthropic;
#[cfg(feature = "provider-ollama")]
pub use neuron_provider_ollama;
#[cfg(feature = "provider-openai")]
pub use neuron_provider_openai;
#[cfg(feature = "state-fs")]
pub use neuron_state_fs;
#[cfg(feature = "state-memory")]
pub use neuron_state_memory;
#[cfg(feature = "core")]
pub use neuron_tool;
#[cfg(feature = "core")]
pub use neuron_turn;

/// Happy-path imports for composing Neuron systems.
pub mod prelude {
    #[cfg(feature = "core")]
    pub use layer0::{
        AgentId, Content, ContentBlock, Effect, Environment, ExitReason, Operator, OperatorConfig,
        OperatorInput, OperatorOutput, Scope, SessionId, StateReader, StateStore, WorkflowId,
    };

    #[cfg(feature = "core")]
    pub use layer0::middleware::{
        DispatchMiddleware, DispatchNext, DispatchStack, ExecMiddleware, ExecNext, ExecStack,
        StoreMiddleware, StoreStack, StoreWriteNext,
    };

    #[cfg(feature = "core")]
    pub use neuron_tool::{ToolDyn, ToolError, ToolRegistry};

    #[cfg(feature = "core")]
    pub use neuron_turn::provider::{Provider, ProviderError};

    #[cfg(feature = "op-react")]
    pub use neuron_op_react::{ReactConfig, ReactOperator};

    #[cfg(feature = "op-single-shot")]
    pub use neuron_op_single_shot::SingleShotOperator;

    #[cfg(feature = "orch-kit")]
    pub use neuron_orch_kit::{Kit, OrchestratedRunner};

    #[cfg(feature = "state-memory")]
    pub use neuron_state_memory::MemoryStore;

    #[cfg(feature = "state-fs")]
    pub use neuron_state_fs::FsStore;
}
