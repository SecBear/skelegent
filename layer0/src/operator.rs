//! The Operator protocol — what one operator does per cycle.

use crate::context::Message;
use crate::dispatch::EffectEmitter;
use crate::dispatch_context::DispatchContext;
use crate::{content::Content, duration::DurationMs, effect::Effect, error::OperatorError, id::*};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// What triggers an operator invocation. Informs context assembly — a scheduled trigger
/// means you need to reconstruct everything from state, while a user
/// message carries conversation context naturally.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    /// Human sent a message.
    User,
    /// Another agent assigned a task.
    Task,
    /// Signal from another workflow/agent.
    Signal,
    /// Cron/schedule triggered.
    Schedule,
    /// System event (file change, webhook, etc.).
    SystemEvent,
    /// Future trigger types.
    Custom(String),
}

/// Input to an operator. Everything the operator needs to execute.
///
/// Design decision: OperatorInput does NOT include conversation history
/// or memory contents. The operator runtime reads those from a StateStore
/// during context assembly. OperatorInput carries the *new* information
/// that triggered this invocation — not the accumulated state.
///
/// This keeps the protocol boundary clean: the caller provides what's
/// new, the operator runtime decides how to assemble context from what's
/// new + what's stored.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorInput {
    /// The new message/task/signal that triggered this operator invocation.
    pub message: Content,

    /// What caused this operator invocation to start.
    pub trigger: TriggerType,

    /// Session for conversation continuity. If None, the operator is stateless.
    /// The operator runtime uses this to read history from the StateStore.
    pub session: Option<SessionId>,

    /// Configuration for this specific operator execution.
    /// None means "use the operator runtime's defaults."
    pub config: Option<OperatorConfig>,

    /// Opaque metadata that passes through the operator unchanged.
    /// Useful for tracing (trace_id), routing (priority), or
    /// domain-specific context that the protocol doesn't need
    /// to understand.
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// Pre-assembled context from the caller.
    ///
    /// When set, the operator runtime seeds its context with these messages
    /// before processing the new `message`. This enables parent operators
    /// to curate child context: full inheritance, summary injection,
    /// filtered history, or any other assembly strategy.
    ///
    /// When `None`, the operator assembles context from scratch using
    /// the `StateStore` and its own identity configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<Message>>,
}

/// Per-operator configuration overrides. Every field is optional —
/// None means "use the implementation's default."
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperatorConfig {
    /// Maximum iterations of the inner ReAct loop.
    pub max_turns: Option<u32>,

    /// Maximum cost for this operator invocation in USD.
    pub max_cost: Option<Decimal>,

    /// Maximum wall-clock time for this operator invocation.
    pub max_duration: Option<DurationMs>,

    /// Model override (implementation-specific string).
    pub model: Option<String>,

    /// Operator restrictions for this operator invocation.
    /// None = use defaults. Some(list) = only these operators.
    #[serde(alias = "allowed_tools")]
    pub allowed_operators: Option<Vec<String>>,

    /// Additional system prompt content to prepend/append.
    /// Does not replace the operator runtime's base identity —
    /// it augments it. Use for per-task instructions.
    pub system_addendum: Option<String>,
}

/// Why an operator invocation ended. The caller needs to know this to decide
/// what happens next (retry? continue? escalate?).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    /// Model produced a final text response (natural completion).
    Complete,
    /// Hit the max_turns limit.
    MaxTurns,
    /// Hit the cost budget (`max_cost`) or the tool-call step limit (`max_tool_calls`).
    /// Use `BudgetEvent` sink notifications to distinguish the two causes.
    BudgetExhausted,
    /// Circuit breaker tripped (consecutive failures).
    CircuitBreaker,
    /// Wall-clock timeout.
    Timeout,
    /// Interceptor/middleware halted execution.
    InterceptorHalt {
        /// The reason the interceptor halted execution.
        reason: String,
    },
    /// Unrecoverable error during execution.
    Error,
    /// Provider safety system stopped generation (HTTP 200, content filtered).
    ///
    /// Semantically distinct from `Error` (not a transport or execution failure)
    /// and `Complete` (model did not finish naturally). Arrives via
    /// `StopReason::ContentFilter` in the provider response — the provider
    /// acknowledged the request but refused to complete it. Not retriable
    /// without modification to the context or request.
    SafetyStop {
        /// Human-readable reason string supplied by the provider or runtime.
        reason: String,
    },
    /// One or more tool calls require human approval before execution.
    /// The calling layer should inspect [`OperatorOutput::effects`] for
    /// [`Effect::ToolApprovalRequired`] entries, obtain approval, then
    /// either execute the tools and re-enter the loop, or inject a denial
    /// message and re-enter.
    AwaitingApproval,
    /// Future exit reasons.
    Custom(String),
}

/// Output from an operator. Contains the response, metadata about
/// execution, and any side-effects the operator wants executed.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorOutput {
    /// The operator's response content.
    pub message: Content,

    /// Why the operator invocation ended.
    pub exit_reason: ExitReason,

    /// Execution metadata (cost, tokens, timing).
    pub metadata: OperatorMetadata,

    /// Side-effects the operator wants executed.
    ///
    /// **Preferred path:** call [`EffectEmitter::effect`](crate::dispatch::EffectEmitter::effect)
    /// during execution. The dispatch handle's [`collect`](crate::dispatch::DispatchHandle::collect)
    /// method gathers emitted effects into this field automatically.
    ///
    /// **Legacy path:** operators may still populate this field directly.
    /// When no [`DispatchEvent::EffectEmitted`](crate::dispatch::DispatchEvent::EffectEmitted)
    /// events are received, `collect()` preserves whatever the operator placed here.
    ///
    /// CRITICAL DESIGN DECISION: The operator declares effects but does
    /// not execute them. The calling layer (orchestrator, lifecycle
    /// coordinator) decides when and how to execute them. This is
    /// what makes the operator runtime independent of the layers around it.
    ///
    /// An operator running in-process has its effects executed immediately.
    /// An operator running in a Temporal activity has its effects serialized
    /// and executed by the workflow. Same operator code, different execution.
    #[serde(default)]
    pub effects: Vec<Effect>,
}

/// Execution metadata. Every field is concrete (not optional) because
/// every operator produces this data. Implementations that can't track
/// a field (e.g., cost for a local model) use zero/default.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorMetadata {
    /// Input tokens consumed.
    pub tokens_in: u64,
    /// Output tokens generated.
    pub tokens_out: u64,
    /// Cost in USD.
    pub cost: Decimal,
    /// Number of ReAct loop iterations used.
    pub turns_used: u32,
    /// Record of each sub-dispatch made.
    #[serde(alias = "tools_called")]
    pub sub_dispatches: Vec<SubDispatchRecord>,
    /// Wall-clock duration of the operator invocation.
    pub duration: DurationMs,
}

/// Record of a single sub-dispatch within an operator execution.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubDispatchRecord {
    /// Name of the operator (sub-dispatch) that was called.
    pub name: String,
    /// How long the sub-dispatch took.
    pub duration: DurationMs,
    /// Whether the call succeeded.
    pub success: bool,
}

impl Default for OperatorMetadata {
    fn default() -> Self {
        Self {
            tokens_in: 0,
            tokens_out: 0,
            cost: Decimal::ZERO,
            turns_used: 0,
            sub_dispatches: vec![],
            duration: DurationMs::ZERO,
        }
    }
}

impl OperatorInput {
    /// Create a new OperatorInput with required fields.
    pub fn new(message: Content, trigger: TriggerType) -> Self {
        Self {
            message,
            trigger,
            session: None,
            config: None,
            metadata: serde_json::Value::Null,
            context: None,
        }
    }
}

impl OperatorOutput {
    /// Create a new OperatorOutput with required fields.
    pub fn new(message: Content, exit_reason: ExitReason) -> Self {
        Self {
            message,
            exit_reason,
            metadata: OperatorMetadata::default(),
            effects: vec![],
        }
    }

    /// Check whether this output contains effects that need an interpreter.
    ///
    /// Returns `true` if [`effects`](Self::effects) is non-empty. Callers that
    /// consume `OperatorOutput` directly (without an `EffectInterpreter` or
    /// `OrchestratedRunner`) should check this and decide whether the unhandled
    /// effects are acceptable or a bug.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let output = op.execute(input, &ctx, &EffectEmitter::noop()).await?
    /// if output.has_unhandled_effects() {
    ///     tracing::warn!("effects will not be executed: {:?}", output.effects);
    /// }
    /// ```
    pub fn has_unhandled_effects(&self) -> bool {
        !self.effects.is_empty()
    }
}

impl SubDispatchRecord {
    /// Create a new SubDispatchRecord.
    pub fn new(name: impl Into<String>, duration: DurationMs, success: bool) -> Self {
        Self {
            name: name.into(),
            duration,
            success,
        }
    }
}

/// Metadata describing a tool/sub-operator's external interface.
///
/// Used by orchestrators, MCP servers, and any component that needs
/// to advertise an operator's capabilities to external systems
/// (including LLM tool-use schemas).
///
/// This is the bridge between the operator protocol and tool-use APIs:
/// an operator that exposes itself as a "tool" attaches `ToolMetadata`
/// so callers know how to invoke it.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    /// Human-readable name for the tool.
    pub name: String,
    /// Description of what the tool does (shown to LLMs in tool-use prompts).
    pub description: String,
    /// JSON Schema describing the expected input.
    pub input_schema: serde_json::Value,
    /// Whether this tool is safe to call concurrently with other tools.
    /// Used by dispatch planners to decide parallel vs. sequential execution.
    pub parallel_safe: bool,
}

impl ToolMetadata {
    /// Create a new `ToolMetadata`.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
        parallel_safe: bool,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            parallel_safe,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// THE TRAIT
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Protocol ① — The Operator
///
/// What one operator does per cycle. Receives input, assembles context,
/// reasons (model call), acts (tool execution), produces output.
///
/// The ReAct while-loop, the agentic loop, the augmented LLM —
/// whatever you call it, this trait is its boundary.
///
/// Implementations:
/// - skelegent's AgentLoop (full-featured operator with tools + context mgmt)
/// - A raw API call wrapper (minimal, no tools)
/// - A human-in-the-loop adapter (waits for human input)
/// - A mock (for testing)
///
/// The trait is intentionally one method. The operator is atomic from the
/// outside — you send input, you get output. Everything that happens
/// inside (how many model calls, how many tool uses, what context
/// strategy) is the implementation's concern.
#[async_trait]
pub trait Operator: Send + Sync {
    /// Execute a single operator invocation.
    ///
    /// The operator runtime:
    /// 1. Assembles context (identity + history + memory + tools)
    /// 2. Runs the ReAct loop (reason → act → observe → repeat)
    /// 3. Returns the output + effects
    ///
    /// The operator MAY read from a StateStore during context assembly.
    /// The operator MUST NOT write to external state directly — it
    /// declares writes as Effects in the output.
    ///
    /// The `ctx` parameter carries dispatch context including identity,
    /// tracing, operator ID, and typed extensions.
    ///
    /// The `emitter` parameter streams observable events (progress,
    /// artifacts) to the dispatch caller in real-time. Operators that
    /// don't stream can ignore it.
    ///
    /// Operators that compose (invoke siblings) hold `Arc<dyn Dispatcher>`
    /// as a field via constructor injection. The execute signature stays
    /// clean — non-composing operators never see dispatch infrastructure.
    async fn execute(
        &self,
        input: OperatorInput,
        ctx: &DispatchContext,
        emitter: &EffectEmitter,
    ) -> Result<OperatorOutput, OperatorError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_metadata_construction() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let meta = ToolMetadata::new("search", "Search the web", schema.clone(), true);
        assert_eq!(meta.name, "search");
        assert_eq!(meta.description, "Search the web");
        assert_eq!(meta.input_schema, schema);
        assert!(meta.parallel_safe);
    }

    #[test]
    fn tool_metadata_serde_roundtrip() {
        let meta = ToolMetadata::new(
            "code_exec",
            "Execute code in a sandbox",
            serde_json::json!({"type": "object"}),
            false,
        );
        let json = serde_json::to_string(&meta).expect("serialize");
        let back: ToolMetadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.name, "code_exec");
        assert_eq!(back.description, "Execute code in a sandbox");
        assert!(!back.parallel_safe);
    }
}
