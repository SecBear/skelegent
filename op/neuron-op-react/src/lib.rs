#![deny(missing_docs)]
//! ReAct operator — model + tools in a reasoning loop.
//!
//! Implements `layer0::Operator` by running the Reason-Act-Observe cycle:
//! assemble context → call model → execute tools → repeat until done.

mod intercept;
pub use intercept::*;

use async_trait::async_trait;
use layer0::content::Content;
use layer0::duration::DurationMs;
use layer0::effect::{Effect, Scope, SignalPayload};
use layer0::error::{OperatorError, OrchError};
use layer0::id::{AgentId, WorkflowId};
use layer0::lifecycle::{BudgetEvent, CompactionEvent};
use layer0::operator::{
    ExitReason, Operator, OperatorInput, OperatorMetadata, OperatorOutput, SubDispatchRecord,
};
use layer0::orchestrator::Orchestrator;
use neuron_tool::adapter::ToolRegistryOrchestrator;
use neuron_tool::{ToolConcurrencyHint, ToolRegistry};

use layer0::context::{Message, Role as L0Role};
use neuron_turn::convert::{message_to_provider, parts_to_content};
use neuron_turn::provider::Provider;
use neuron_turn::types::*;
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Sink for operator-emitted budget lifecycle events.
///
/// Implement this trait to observe step-limit, loop-detection, and timeout events
/// from ReactOperator. All methods receive owned events for maximum flexibility.
pub trait BudgetEventSink: Send + Sync {
    /// Called when a budget-related event occurs.
    fn on_budget_event(&self, event: BudgetEvent);
}

/// Sink for operator-emitted compaction lifecycle events.
///
/// Implement this trait to observe compaction failures, skips, and quality
/// outcomes from ReactOperator.
pub trait CompactionEventSink: Send + Sync {
    /// Called when a compaction-related event occurs.
    fn on_compaction_event(&self, event: CompactionEvent);
}

/// Snapshot of the context window at the time [`ReactOperator::context_snapshot`] is called.
///
/// Reflects the latest view of the in-flight context buffer maintained by the operator.
/// Safe to clone, serialize, and send across threads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextSnapshot {
    /// Messages currently in the context window, with their annotations.
    pub messages: Vec<Message>,
    /// Approximate token count of the current context (4 chars ≈ 1 token heuristic).
    pub token_count: usize,
    /// Number of messages pinned (will survive compaction).
    pub pinned_count: usize,
    /// Number of messages removed in the most recent compaction cycle.
    /// Zero if compaction has not yet run.
    pub last_compaction_removed: usize,
}

/// Static configuration for a ReactOperator instance.
pub struct ReactConfig {
    /// Base system prompt.
    pub system_prompt: String,
    /// Default model identifier.
    pub default_model: String,
    /// Default max tokens per response.
    pub default_max_tokens: u32,
    /// Default max turns before stopping.
    pub default_max_turns: u32,
    /// Fraction of the token budget reserved for compaction headroom.
    /// Compaction triggers at `max_tokens * 4 * (1 - compaction_reserve_pct)`.
    /// Must be in 0.01..=0.50. Default: 0.20 (20%).
    pub compaction_reserve_pct: f32,
    /// Maximum total tool calls across all turns. None = unlimited.
    pub max_tool_calls: Option<u32>,
    /// Maximum consecutive identical tool calls (same name + input hash).
    /// Exits with ExitReason::Custom("stuck_detected") when exceeded.
    pub max_repeat_calls: Option<u32>,
    /// Optional model selector. Called before each inference with the current request.
    /// Returns a model name override, or None to use the default.
    /// Enables task-type routing (e.g. route by message count, tool count, or cost).
    #[allow(clippy::type_complexity)]
    pub model_selector: Option<Arc<dyn Fn(&ProviderRequest) -> Option<String> + Send + Sync>>,
}

impl Default for ReactConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            default_model: String::new(),
            default_max_tokens: 4096,
            default_max_turns: 10,
            compaction_reserve_pct: 0.20,
            max_tool_calls: None,
            max_repeat_calls: None,
            model_selector: None,
        }
    }
}

impl ReactConfig {
    /// Validate that all configuration values are within acceptable ranges.
    ///
    /// Returns `Err` if any field is out of range.
    pub fn validated(self) -> Result<Self, &'static str> {
        if !(0.01..=0.50).contains(&self.compaction_reserve_pct) {
            return Err("compaction_reserve_pct must be 0.01..=0.50");
        }
        Ok(self)
    }
}

/// Resolved configuration merging defaults with per-request overrides.
struct ResolvedConfig {
    model: Option<String>,
    system: String,
    max_turns: u32,
    max_cost: Option<Decimal>,
    max_duration: Option<DurationMs>,
    allowed_operators: Option<Vec<String>>,
    max_tokens: u32,
}

// Re-export turn-kit primitives
pub use neuron_turn_kit::{
    BarrierPlanner, BatchItem, Concurrency, ConcurrencyDecider, ContextCommand, DispatchPlanner,
    SteeringCommand, SteeringSource,
};

/// Default decider: all tools Exclusive.
struct DefaultDecider;
impl ConcurrencyDecider for DefaultDecider {
    /// Return the concurrency class for a tool by name.
    fn concurrency(&self, _operator_name: &str) -> Concurrency {
        Concurrency::Exclusive
    }
}

/// Concurrency decider that reads per-tool metadata from ToolRegistry.
struct MetadataDecider {
    tools: ToolRegistry,
}
impl ConcurrencyDecider for MetadataDecider {
    fn concurrency(&self, operator_name: &str) -> Concurrency {
        match self.tools.get(operator_name) {
            Some(tool) => match tool.concurrency_hint() {
                ToolConcurrencyHint::Shared => Concurrency::Shared,
                ToolConcurrencyHint::Exclusive => Concurrency::Exclusive,
                _ => Concurrency::Exclusive,
            },
            None => Concurrency::Exclusive,
        }
    }
}

/// Sequential planner: each tool runs alone.
struct SequentialPlanner;
impl DispatchPlanner for SequentialPlanner {
    fn plan(
        &self,
        dispatch_requests: &[(String, String, serde_json::Value)],
        _decider: &dyn ConcurrencyDecider,
    ) -> Vec<BatchItem> {
        dispatch_requests
            .iter()
            .cloned()
            .map(BatchItem::Exclusive)
            .collect()
    }
}
/// A compaction function that reduces a message list.
///
/// Called when estimated token count exceeds the effective context limit.
/// Should return a shorter list, preserving pinned messages.
pub type Compactor = dyn Fn(&[Message]) -> Vec<Message> + Send + Sync;

/// A full-featured Operator implementation with a ReAct loop.
///
/// Generic over `P: Provider` (not object-safe). The object-safe boundary
/// is `layer0::Operator`, which `ReactOperator<P>` implements via `#[async_trait]`.
pub struct ReactOperator<P: Provider> {
    provider: P,
    tools: ToolRegistry,
    compactor: Option<Box<Compactor>>,
    interceptor: Option<Arc<dyn ReactInterceptor>>,
    state_reader: Arc<dyn layer0::StateReader>,
    config: ReactConfig,
    planner: Box<dyn DispatchPlanner>,
    decider: Box<dyn ConcurrencyDecider>,
    steering: Option<Arc<dyn SteeringSource>>,
    budget_sink: Option<Arc<dyn BudgetEventSink>>,
    compaction_sink: Option<Arc<dyn CompactionEventSink>>,
    /// Orchestrator for sub-operator dispatch. All tool calls route through
    /// this; direct `ToolDyn::call()` is no longer supported.
    orchestrator: Arc<dyn Orchestrator>,
    /// Live snapshot buffer, updated at key mutation points during `execute`.
    current_context: Arc<Mutex<Vec<Message>>>,
    /// Number of messages removed in the most recent compaction cycle.
    last_compaction_removed: Arc<Mutex<usize>>,
}

impl<P: Provider> ReactOperator<P> {
    /// Create a new ReactOperator with all dependencies.
    pub fn new(
        provider: P,
        tools: ToolRegistry,
        state_reader: Arc<dyn layer0::StateReader>,
        config: ReactConfig,
    ) -> Self {
        let orchestrator: Arc<dyn Orchestrator> =
            Arc::new(ToolRegistryOrchestrator::new(tools.clone()));
        Self {
            provider,
            tools,
            compactor: None,
            interceptor: None,
            state_reader,
            config,
            planner: Box::new(SequentialPlanner),
            decider: Box::new(DefaultDecider),
            steering: None,
            budget_sink: None,
            compaction_sink: None,
            orchestrator,
            current_context: Arc::new(Mutex::new(Vec::new())),
            last_compaction_removed: Arc::new(Mutex::new(0)),
        }
    }
    /// Opt-in: set a custom dispatch planner.
    pub fn with_planner(mut self, planner: Box<dyn DispatchPlanner>) -> Self {
        self.planner = planner;
        self
    }
    /// Opt-in: set a custom concurrency decider.
    pub fn with_concurrency_decider(mut self, decider: Box<dyn ConcurrencyDecider>) -> Self {
        self.decider = decider;
        self
    }
    /// Opt-in: use tool metadata to decide concurrency.
    pub fn with_metadata_concurrency(mut self) -> Self {
        self.decider = Box::new(MetadataDecider {
            tools: self.tools.clone(),
        });
        self
    }
    /// Opt-in: attach a [`ReactInterceptor`] for loop interception.
    pub fn with_interceptor(mut self, interceptor: Arc<dyn ReactInterceptor>) -> Self {
        self.interceptor = Some(interceptor);
        self
    }

    /// Opt-in: attach a steering source.
    pub fn with_steering(mut self, s: Arc<dyn SteeringSource>) -> Self {
        self.steering = Some(s);
        self
    }
    /// Opt-in: attach a sink for budget lifecycle events (step-limit, loop, timeout).
    pub fn with_budget_sink(mut self, sink: Arc<dyn BudgetEventSink>) -> Self {
        self.budget_sink = Some(sink);
        self
    }
    /// Opt-in: attach a sink for compaction lifecycle events (quality, failure).
    pub fn with_compaction_sink(mut self, sink: Arc<dyn CompactionEventSink>) -> Self {
        self.compaction_sink = Some(sink);
        self
    }
    /// Override the orchestrator used for sub-operator dispatch.
    ///
    /// By default, `new()` creates a `ToolRegistryOrchestrator` from the
    /// provided `ToolRegistry`. Use this to substitute a custom orchestrator
    /// (e.g. one that routes to remote agents or applies middleware).
    ///
    /// **Note:** streaming is not supported through the orchestrator path;
    /// dispatch is request-response only.
    pub fn with_orchestrator(mut self, orch: Arc<dyn Orchestrator>) -> Self {
        self.orchestrator = orch;
        self
    }
    /// Opt-in: set a model selector callback for per-inference routing.
    ///
    /// The selector is called before each inference call. Return `Some(model)` to
    /// override the model for that call, or `None` to use the default.
    pub fn with_model_selector(
        mut self,
        f: impl Fn(&ProviderRequest) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.config.model_selector = Some(Arc::new(f));
        self
    }
    /// Opt-in: attach a compaction function for context window management.
    ///
    /// Called when estimated token count exceeds the effective limit.
    /// Should return a shorter message list (preserving pinned messages).
    pub fn with_compactor(
        mut self,
        f: impl Fn(&[Message]) -> Vec<Message> + Send + Sync + 'static,
    ) -> Self {
        self.compactor = Some(Box::new(f));
        self
    }

    /// Return a point-in-time snapshot of the operator's context window.
    ///
    /// Safe to call before the first [`Operator::execute`] invocation — returns an
    /// empty snapshot in that case. Also safe to call concurrently with a running
    /// `execute` call; the snapshot reflects the most recent completed update point.
    ///
    /// The returned [`ContextSnapshot`] is a deep clone — subsequent mutations to the
    /// operator do not affect it.
    pub fn context_snapshot(&self) -> ContextSnapshot {
        let messages = self
            .current_context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let token_count = messages.iter().map(|m| m.estimated_tokens()).sum::<usize>();
        let pinned_count = messages
            .iter()
            .filter(|m| matches!(m.meta.policy, layer0::CompactionPolicy::Pinned))
            .count();
        let last_compaction_removed = *self
            .last_compaction_removed
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        ContextSnapshot {
            messages,
            token_count,
            pinned_count,
            last_compaction_removed,
        }
    }

    fn resolve_config(&self, input: &OperatorInput) -> ResolvedConfig {
        let tc = input.config.as_ref();
        let system = match tc.and_then(|c| c.system_addendum.as_ref()) {
            Some(addendum) => format!("{}\n{}", self.config.system_prompt, addendum),
            None => self.config.system_prompt.clone(),
        };
        ResolvedConfig {
            model: tc.and_then(|c| c.model.clone()).or_else(|| {
                if self.config.default_model.is_empty() {
                    None
                } else {
                    Some(self.config.default_model.clone())
                }
            }),
            system,
            max_turns: tc
                .and_then(|c| c.max_turns)
                .unwrap_or(self.config.default_max_turns),
            max_cost: tc.and_then(|c| c.max_cost),
            max_duration: tc.and_then(|c| c.max_duration),
            allowed_operators: tc.and_then(|c| c.allowed_operators.clone()),
            max_tokens: self.config.default_max_tokens,
        }
    }

    fn build_tool_schemas(&self, config: &ResolvedConfig) -> Vec<ToolSchema> {
        let mut schemas: Vec<ToolSchema> = self
            .tools
            .iter()
            .map(|tool| ToolSchema {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect();

        // Add effect tool schemas
        schemas.extend(effect_tool_schemas());

        // Filter by allowed_operators if specified
        if let Some(allowed) = &config.allowed_operators {
            schemas.retain(|s| allowed.contains(&s.name));
        }

        schemas
    }

    async fn assemble_context(
        &self,
        input: &OperatorInput,
    ) -> Result<Vec<Message>, OperatorError> {
        let mut messages = Vec::new();

        // Read history from state if session is present
        if let Some(session) = &input.session {
            let scope = Scope::Session(session.clone());
            match self.state_reader.read(&scope, "messages").await {
                Ok(Some(history)) => {
                    if let Ok(history_messages) =
                        serde_json::from_value::<Vec<ProviderMessage>>(history)
                    {
                        messages = history_messages
                            .into_iter()
                            .map(Message::from)
                            .collect();
                    }
                }
                Ok(None) => {} // No history yet
                Err(_) => {}   // State read errors are non-fatal
            }
        }

        // Add the new user message
        messages.push(Message::new(L0Role::User, input.message.clone()));

        Ok(messages)
    }

    fn try_as_effect(&self, name: &str, input: &serde_json::Value) -> Option<Effect> {
        match name {
            "write_memory" => {
                let scope_str = input.get("scope")?.as_str()?;
                let key = input.get("key")?.as_str()?.to_string();
                let value = input.get("value")?.clone();
                let scope = parse_scope(scope_str);
                Some(Effect::WriteMemory {
                    scope,
                    key,
                    value,
                    tier: None,
                    lifetime: None,
                    content_kind: None,
                    salience: None,
                    ttl: None,
                })
            }
            "delete_memory" => {
                let scope_str = input.get("scope")?.as_str()?;
                let key = input.get("key")?.as_str()?.to_string();
                let scope = parse_scope(scope_str);
                Some(Effect::DeleteMemory { scope, key })
            }
            "delegate" => {
                let agent = input.get("agent")?.as_str()?;
                let message = input.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let delegate_input =
                    OperatorInput::new(Content::text(message), layer0::operator::TriggerType::Task);
                Some(Effect::Delegate {
                    agent: AgentId::new(agent),
                    input: Box::new(delegate_input),
                })
            }
            "handoff" => {
                let agent = input.get("agent")?.as_str()?;
                let state = input
                    .get("state")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                Some(Effect::Handoff {
                    agent: AgentId::new(agent),
                    state,
                })
            }
            "signal" => {
                let target = input.get("target")?.as_str()?;
                let signal_type = input
                    .get("signal_type")
                    .and_then(|s| s.as_str())
                    .unwrap_or("default");
                let data = input
                    .get("data")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                Some(Effect::Signal {
                    target: WorkflowId::new(target),
                    payload: SignalPayload::new(signal_type, data),
                })
            }
            _ => None,
        }
    }

    fn build_metadata(
        &self,
        tokens_in: u64,
        tokens_out: u64,
        cost: Decimal,
        turns_used: u32,
        sub_dispatches: Vec<SubDispatchRecord>,
        duration: DurationMs,
    ) -> OperatorMetadata {
        let mut meta = OperatorMetadata::default();
        meta.tokens_in = tokens_in;
        meta.tokens_out = tokens_out;
        meta.cost = cost;
        meta.turns_used = turns_used;
        meta.sub_dispatches = sub_dispatches;
        meta.duration = duration;
        meta
    }

    fn make_output(
        message: Content,
        exit_reason: ExitReason,
        metadata: OperatorMetadata,
        effects: Vec<Effect>,
    ) -> OperatorOutput {
        let mut output = OperatorOutput::new(message, exit_reason);
        output.metadata = metadata;
        output.effects = effects;
        output
    }

    fn build_loop_state(
        &self,
        tokens_in: u64,
        tokens_out: u64,
        cost: Decimal,
        turns_completed: u32,
        elapsed: DurationMs,
    ) -> LoopState {
        LoopState {
            tokens_in,
            tokens_out,
            cost,
            turns_completed,
            elapsed,
        }
    }
    /// Poll the steering source and dispatch interceptor events.
    ///
    /// Returns injected messages (after interceptor approval) and context commands (unconditional).
    /// Context commands bypass the interceptor — they are direct buffer manipulation.
    async fn poll_steering(
        &self,
        ti: u64,
        to: u64,
        cost: Decimal,
        turns: u32,
        elapsed: DurationMs,
    ) -> (Vec<ProviderMessage>, Vec<ContextCommand>) {
        let Some(s) = &self.steering else {
            return (vec![], vec![]);
        };
        let commands = s.drain();
        if commands.is_empty() {
            return (vec![], vec![]);
        }
        let mut msgs_to_inject = Vec::new();
        let mut ctx_cmds = Vec::new();
        for cmd in commands {
            match cmd {
                SteeringCommand::Message(msg) => msgs_to_inject.push(msg),
                SteeringCommand::Context(cmd) => ctx_cmds.push(cmd),
            }
        }
        if msgs_to_inject.is_empty() {
            return (vec![], ctx_cmds);
        }
        if let Some(ref interceptor) = self.interceptor {
            let state = self.build_loop_state(ti, to, cost, turns, elapsed);
            let msg_strs: Vec<String> = msgs_to_inject.iter().map(|m| format!("{:?}", m)).collect();
            if let ReactAction::Halt { .. } =
                interceptor.pre_steering_inject(&state, &msg_strs).await
            {
                return (vec![], ctx_cmds);
            }
        }
        (msgs_to_inject, ctx_cmds)
    }
}

/// Apply a list of context manipulation commands to the message buffer.
///
/// Commands execute unconditionally — they bypass the `PreSteeringInject` hook.
pub(crate) fn apply_context_commands(
    messages: &mut Vec<Message>,
    cmds: Vec<ContextCommand>,
) {
    for cmd in cmds {
        match cmd {
            ContextCommand::Pin { message_index } => {
                if let Some(msg) = messages.get_mut(message_index) {
                    msg.meta.policy = layer0::CompactionPolicy::Pinned;
                }
            }
            ContextCommand::DropOldest { count } => {
                let droppable: Vec<usize> = messages
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !matches!(m.meta.policy, layer0::CompactionPolicy::Pinned))
                    .map(|(i, _)| i)
                    .take(count)
                    .collect();
                for i in droppable.into_iter().rev() {
                    messages.remove(i);
                }
            }
            ContextCommand::ClearWorking => {
                messages.retain(|m| matches!(m.meta.policy, layer0::CompactionPolicy::Pinned));
            }
            ContextCommand::SaveSnapshot { path } => match serde_json::to_vec(messages) {
                Ok(data) => {
                    if let Err(e) = std::fs::write(&path, &data) {
                        eprintln!(
                            "[steering] SaveSnapshot write failed: path={}, error={}",
                            path.display(),
                            e
                        );
                    }
                }
                Err(e) => eprintln!("[steering] SaveSnapshot serialization failed: {}", e),
            },
            ContextCommand::LoadSnapshot { path } => match std::fs::read(&path) {
                Ok(data) => {
                    match serde_json::from_slice::<Vec<Message>>(&data) {
                        Ok(loaded) => {
                            messages.clear();
                            messages.extend(loaded);
                        }
                        Err(e) => eprintln!(
                            "[steering] LoadSnapshot deserialization failed: path={}, error={}",
                            path.display(),
                            e
                        ),
                    }
                }
                Err(e) => eprintln!(
                    "[steering] LoadSnapshot read failed: path={}, error={}",
                    path.display(),
                    e
                ),
            },
        }
    }
}

#[async_trait]
impl<P: Provider + 'static> Operator for ReactOperator<P> {
    #[tracing::instrument(skip_all, fields(trigger = ?input.trigger))]
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let start = Instant::now();
        tracing::info!("react loop starting");
        let config = self.resolve_config(&input);
        let mut messages = self.assemble_context(&input).await?;
        *self
            .current_context
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages.clone();
        let tools = self.build_tool_schemas(&config);

        let mut total_tokens_in: u64 = 0;
        let mut total_tokens_out: u64 = 0;
        let mut total_cost = Decimal::ZERO;
        let mut turns_used: u32 = 0;
        let mut dispatch_records: Vec<SubDispatchRecord> = vec![];
        let mut effects: Vec<Effect> = vec![];
        let mut last_content: Vec<ContentPart> = vec![];
        let mut total_sub_dispatches: u32 = 0;
        let mut recent_calls: std::collections::VecDeque<(String, u64)> =
            std::collections::VecDeque::new();

        loop {
            self.state_reader.clear_transient();
            turns_used += 1;

            // 1. Interceptor: PreInference
            if let Some(ref interceptor) = self.interceptor {
                let state = self.build_loop_state(
                    total_tokens_in,
                    total_tokens_out,
                    total_cost,
                    turns_used - 1,
                    DurationMs::from(start.elapsed()),
                );
                if let ReactAction::Halt { reason } = interceptor.pre_inference(&state).await {
                    return Ok(Self::make_output(
                        parts_to_content(&last_content),
                        ExitReason::InterceptorHalt { reason },
                        self.build_metadata(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
            }

            // 2. Build ProviderRequest
            let request = ProviderRequest {
                model: config.model.clone(),
                messages: messages.iter().map(message_to_provider).collect(),
                tools: tools.clone(),
                max_tokens: Some(config.max_tokens),
                temperature: None,
                system: Some(config.system.clone()),
                extra: input.metadata.clone(),
            };

            // Apply model selector if configured
            let request = if let Some(sel) = &self.config.model_selector {
                let mut req = request;
                if let Some(model) = sel(&req) {
                    req.model = Some(model);
                }
                req
            } else {
                request
            };

            // 3. Call provider
            let response = self.provider.complete(request).await.map_err(|e| {
                if e.is_retryable() {
                    OperatorError::Retryable(e.to_string())
                } else {
                    OperatorError::Model(e.to_string())
                }
            })?;

            // 4. Interceptor: PostInference
            if let Some(ref interceptor) = self.interceptor {
                let state = self.build_loop_state(
                    total_tokens_in + response.usage.input_tokens,
                    total_tokens_out + response.usage.output_tokens,
                    total_cost + response.cost.unwrap_or(Decimal::ZERO),
                    turns_used,
                    DurationMs::from(start.elapsed()),
                );
                let resp_content = parts_to_content(&response.content);
                if let ReactAction::Halt { reason } =
                    interceptor.post_inference(&state, &resp_content).await
                {
                    return Ok(Self::make_output(
                        parts_to_content(&response.content),
                        ExitReason::InterceptorHalt { reason },
                        self.build_metadata(
                            total_tokens_in + response.usage.input_tokens,
                            total_tokens_out + response.usage.output_tokens,
                            total_cost + response.cost.unwrap_or(Decimal::ZERO),
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
            }

            // 5. Aggregate tokens + cost
            total_tokens_in += response.usage.input_tokens;
            total_tokens_out += response.usage.output_tokens;
            if let Some(cost) = response.cost {
                total_cost += cost;
            }

            last_content.clone_from(&response.content);

            // 6. Check StopReason
            match response.stop_reason {
                StopReason::MaxTokens => {
                    return Err(OperatorError::Model("output truncated (max_tokens)".into()));
                }
                StopReason::ContentFilter => {
                    return Ok(Self::make_output(
                        parts_to_content(&response.content),
                        ExitReason::SafetyStop {
                            reason: "content_filter".into(),
                        },
                        self.build_metadata(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
                StopReason::EndTurn => {
                    return Ok(Self::make_output(
                        parts_to_content(&response.content),
                        ExitReason::Complete,
                        self.build_metadata(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
                StopReason::ToolUse => {
                    // Continue to tool execution below
                }
            }

            // 7. Tool execution
            // Add assistant message to context
            messages.push(Message::new(L0Role::Assistant, parts_to_content(&response.content)));

            let mut dispatch_results: Vec<ContentPart> = Vec::new();
            // Use planner to decide batches. Build (id,name,input) vector first.
            let planned = {
                let calls: Vec<(String, String, serde_json::Value)> = response
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::ToolUse { id, name, input } => {
                            Some((id.clone(), name.clone(), input.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                self.planner.plan(&calls, self.decider.as_ref())
            };

            let mut _steered = false;
            'batches: for batch in planned {
                match batch {
                    BatchItem::Shared(call_group) => {
                        // Pre-batch steering poll
                        {
                            let (injected, ctx_cmds) = self
                                .poll_steering(
                                    total_tokens_in,
                                    total_tokens_out,
                                    total_cost,
                                    turns_used,
                                    DurationMs::from(start.elapsed()),
                                )
                                .await;
                            apply_context_commands(&mut messages, ctx_cmds);
                            if !injected.is_empty() {
                                messages.extend(injected.into_iter().map(Message::from));
                                // All tools in this batch are skipped with placeholders
                                let skipped_names: Vec<String> =
                                    call_group.iter().map(|(_, n, _)| n.clone()).collect();
                                for (id, name, _input) in call_group.into_iter() {
                                    dispatch_results.push(ContentPart::ToolResult {
                                        tool_use_id: id,
                                        content: "Skipped due to steering".into(),
                                        is_error: false,
                                    });
                                    dispatch_records.push(SubDispatchRecord::new(
                                        &name,
                                        DurationMs::ZERO,
                                        false,
                                    ));
                                }
                                if !skipped_names.is_empty()
                                    && let Some(ref interceptor) = self.interceptor
                                {
                                    let state = self.build_loop_state(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    );
                                    interceptor.post_steering_skip(&state, &skipped_names).await;
                                }
                                _steered = true;
                                break 'batches;
                            }
                        }
                        // Execute shared tools sequentially to allow steering to interrupt mid-batch
                        let len = call_group.len();
                        for idx in 0..len {
                            // Pre-next-tool steering poll (after some tools completed)
                            if idx > 0 {
                                let (injected, ctx_cmds) = self
                                    .poll_steering(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    )
                                    .await;
                                apply_context_commands(&mut messages, ctx_cmds);
                                if !injected.is_empty() {
                                    messages.extend(injected.into_iter().map(Message::from));
                                    let skipped_names: Vec<String> = call_group
                                        .iter()
                                        .skip(idx)
                                        .map(|(_, n, _)| n.clone())
                                        .collect();
                                    for (rid, rname, _rinput) in
                                        call_group.iter().skip(idx).cloned()
                                    {
                                        dispatch_results.push(ContentPart::ToolResult {
                                            tool_use_id: rid,
                                            content: "Skipped due to steering".into(),
                                            is_error: false,
                                        });
                                        dispatch_records.push(SubDispatchRecord::new(
                                            &rname,
                                            DurationMs::ZERO,
                                            false,
                                        ));
                                    }
                                    if !skipped_names.is_empty()
                                        && let Some(ref interceptor) = self.interceptor
                                    {
                                        let state = self.build_loop_state(
                                            total_tokens_in,
                                            total_tokens_out,
                                            total_cost,
                                            turns_used,
                                            DurationMs::from(start.elapsed()),
                                        );
                                        interceptor
                                            .post_steering_skip(&state, &skipped_names)
                                            .await;
                                    }
                                    _steered = true;
                                }
                            }
                            let (id, name, dispatch_input) = call_group[idx].clone();
                            // Effects handled immediately
                            if let Some(effect) = self.try_as_effect(&name, &dispatch_input) {
                                effects.push(effect);
                                dispatch_results.push(ContentPart::ToolResult {
                                    tool_use_id: id,
                                    content: format!("{name} effect recorded."),
                                    is_error: false,
                                });
                                dispatch_records.push(SubDispatchRecord::new(
                                    &name,
                                    DurationMs::ZERO,
                                    true,
                                ));
                                // track effect tool call
                                total_sub_dispatches += 1;
                                {
                                    use std::hash::{Hash, Hasher};
                                    let mut hasher =
                                        std::collections::hash_map::DefaultHasher::new();
                                    dispatch_input.to_string().hash(&mut hasher);
                                    let cap = self
                                        .config
                                        .max_repeat_calls
                                        .map(|v| v as usize)
                                        .unwrap_or(0)
                                        .max(10);
                                    recent_calls.push_back((name.to_string(), hasher.finish()));
                                    while recent_calls.len() > cap {
                                        recent_calls.pop_front();
                                    }
                                }
                            } else {
                                let mut actual_input = dispatch_input.clone();
                                if let Some(ref interceptor) = self.interceptor {
                                    let state = self.build_loop_state(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    );
                                    match interceptor
                                        .pre_sub_dispatch(&state, &name, &dispatch_input)
                                        .await
                                    {
                                        SubDispatchAction::Halt { reason } => {
                                            return Ok(Self::make_output(
                                                parts_to_content(&last_content),
                                                ExitReason::InterceptorHalt { reason },
                                                self.build_metadata(
                                                    total_tokens_in,
                                                    total_tokens_out,
                                                    total_cost,
                                                    turns_used,
                                                    dispatch_records,
                                                    DurationMs::from(start.elapsed()),
                                                ),
                                                effects,
                                            ));
                                        }
                                        SubDispatchAction::Skip { reason } => {
                                            dispatch_results.push(ContentPart::ToolResult {
                                                tool_use_id: id,
                                                content: format!("Skipped: {reason}"),
                                                is_error: false,
                                            });
                                            dispatch_records.push(SubDispatchRecord::new(
                                                &name,
                                                DurationMs::ZERO,
                                                false,
                                            ));
                                            continue;
                                        }
                                        SubDispatchAction::ModifyInput { new_input } => {
                                            actual_input = new_input;
                                        }
                                        SubDispatchAction::Continue => {}
                                    }
                                }
                                // Execute via orchestrator dispatch
                                let tool_start = Instant::now();
                                let (mut result_content, is_error, success, duration) = {
                                    let orch_input = OperatorInput::new(
                                        Content::text(actual_input.to_string()),
                                        layer0::operator::TriggerType::Task,
                                    );
                                    match self
                                        .orchestrator
                                        .dispatch(&AgentId::new(&name), orch_input)
                                        .await
                                    {
                                        Ok(output) => {
                                            let text = output
                                                .message
                                                .as_text()
                                                .unwrap_or("null")
                                                .to_string();
                                            (
                                                text,
                                                false,
                                                true,
                                                DurationMs::from(tool_start.elapsed()),
                                            )
                                        }
                                        Err(OrchError::AgentNotFound(_)) => (
                                            neuron_tool::ToolError::NotFound(name.clone())
                                                .to_string(),
                                            true,
                                            false,
                                            DurationMs::from(tool_start.elapsed()),
                                        ),
                                        Err(e) => (
                                            e.to_string(),
                                            true,
                                            false,
                                            DurationMs::from(tool_start.elapsed()),
                                        ),
                                    }
                                };
                                if let Some(ref interceptor) = self.interceptor {
                                    let state = self.build_loop_state(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    );
                                    match interceptor
                                        .post_sub_dispatch(&state, &name, &result_content)
                                        .await
                                    {
                                        SubDispatchResult::Halt { reason } => {
                                            return Ok(Self::make_output(
                                                parts_to_content(&last_content),
                                                ExitReason::InterceptorHalt { reason },
                                                self.build_metadata(
                                                    total_tokens_in,
                                                    total_tokens_out,
                                                    total_cost,
                                                    turns_used,
                                                    dispatch_records,
                                                    DurationMs::from(start.elapsed()),
                                                ),
                                                effects,
                                            ));
                                        }
                                        SubDispatchResult::ModifyOutput { new_output } => {
                                            result_content = new_output;
                                        }
                                        SubDispatchResult::Continue => {}
                                    }
                                }
                                dispatch_results.push(ContentPart::ToolResult {
                                    tool_use_id: id,
                                    content: result_content,
                                    is_error,
                                });
                                // track regular tool call
                                total_sub_dispatches += 1;
                                {
                                    use std::hash::{Hash, Hasher};
                                    let mut hasher =
                                        std::collections::hash_map::DefaultHasher::new();
                                    actual_input.to_string().hash(&mut hasher);
                                    let cap = self
                                        .config
                                        .max_repeat_calls
                                        .map(|v| v as usize)
                                        .unwrap_or(0)
                                        .max(10);
                                    recent_calls.push_back((name.clone(), hasher.finish()));
                                    while recent_calls.len() > cap {
                                        recent_calls.pop_front();
                                    }
                                }
                                dispatch_records
                                    .push(SubDispatchRecord::new(name, duration, success));
                            }
                            // Mid-batch steering poll — skip remaining tools in this batch
                            {
                                let (injected, ctx_cmds) = self
                                    .poll_steering(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    )
                                    .await;
                                apply_context_commands(&mut messages, ctx_cmds);
                                if !injected.is_empty() {
                                    messages.extend(injected.into_iter().map(Message::from));
                                    if idx + 1 < len {
                                        let skipped_names: Vec<String> = call_group
                                            .iter()
                                            .skip(idx + 1)
                                            .map(|(_, n, _)| n.clone())
                                            .collect();
                                        for (rid, rname, _rinput) in
                                            call_group.iter().skip(idx + 1).cloned()
                                        {
                                            dispatch_results.push(ContentPart::ToolResult {
                                                tool_use_id: rid,
                                                content: "Skipped due to steering".into(),
                                                is_error: false,
                                            });
                                            dispatch_records.push(SubDispatchRecord::new(
                                                &rname,
                                                DurationMs::ZERO,
                                                false,
                                            ));
                                        }
                                        if !skipped_names.is_empty()
                                            && let Some(ref interceptor) = self.interceptor
                                        {
                                            let state = self.build_loop_state(
                                                total_tokens_in,
                                                total_tokens_out,
                                                total_cost,
                                                turns_used,
                                                DurationMs::from(start.elapsed()),
                                            );
                                            interceptor
                                                .post_steering_skip(&state, &skipped_names)
                                                .await;
                                        }
                                        break 'batches;
                                    }
                                }
                            }
                        }
                        // Post-batch steering poll
                        {
                            let (injected, ctx_cmds) = self
                                .poll_steering(
                                    total_tokens_in,
                                    total_tokens_out,
                                    total_cost,
                                    turns_used,
                                    DurationMs::from(start.elapsed()),
                                )
                                .await;
                            apply_context_commands(&mut messages, ctx_cmds);
                            if !injected.is_empty() {
                                messages.extend(injected.into_iter().map(Message::from));
                                _steered = true;
                                break 'batches;
                            }
                        }
                    }
                    BatchItem::Exclusive((id, name, dispatch_input)) => {
                        // Pre-exclusive steering poll
                        {
                            let (injected, ctx_cmds) = self
                                .poll_steering(
                                    total_tokens_in,
                                    total_tokens_out,
                                    total_cost,
                                    turns_used,
                                    DurationMs::from(start.elapsed()),
                                )
                                .await;
                            apply_context_commands(&mut messages, ctx_cmds);
                            if !injected.is_empty() {
                                messages.extend(injected.into_iter().map(Message::from));
                                let skipped_names = vec![name.clone()];
                                dispatch_results.push(ContentPart::ToolResult {
                                    tool_use_id: id,
                                    content: "Skipped due to steering".into(),
                                    is_error: false,
                                });
                                dispatch_records.push(SubDispatchRecord::new(
                                    &name,
                                    DurationMs::ZERO,
                                    false,
                                ));
                                if let Some(ref interceptor) = self.interceptor {
                                    let state = self.build_loop_state(
                                        total_tokens_in,
                                        total_tokens_out,
                                        total_cost,
                                        turns_used,
                                        DurationMs::from(start.elapsed()),
                                    );
                                    interceptor.post_steering_skip(&state, &skipped_names).await;
                                }
                                _steered = true;
                                break 'batches;
                            }
                        }
                        if let Some(effect) = self.try_as_effect(&name, &dispatch_input) {
                            effects.push(effect);
                            dispatch_results.push(ContentPart::ToolResult {
                                tool_use_id: id,
                                content: format!("{name} effect recorded."),
                                is_error: false,
                            });
                            dispatch_records.push(SubDispatchRecord::new(
                                &name,
                                DurationMs::ZERO,
                                true,
                            ));
                            // track effect tool call
                            total_sub_dispatches += 1;
                            {
                                use std::hash::{Hash, Hasher};
                                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                                dispatch_input.to_string().hash(&mut hasher);
                                let cap = self
                                    .config
                                    .max_repeat_calls
                                    .map(|v| v as usize)
                                    .unwrap_or(0)
                                    .max(10);
                                recent_calls.push_back((name.to_string(), hasher.finish()));
                                while recent_calls.len() > cap {
                                    recent_calls.pop_front();
                                }
                            }
                            continue;
                        }
                        let mut actual_input = dispatch_input.clone();
                        if let Some(ref interceptor) = self.interceptor {
                            let state = self.build_loop_state(
                                total_tokens_in,
                                total_tokens_out,
                                total_cost,
                                turns_used,
                                DurationMs::from(start.elapsed()),
                            );
                            match interceptor
                                .pre_sub_dispatch(&state, &name, &dispatch_input)
                                .await
                            {
                                SubDispatchAction::Halt { reason } => {
                                    return Ok(Self::make_output(
                                        parts_to_content(&last_content),
                                        ExitReason::InterceptorHalt { reason },
                                        self.build_metadata(
                                            total_tokens_in,
                                            total_tokens_out,
                                            total_cost,
                                            turns_used,
                                            dispatch_records,
                                            DurationMs::from(start.elapsed()),
                                        ),
                                        effects,
                                    ));
                                }
                                SubDispatchAction::Skip { reason } => {
                                    dispatch_results.push(ContentPart::ToolResult {
                                        tool_use_id: id,
                                        content: format!("Skipped: {reason}"),
                                        is_error: false,
                                    });
                                    dispatch_records.push(SubDispatchRecord::new(
                                        &name,
                                        DurationMs::ZERO,
                                        false,
                                    ));
                                    continue;
                                }
                                SubDispatchAction::ModifyInput { new_input } => {
                                    actual_input = new_input;
                                }
                                SubDispatchAction::Continue => {}
                            }
                        }
                        let tool_start = Instant::now();
                        // Execute via orchestrator dispatch
                        let (mut result_content, is_error, success, tool_duration) = {
                            let orch_input = OperatorInput::new(
                                Content::text(actual_input.to_string()),
                                layer0::operator::TriggerType::Task,
                            );
                            match self
                                .orchestrator
                                .dispatch(&AgentId::new(&name), orch_input)
                                .await
                            {
                                Ok(output) => {
                                    let text =
                                        output.message.as_text().unwrap_or("null").to_string();
                                    (text, false, true, DurationMs::from(tool_start.elapsed()))
                                }
                                Err(OrchError::AgentNotFound(_)) => (
                                    neuron_tool::ToolError::NotFound(name.clone()).to_string(),
                                    true,
                                    false,
                                    DurationMs::from(tool_start.elapsed()),
                                ),
                                Err(e) => (
                                    e.to_string(),
                                    true,
                                    false,
                                    DurationMs::from(tool_start.elapsed()),
                                ),
                            }
                        };
                        if let Some(ref interceptor) = self.interceptor {
                            let state = self.build_loop_state(
                                total_tokens_in,
                                total_tokens_out,
                                total_cost,
                                turns_used,
                                DurationMs::from(start.elapsed()),
                            );
                            match interceptor
                                .post_sub_dispatch(&state, &name, &result_content)
                                .await
                            {
                                SubDispatchResult::Halt { reason } => {
                                    return Ok(Self::make_output(
                                        parts_to_content(&last_content),
                                        ExitReason::InterceptorHalt { reason },
                                        self.build_metadata(
                                            total_tokens_in,
                                            total_tokens_out,
                                            total_cost,
                                            turns_used,
                                            dispatch_records,
                                            DurationMs::from(start.elapsed()),
                                        ),
                                        effects,
                                    ));
                                }
                                SubDispatchResult::ModifyOutput { new_output } => {
                                    result_content = new_output;
                                }
                                SubDispatchResult::Continue => {}
                            }
                        }
                        dispatch_results.push(ContentPart::ToolResult {
                            tool_use_id: id,
                            content: result_content,
                            is_error,
                        });
                        // track tool call
                        total_sub_dispatches += 1;
                        {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            actual_input.to_string().hash(&mut hasher);
                            let cap = self
                                .config
                                .max_repeat_calls
                                .map(|v| v as usize)
                                .unwrap_or(0)
                                .max(10);
                            recent_calls.push_back((name.clone(), hasher.finish()));
                            while recent_calls.len() > cap {
                                recent_calls.pop_front();
                            }
                        }
                        dispatch_records.push(SubDispatchRecord::new(name, tool_duration, success));
                        // Post-exclusive steering poll
                        {
                            let (injected, ctx_cmds) = self
                                .poll_steering(
                                    total_tokens_in,
                                    total_tokens_out,
                                    total_cost,
                                    turns_used,
                                    DurationMs::from(start.elapsed()),
                                )
                                .await;
                            apply_context_commands(&mut messages, ctx_cmds);
                            if !injected.is_empty() {
                                messages.extend(injected.into_iter().map(Message::from));
                                _steered = true;
                                break 'batches;
                            }
                        }
                    }
                }
            }

            // Add tool results as user message
            messages.push(Message::new(L0Role::User, parts_to_content(&dispatch_results)));
            *self
                .current_context
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = messages.clone();

            // 8. Interceptor: ExitCheck — safety halt fires before any limit checks
            if let Some(ref interceptor) = self.interceptor {
                let state = self.build_loop_state(
                    total_tokens_in,
                    total_tokens_out,
                    total_cost,
                    turns_used,
                    DurationMs::from(start.elapsed()),
                );
                if let ReactAction::Halt { reason } = interceptor.exit_check(&state).await {
                    return Ok(Self::make_output(
                        parts_to_content(&last_content),
                        ExitReason::InterceptorHalt { reason },
                        self.build_metadata(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
            }

            // 9. Check limits
            // 9a. Step/loop limits
            if let Some(max_tc) = self.config.max_tool_calls {
                let threshold = (max_tc as f32 * 0.80) as u32;
                if total_sub_dispatches >= threshold
                    && total_sub_dispatches < max_tc
                    && let Some(ref sink) = self.budget_sink
                {
                    sink.on_budget_event(BudgetEvent::StepLimitApproaching {
                        agent: AgentId::new("react"),
                        current: total_sub_dispatches,
                        max: max_tc,
                    });
                }
            }

            if let Some(max_tc) = self.config.max_tool_calls
                && total_sub_dispatches >= max_tc
            {
                if let Some(ref sink) = self.budget_sink {
                    sink.on_budget_event(BudgetEvent::StepLimitReached {
                        agent: AgentId::new("react"),
                        total_sub_dispatches,
                    });
                }

                return Ok(Self::make_output(
                    parts_to_content(&last_content),
                    ExitReason::BudgetExhausted,
                    self.build_metadata(
                        total_tokens_in,
                        total_tokens_out,
                        total_cost,
                        turns_used,
                        dispatch_records,
                        DurationMs::from(start.elapsed()),
                    ),
                    effects,
                ));
            }
            if let Some(max_rep) = self.config.max_repeat_calls
                && max_rep > 0
                && recent_calls.len() >= max_rep as usize
            {
                let first = recent_calls.front().cloned();
                if recent_calls.iter().all(|c| Some(c) == first.as_ref()) {
                    if let Some(ref sink) = self.budget_sink {
                        sink.on_budget_event(BudgetEvent::LoopDetected {
                            agent: AgentId::new("react"),
                            operator_name: first
                                .as_ref()
                                .map(|(n, _)| n.clone())
                                .unwrap_or_default(),
                            consecutive_count: recent_calls.len() as u32,
                            max: max_rep,
                        });
                    }

                    return Ok(Self::make_output(
                        parts_to_content(&last_content),
                        ExitReason::Custom("stuck_detected".into()),
                        self.build_metadata(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            dispatch_records,
                            DurationMs::from(start.elapsed()),
                        ),
                        effects,
                    ));
                }
            }
            // 9b. MaxTurns
            if turns_used >= config.max_turns {
                return Ok(Self::make_output(
                    parts_to_content(&last_content),
                    ExitReason::MaxTurns,
                    self.build_metadata(
                        total_tokens_in,
                        total_tokens_out,
                        total_cost,
                        turns_used,
                        dispatch_records,
                        DurationMs::from(start.elapsed()),
                    ),
                    effects,
                ));
            }

            if let Some(max_cost) = &config.max_cost
                && total_cost >= *max_cost
            {
                return Ok(Self::make_output(
                    parts_to_content(&last_content),
                    ExitReason::BudgetExhausted,
                    self.build_metadata(
                        total_tokens_in,
                        total_tokens_out,
                        total_cost,
                        turns_used,
                        dispatch_records,
                        DurationMs::from(start.elapsed()),
                    ),
                    effects,
                ));
            }

            if let Some(max_duration) = &config.max_duration {
                let threshold = max_duration.to_std().mul_f32(0.80);
                if start.elapsed() >= threshold
                    && start.elapsed() < max_duration.to_std()
                    && let Some(ref sink) = self.budget_sink
                {
                    sink.on_budget_event(BudgetEvent::TimeoutApproaching {
                        agent: AgentId::new("react"),
                        elapsed: DurationMs::from(start.elapsed()),
                        max_duration: *max_duration,
                    });
                }
            }

            if let Some(max_duration) = &config.max_duration
                && start.elapsed() >= max_duration.to_std()
            {
                if let Some(ref sink) = self.budget_sink {
                    sink.on_budget_event(BudgetEvent::TimeoutReached {
                        agent: AgentId::new("react"),
                        elapsed: DurationMs::from(start.elapsed()),
                    });
                }

                return Ok(Self::make_output(
                    parts_to_content(&last_content),
                    ExitReason::Timeout,
                    self.build_metadata(
                        total_tokens_in,
                        total_tokens_out,
                        total_cost,
                        turns_used,
                        dispatch_records,
                        DurationMs::from(start.elapsed()),
                    ),
                    effects,
                ));
            }

            // 10. Context compaction
            let effective_limit =
                (config.max_tokens as f32 * 4.0 * (1.0 - self.config.compaction_reserve_pct))
                    as usize;
            let total_estimated: usize = messages.iter().map(|m| m.estimated_tokens()).sum();
            if total_estimated > effective_limit
                && let Some(ref compactor) = self.compactor
            {
                    // Interceptor: PreCompaction
                    let should_compact = if let Some(ref interceptor) = self.interceptor {
                        let state = self.build_loop_state(
                            total_tokens_in,
                            total_tokens_out,
                            total_cost,
                            turns_used,
                            DurationMs::from(start.elapsed()),
                        );
                        matches!(
                            interceptor.pre_compaction(&state, messages.len()).await,
                            ReactAction::Continue
                        )
                    } else {
                        true
                    };
                    if !should_compact {
                        if let Some(ref sink) = self.compaction_sink {
                            sink.on_compaction_event(CompactionEvent::CompactionSkipped {
                                agent: AgentId::new("react"),
                                reason: "blocked by interceptor".into(),
                            });
                        }
                    } else {
                        let before_count = messages.len() as u32;
                        let before_tokens = total_estimated as u64;
                        let compacted = compactor(&messages);
                        let after_count = compacted.len() as u32;
                        let after_tokens: u64 = compacted.iter().map(|m| m.estimated_tokens() as u64).sum();
                        if let Some(ref sink) = self.compaction_sink {
                            sink.on_compaction_event(CompactionEvent::CompactionQuality {
                                agent: AgentId::new("react"),
                                tokens_before: before_tokens,
                                tokens_after: after_tokens,
                                items_preserved: after_count,
                                items_lost: before_count.saturating_sub(after_count),
                            });
                        }
                        messages = compacted;
                        *self
                            .last_compaction_removed
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) =
                            before_count.saturating_sub(after_count) as usize;
                        *self
                            .current_context
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = messages.clone();
                        // Interceptor: PostCompaction
                        if let Some(ref interceptor) = self.interceptor {
                            let state = self.build_loop_state(
                                total_tokens_in,
                                total_tokens_out,
                                total_cost,
                                turns_used,
                                DurationMs::from(start.elapsed()),
                            );
                            interceptor
                                .post_compaction(
                                    &state,
                                    before_count as usize,
                                    after_count as usize,
                                )
                                .await;
                        }
                    }
                }

            // 11. Loop repeats
        }
    }
}

/// Schemas for effect tools that the model can call.
fn effect_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "write_memory".into(),
            description: "Write a value to persistent memory.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "description": "Memory scope (e.g. 'global', 'session:id')"},
                    "key": {"type": "string", "description": "Memory key"},
                    "value": {"description": "Value to store"}
                },
                "required": ["scope", "key", "value"]
            }),
        },
        ToolSchema {
            name: "delete_memory".into(),
            description: "Delete a value from persistent memory.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "description": "Memory scope"},
                    "key": {"type": "string", "description": "Memory key"}
                },
                "required": ["scope", "key"]
            }),
        },
        ToolSchema {
            name: "delegate".into(),
            description: "Delegate a task to another agent.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": {"type": "string", "description": "Agent ID to delegate to"},
                    "message": {"type": "string", "description": "Task description for the agent"}
                },
                "required": ["agent", "message"]
            }),
        },
        ToolSchema {
            name: "handoff".into(),
            description: "Hand off the conversation to another agent.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": {"type": "string", "description": "Agent ID to hand off to"},
                    "state": {"description": "State to pass to the next agent"}
                },
                "required": ["agent"]
            }),
        },
        ToolSchema {
            name: "signal".into(),
            description: "Send a signal to another workflow.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "Target workflow ID"},
                    "signal_type": {"type": "string", "description": "Signal type identifier"},
                    "data": {"description": "Signal payload data"}
                },
                "required": ["target"]
            }),
        },
    ]
}

/// Parse a scope string into a layer0 Scope.
fn parse_scope(s: &str) -> Scope {
    if s == "global" {
        return Scope::Global;
    }
    if let Some(id) = s.strip_prefix("session:") {
        return Scope::Session(layer0::SessionId::new(id));
    }
    if let Some(id) = s.strip_prefix("workflow:") {
        return Scope::Workflow(layer0::WorkflowId::new(id));
    }
    Scope::Custom(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_tool::ToolRegistry;
    use neuron_turn::provider::ProviderError;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // -- Mock Provider --

    struct MockProvider {
        responses: Mutex<VecDeque<ProviderResponse>>,
        call_count: AtomicUsize,
    }

    impl MockProvider {
        fn new(responses: Vec<ProviderResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                call_count: AtomicUsize::new(0),
            }
        }
    }

    impl Provider for MockProvider {
        fn complete(
            &self,
            _request: ProviderRequest,
        ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send
        {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("MockProvider: no more responses queued");
            async move { Ok(response) }
        }
    }

    // -- Mock StateReader --

    struct NullStateReader;

    #[async_trait]
    impl layer0::StateReader for NullStateReader {
        async fn read(
            &self,
            _scope: &Scope,
            _key: &str,
        ) -> Result<Option<serde_json::Value>, layer0::StateError> {
            Ok(None)
        }
        async fn list(
            &self,
            _scope: &Scope,
            _prefix: &str,
        ) -> Result<Vec<String>, layer0::StateError> {
            Ok(vec![])
        }
        async fn search(
            &self,
            _scope: &Scope,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<layer0::state::SearchResult>, layer0::StateError> {
            Ok(vec![])
        }
    }

    // -- Mock Tool --

    struct EchoTool;

    impl neuron_tool::ToolDyn for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, neuron_tool::ToolError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
    }

    // -- Helpers --

    fn simple_text_response(text: &str) -> ProviderResponse {
        ProviderResponse {
            content: vec![ContentPart::Text {
                text: text.to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            model: "mock-model".into(),
            cost: Some(Decimal::new(1, 4)), // $0.0001
            truncated: None,
        }
    }

    fn tool_use_response(
        tool_id: &str,
        operator_name: &str,
        input: serde_json::Value,
    ) -> ProviderResponse {
        ProviderResponse {
            content: vec![ContentPart::ToolUse {
                id: tool_id.to_string(),
                name: operator_name.to_string(),
                input,
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                ..Default::default()
            },
            model: "mock-model".into(),
            cost: Some(Decimal::new(2, 4)), // $0.0002
            truncated: None,
        }
    }

    fn make_op<P: Provider>(provider: P) -> ReactOperator<P> {
        ReactOperator::new(
            provider,
            ToolRegistry::new(),
            Arc::new(NullStateReader),
            ReactConfig::default(),
        )
    }

    fn make_op_with_tools<P: Provider>(provider: P, tools: ToolRegistry) -> ReactOperator<P> {
        ReactOperator::new(
            provider,
            tools,
            Arc::new(NullStateReader),
            ReactConfig::default(),
        )
    }

    fn simple_input(text: &str) -> OperatorInput {
        OperatorInput::new(Content::text(text), layer0::operator::TriggerType::User)
    }

    // -- Tests --

    #[tokio::test]
    async fn simple_completion() {
        let provider = MockProvider::new(vec![simple_text_response("Hello!")]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Hi")).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Hello!");
        assert_eq!(output.metadata.turns_used, 1);
        assert_eq!(output.metadata.tokens_in, 10);
        assert_eq!(output.metadata.tokens_out, 5);
        assert!(output.effects.is_empty());
    }

    #[tokio::test]
    async fn tool_use_and_followup() {
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({"msg": "test"})),
            simple_text_response("Done."),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = make_op_with_tools(provider, tools);

        let output = op.execute(simple_input("Use echo")).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.metadata.turns_used, 2);
        assert_eq!(output.metadata.sub_dispatches.len(), 1);
        assert_eq!(output.metadata.sub_dispatches[0].name, "echo");
    }

    #[tokio::test]
    async fn unknown_tool_returns_error_result() {
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "nonexistent_tool", json!({})),
            simple_text_response("Got an error."),
        ]);
        let op = make_op(provider);

        // Should not panic — unknown tool produces an error result but loop continues
        let output = op.execute(simple_input("Use nonexistent")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        // The tool call was recorded
        assert_eq!(output.metadata.sub_dispatches.len(), 1);
    }

    #[tokio::test]
    async fn max_turns_enforced() {
        // Provider always returns ToolUse — loop should hit max_turns limit
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({})),
            tool_use_response("tu_2", "echo", json!({})),
            tool_use_response("tu_3", "echo", json!({})),
            simple_text_response("never reached"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));

        let mut op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 2,
                        ..Default::default()
                    },
                );
        // Avoid unused warning
        let _ = &mut op;

        let op = ReactOperator::new(
                    MockProvider::new(vec![
                        tool_use_response("tu_1", "echo", json!({})),
                        tool_use_response("tu_2", "echo", json!({})),
                        simple_text_response("never reached"),
                    ]),
                    {
                        let mut t = ToolRegistry::new();
                        t.register(Arc::new(EchoTool));
                        t
                    },
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 2,
                        ..Default::default()
                    },
                );

        let output = op.execute(simple_input("loop")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::MaxTurns);
        assert_eq!(output.metadata.turns_used, 2);
    }

    #[tokio::test]
    async fn budget_exhausted() {
        // Two calls, each costing $0.0001, with max_cost = $0.00015
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                );

        let mut input = simple_input("spend");
        let mut tc = layer0::operator::OperatorConfig::default();
        tc.max_cost = Some(Decimal::new(15, 5)); // $0.00015
        input.config = Some(tc);

        let output = op.execute(input).await.unwrap();
        // First call costs $0.0002 > $0.00015, so BudgetExhausted after second call
        assert_eq!(output.exit_reason, ExitReason::BudgetExhausted);
    }

    #[tokio::test]
    async fn max_tokens_returns_model_error() {
        let provider = MockProvider::new(vec![ProviderResponse {
            content: vec![],
            stop_reason: StopReason::MaxTokens,
            usage: TokenUsage::default(),
            model: "mock".into(),
            cost: None,
            truncated: None,
        }]);
        let op = make_op(provider);

        let result = op.execute(simple_input("Hi")).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            OperatorError::Model(msg) => assert!(msg.contains("max_tokens")),
            other => panic!("expected OperatorError::Model, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn content_filter_returns_safety_stop() {
        let provider = MockProvider::new(vec![ProviderResponse {
            content: vec![],
            stop_reason: StopReason::ContentFilter,
            usage: TokenUsage::default(),
            model: "mock".into(),
            cost: None,
            truncated: None,
        }]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Hi")).await.unwrap();
        match output.exit_reason {
            ExitReason::SafetyStop { reason } => assert_eq!(reason, "content_filter"),
            other => panic!("expected SafetyStop, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn cost_aggregated_across_turns() {
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = make_op_with_tools(provider, tools);

        let output = op.execute(simple_input("Hi")).await.unwrap();

        // First call: $0.0002, second call: $0.0001
        assert_eq!(output.metadata.cost, Decimal::new(3, 4));
        assert_eq!(output.metadata.tokens_in, 20);
        assert_eq!(output.metadata.tokens_out, 20);
    }

    #[tokio::test]
    async fn operator_config_overrides_defaults() {
        let provider = MockProvider::new(vec![simple_text_response("Hi")]);
        let op = make_op(provider);

        let mut input = simple_input("test");
        let mut tc = layer0::operator::OperatorConfig::default();
        tc.system_addendum = Some("Be concise.".into());
        tc.model = Some("custom-model".into());
        tc.max_turns = Some(5);
        input.config = Some(tc);

        let output = op.execute(input).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[tokio::test]
    async fn effect_tool_write_memory() {
        let provider = MockProvider::new(vec![
            // Model calls write_memory
            ProviderResponse {
                content: vec![ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "write_memory".into(),
                    input: json!({"scope": "global", "key": "test", "value": "hello"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                model: "mock".into(),
                cost: None,
                truncated: None,
            },
            simple_text_response("Memory written."),
        ]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Write memory")).await.unwrap();

        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::WriteMemory { key, .. } => assert_eq!(key, "test"),
            _ => panic!("expected WriteMemory"),
        }
    }

    #[test]
    fn parse_scope_variants() {
        assert_eq!(parse_scope("global"), Scope::Global);
        assert_eq!(
            parse_scope("session:abc"),
            Scope::Session(layer0::SessionId::new("abc"))
        );
        assert_eq!(
            parse_scope("workflow:wf1"),
            Scope::Workflow(layer0::WorkflowId::new("wf1"))
        );
        match parse_scope("other") {
            Scope::Custom(s) => assert_eq!(s, "other"),
            _ => panic!("expected Custom"),
        }
    }

    #[tokio::test]
    async fn effect_tool_delete_memory() {
        let provider = MockProvider::new(vec![
            ProviderResponse {
                content: vec![ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "delete_memory".into(),
                    input: json!({"scope": "global", "key": "old_key"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
                model: "mock".into(),
                cost: None,
                truncated: None,
            },
            simple_text_response("Deleted."),
        ]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Delete memory")).await.unwrap();
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::DeleteMemory { key, .. } => assert_eq!(key, "old_key"),
            _ => panic!("expected DeleteMemory"),
        }
    }

    #[tokio::test]
    async fn effect_tool_delegate() {
        let provider = MockProvider::new(vec![
            ProviderResponse {
                content: vec![ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "delegate".into(),
                    input: json!({"agent": "helper", "message": "do this task"}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
                model: "mock".into(),
                cost: None,
                truncated: None,
            },
            simple_text_response("Delegated."),
        ]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Delegate task")).await.unwrap();
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::Delegate { agent, input } => {
                assert_eq!(agent.as_str(), "helper");
                assert_eq!(input.message.as_text().unwrap(), "do this task");
            }
            _ => panic!("expected Delegate"),
        }
    }

    #[tokio::test]
    async fn effect_tool_handoff() {
        let provider = MockProvider::new(vec![
            ProviderResponse {
                content: vec![ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "handoff".into(),
                    input: json!({"agent": "specialist", "state": {"context": "data"}}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
                model: "mock".into(),
                cost: None,
                truncated: None,
            },
            simple_text_response("Handed off."),
        ]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Handoff")).await.unwrap();
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::Handoff { agent, state } => {
                assert_eq!(agent.as_str(), "specialist");
                assert_eq!(state["context"], "data");
            }
            _ => panic!("expected Handoff"),
        }
    }

    #[tokio::test]
    async fn effect_tool_signal() {
        let provider = MockProvider::new(vec![
            ProviderResponse {
                content: vec![ContentPart::ToolUse {
                    id: "tu_1".into(),
                    name: "signal".into(),
                    input: json!({"target": "workflow_1", "signal_type": "completed", "data": {"result": "ok"}}),
                }],
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
                model: "mock".into(),
                cost: None,
                truncated: None,
            },
            simple_text_response("Signal sent."),
        ]);
        let op = make_op(provider);

        let output = op.execute(simple_input("Signal")).await.unwrap();
        assert_eq!(output.effects.len(), 1);
        match &output.effects[0] {
            Effect::Signal { target, payload } => {
                assert_eq!(target.as_str(), "workflow_1");
                assert_eq!(payload.signal_type, "completed");
            }
            _ => panic!("expected Signal"),
        }
    }

    #[test]
    fn effect_tool_schemas_all_present() {
        let schemas = effect_tool_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"write_memory"));
        assert!(names.contains(&"delete_memory"));
        assert!(names.contains(&"delegate"));
        assert!(names.contains(&"handoff"));
        assert!(names.contains(&"signal"));
        assert_eq!(schemas.len(), 5);
    }

    #[test]
    fn react_operator_implements_operator_trait() {
        // Compile-time check: ReactOperator<MockProvider> implements Operator
        fn _assert_operator<T: Operator>() {}
        _assert_operator::<ReactOperator<MockProvider>>();
    }

    #[tokio::test]
    async fn react_operator_as_arc_dyn_operator() {
        // ReactOperator<P> can be used as Arc<dyn Operator>
        let provider = MockProvider::new(vec![simple_text_response("Hello!")]);
        let op: Arc<dyn Operator> = Arc::new(ReactOperator::new(
                    provider,
                    ToolRegistry::new(),
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                ));

        let output = op.execute(simple_input("Hi")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[tokio::test]
    async fn provider_retryable_error_maps_to_retryable() {
        struct ErrorProvider;
        impl Provider for ErrorProvider {
            #[allow(clippy::manual_async_fn)]
            fn complete(
                &self,
                _request: ProviderRequest,
            ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send
            {
                async { Err(ProviderError::RateLimited) }
            }
        }

        let op = ReactOperator::new(
                    ErrorProvider,
                    ToolRegistry::new(),
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                );

        let result = op.execute(simple_input("test")).await;
        assert!(matches!(result, Err(OperatorError::Retryable(_))));
    }

    #[tokio::test]
    async fn provider_call_count() {
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({})),
            tool_use_response("tu_2", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));

        struct CountingProvider {
            inner: MockProvider,
            count: std::sync::Arc<AtomicUsize>,
        }
        impl Provider for CountingProvider {
            #[allow(clippy::manual_async_fn)]
            fn complete(
                &self,
                request: ProviderRequest,
            ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send
            {
                self.count.fetch_add(1, Ordering::SeqCst);
                self.inner.complete(request)
            }
        }

        let counting_provider = CountingProvider {
            inner: MockProvider::new(vec![
                tool_use_response("tu_1", "echo", json!({})),
                tool_use_response("tu_2", "echo", json!({})),
                simple_text_response("Done"),
            ]),
            count: call_count.clone(),
        };

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = make_op_with_tools(counting_provider, tools);

        op.execute(simple_input("Multi-turn")).await.unwrap();
        // Only counting_provider was called — provider was called 3 times
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
        // The unused `provider` variable should not cause issues
        drop(provider);
    }

    // -- Steering Mocks --
    struct MockSteering {
        seq: Mutex<VecDeque<Vec<ProviderMessage>>>,
        calls: AtomicUsize,
    }
    impl MockSteering {
        fn new(seq: Vec<Vec<ProviderMessage>>) -> Self {
            Self {
                seq: Mutex::new(seq.into()),
                calls: AtomicUsize::new(0),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    impl SteeringSource for MockSteering {
        fn drain(&self) -> Vec<SteeringCommand> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seq
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default()
                .into_iter()
                .map(SteeringCommand::Message)
                .collect()
        }
    }

    struct CountingEchoTool {
        hits: std::sync::Arc<AtomicUsize>,
    }
    impl CountingEchoTool {
        fn new(h: std::sync::Arc<AtomicUsize>) -> Self {
            Self { hits: h }
        }
    }
    impl neuron_tool::ToolDyn for CountingEchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input (counting)"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type":"object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, neuron_tool::ToolError>>
                    + Send
                    + '_,
            >,
        > {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
    }

    struct SharedOnlyDecider;
    impl ConcurrencyDecider for SharedOnlyDecider {
        fn concurrency(&self, operator_name: &str) -> Concurrency {
            if operator_name == "echo" {
                Concurrency::Shared
            } else {
                Concurrency::Exclusive
            }
        }
    }

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: Role::User,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }

    #[tokio::test]
    async fn steering_skips_remaining_shared_batch() {
        // Provider returns two shared tool uses in one response
        let first = ProviderResponse {
            content: vec![
                ContentPart::ToolUse {
                    id: "t1".into(),
                    name: "echo".into(),
                    input: json!({"n":1}),
                },
                ContentPart::ToolUse {
                    id: "t2".into(),
                    name: "echo".into(),
                    input: json!({"n":2}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        let steering = Arc::new(MockSteering::new(vec![
            vec![],                  // pre-batch: no steering
            vec![user_msg("STEER")], // after first result: trigger steering
        ]));
        let steering_ref = steering.clone();
        let op = make_op_with_tools(provider, tools)
            .with_planner(Box::new(BarrierPlanner))
            .with_concurrency_decider(Box::new(SharedOnlyDecider))
            .with_steering(steering);
        let output = op.execute(simple_input("run"));
        let output = output.await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert!(steering_ref.call_count() >= 1);
        // Only first tool executed
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(output.metadata.turns_used, 2);
        assert_eq!(output.metadata.sub_dispatches.len(), 2);
        assert_eq!(output.metadata.sub_dispatches[0].name, "echo");
        assert_eq!(output.metadata.sub_dispatches[1].name, "echo");
    }
    #[tokio::test]
    async fn steering_skips_before_exclusive() {
        // Single exclusive tool use, steering triggers before execution
        let first = ProviderResponse {
            content: vec![ContentPart::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        // Provider should be called again after steering injection
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        struct CountingProvider {
            inner: MockProvider,
            count: std::sync::Arc<AtomicUsize>,
        }
        impl Provider for CountingProvider {
            #[allow(clippy::manual_async_fn)]
            fn complete(
                &self,
                request: ProviderRequest,
            ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send
            {
                self.count.fetch_add(1, Ordering::SeqCst);
                self.inner.complete(request)
            }
        }
        let counting = CountingProvider {
            inner: MockProvider::new(vec![first, simple_text_response("Done")]),
            count: call_count.clone(),
        };
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        let steering = Arc::new(MockSteering::new(vec![
            vec![user_msg("STEER")], // pre-exclusive: trigger
        ]));
        let op = ReactOperator::new(
                    counting,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                )
        .with_steering(steering);
        let output = op.execute(simple_input("run"));
        let output = output.await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        // Tool implementation was never called
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // Provider called twice (two turns)
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(output.metadata.turns_used, 2);
    }

    #[tokio::test]
    async fn no_steering_default() {
        // Two shared tools; without steering both execute
        let first = ProviderResponse {
            content: vec![
                ContentPart::ToolUse {
                    id: "t1".into(),
                    name: "echo".into(),
                    input: json!({}),
                },
                ContentPart::ToolUse {
                    id: "t2".into(),
                    name: "echo".into(),
                    input: json!({}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        let op = make_op_with_tools(provider, tools)
            .with_planner(Box::new(BarrierPlanner))
            .with_concurrency_decider(Box::new(SharedOnlyDecider));
        let output = op.execute(simple_input("run"));
        let output = output.await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(output.metadata.sub_dispatches.len(), 2);
        assert_eq!(output.metadata.turns_used, 2);
    }

    // -- Streaming Tool + Hook tests --
    struct StreamEcho;
    impl neuron_tool::ToolDyn for StreamEcho {
        fn name(&self) -> &str {
            "stream_echo"
        }
        fn description(&self) -> &str {
            "Streams echo chunks"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type":"object"})
        }
        fn call(
            &self,
            _input: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, neuron_tool::ToolError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { Ok(serde_json::json!({"note":"non-stream fallback"})) })
        }
        fn maybe_streaming(&self) -> Option<&dyn neuron_tool::ToolDynStreaming> {
            Some(self)
        }
    }
    impl neuron_tool::ToolDynStreaming for StreamEcho {
        fn call_streaming<'a>(
            &'a self,
            _input: serde_json::Value,
            on_chunk: Box<dyn Fn(&str) + Send + Sync + 'a>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), neuron_tool::ToolError>> + Send + 'a>,
        > {
            Box::pin(async move {
                for ch in ["A", "B", "C"] {
                    on_chunk(ch);
                }
                Ok(())
            })
        }
    }

    struct CollectInterceptor {
        finals: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl ReactInterceptor for CollectInterceptor {
        async fn post_sub_dispatch(
            &self,
            _state: &LoopState,
            _tool_name: &str,
            result: &str,
        ) -> SubDispatchResult {
            self.finals.lock().unwrap().push(result.to_string());
            SubDispatchResult::Continue
        }
    }

    #[tokio::test]
    async fn streaming_tool_dispatched_via_orchestrator_no_chunks() {
        // After Phase 4.4, all tool dispatch goes through the orchestrator.
        // Orchestrator dispatch is request-response only (no streaming).
        // StreamEcho.call() returns {"note":"non-stream fallback"} — that
        // is what we get. No SubDispatchUpdate chunks are emitted.
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(StreamEcho));
        let chunks = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let finals = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let interceptor: Arc<dyn ReactInterceptor> = Arc::new(CollectInterceptor {
            finals: finals.clone(),
        });
        let op = ReactOperator::new(
                    MockProvider::new(vec![
                        tool_use_response("tu_s", "stream_echo", json!({})),
                        simple_text_response("OK"),
                    ]),
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                )
        .with_interceptor(interceptor);
        let _ = op.execute(simple_input("run")).await.unwrap();
        // No streaming chunks — orchestrator dispatch is request-response only
        let got_chunks = chunks.lock().unwrap().clone();
        assert!(
            got_chunks.is_empty(),
            "orchestrator dispatch does not emit streaming chunks"
        );
        // PostSubDispatch still fires with the non-streaming result
        let got_finals = finals.lock().unwrap().clone();
        assert_eq!(got_finals.len(), 1);
        // ToolOperator wraps the call() output as JSON text
        assert!(
            got_finals[0].contains("non-stream fallback"),
            "expected non-streaming fallback output, got: {}",
            got_finals[0]
        );
    }

    struct CountingSharedEchoTool {
        hits: std::sync::Arc<AtomicUsize>,
    }
    impl CountingSharedEchoTool {
        fn new(h: std::sync::Arc<AtomicUsize>) -> Self {
            Self { hits: h }
        }
    }
    impl neuron_tool::ToolDyn for CountingSharedEchoTool {
        fn name(&self) -> &str {
            "meta_echo"
        }
        fn description(&self) -> &str {
            "Echoes input (shared via metadata)"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type":"object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, neuron_tool::ToolError>>
                    + Send
                    + '_,
            >,
        > {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(json!({"echoed": input})) })
        }
        fn concurrency_hint(&self) -> neuron_tool::ToolConcurrencyHint {
            neuron_tool::ToolConcurrencyHint::Shared
        }
    }

    #[tokio::test]
    async fn metadata_concurrency_batches_shared() {
        // Two uses of the same tool should batch as Shared when metadata decider is used
        let first = ProviderResponse {
            content: vec![
                ContentPart::ToolUse {
                    id: "t1".into(),
                    name: "meta_echo".into(),
                    input: json!({}),
                },
                ContentPart::ToolUse {
                    id: "t2".into(),
                    name: "meta_echo".into(),
                    input: json!({}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 15,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingSharedEchoTool::new(hits.clone())));
        let op = make_op_with_tools(provider, tools)
            .with_planner(Box::new(BarrierPlanner))
            .with_metadata_concurrency();
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(output.metadata.sub_dispatches.len(), 2);
        assert_eq!(output.metadata.turns_used, 2);
    }
    // ── mock structures ──────────────────────────────────────────────

    /// A hook that always returns Halt when it fires at one of its points.
    struct ExitCheckInterceptor {
        reason: String,
    }
    #[async_trait]
    impl ReactInterceptor for ExitCheckInterceptor {
        async fn exit_check(&self, _state: &LoopState) -> ReactAction {
            ReactAction::Halt {
                reason: self.reason.clone(),
            }
        }
    }

    struct SteeringBlockInterceptor {
        reason: String,
    }
    #[async_trait]
    impl ReactInterceptor for SteeringBlockInterceptor {
        async fn pre_steering_inject(
            &self,
            _state: &LoopState,
            _messages: &[String],
        ) -> ReactAction {
            ReactAction::Halt {
                reason: self.reason.clone(),
            }
        }
    }

    struct RecordSkipInterceptor {
        recorded: std::sync::Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl ReactInterceptor for RecordSkipInterceptor {
        async fn post_steering_skip(&self, _state: &LoopState, skipped: &[String]) {
            let mut v = self.recorded.lock().unwrap();
            v.extend_from_slice(skipped);
        }
    }


    /// A provider that records the model field it receives.
    struct RecordingProvider {
        inner: MockProvider,
        models_seen: std::sync::Arc<Mutex<Vec<Option<String>>>>,
    }
    impl Provider for RecordingProvider {
        #[allow(clippy::manual_async_fn)]
        fn complete(
            &self,
            request: ProviderRequest,
        ) -> impl std::future::Future<
            Output = Result<ProviderResponse, neuron_turn::provider::ProviderError>,
        > + Send {
            self.models_seen.lock().unwrap().push(request.model.clone());
            self.inner.complete(request)
        }
    }

    // ── tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn exit_priority_hook_before_limits() {
        // ExitCheck guardrail fires → InterceptorHalt, even though MaxTurns would also fire.
        // max_turns=1, provider always returns ToolUse so the turn count reaches limit.
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({})),
            // Second response never reached
            simple_text_response("never"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let interceptor: Arc<dyn ReactInterceptor> = Arc::new(ExitCheckInterceptor {
            reason: "observer_halt_test".into(),
        });
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 1,
                        ..Default::default()
                    },
                )
        .with_interceptor(interceptor);
        let output = op.execute(simple_input("run")).await.unwrap();
        // Must be InterceptorHalt, not MaxTurns
        match &output.exit_reason {
            ExitReason::InterceptorHalt { reason } => {
                assert_eq!(reason, "observer_halt_test");
            }
            other => panic!("expected InterceptorHalt, got {:?}", other),
        }
    }

    // ── tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn steering_guardrail_blocks_injection() {
        // A Halt guardrail at PreSteeringInject must prevent injection.
        // The tool should still execute normally.
        let first = ProviderResponse {
            content: vec![ContentPart::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({"n": 1}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        // Steering always returns a message
        let steering = Arc::new(MockSteering::new(vec![vec![user_msg("blocked steering")]]));
        let interceptor: Arc<dyn ReactInterceptor> = Arc::new(SteeringBlockInterceptor {
            reason: "injection_blocked".into(),
        });
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                )
        .with_interceptor(interceptor)
        .with_steering(steering);
        let output = op.execute(simple_input("run")).await.unwrap();
        // Tool still executed (injection was blocked → tool ran)
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[tokio::test]
    async fn steering_observer_sees_skipped_operators() {
        // Observer at PostSteeringSkip receives the skipped tool names.
        let first = ProviderResponse {
            content: vec![ContentPart::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        // Steering fires immediately to skip the tool
        let steering = Arc::new(MockSteering::new(vec![vec![user_msg("STEER NOW")]]));
        let recorded = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let interceptor: Arc<dyn ReactInterceptor> = Arc::new(RecordSkipInterceptor {
            recorded: recorded.clone(),
        });
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig::default(),
                )
        .with_interceptor(interceptor)
        .with_steering(steering);
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        // Tool was skipped by steering
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // Observer received the skipped tool name
        let seen = recorded.lock().unwrap().clone();
        assert!(
            seen.contains(&"echo".to_string()),
            "expected 'echo' in {:?}",
            seen
        );
    }

    #[tokio::test]
    async fn steering_with_no_hooks_unchanged() {
        // Regression: steering with no hooks registered behaves the same as before.
        let first = ProviderResponse {
            content: vec![ContentPart::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            model: "mock".into(),
            cost: None,
            truncated: None,
        };
        let provider = MockProvider::new(vec![first, simple_text_response("Done")]);
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(CountingEchoTool::new(hits.clone())));
        // Steering fires to skip the tool (no hooks — should still skip)
        let steering = Arc::new(MockSteering::new(vec![vec![user_msg("STEER")]]));
        let op = make_op_with_tools(provider, tools).with_steering(steering);
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        // Tool was skipped
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // Steering injection happened (message visible in next turn)
        assert_eq!(output.metadata.turns_used, 2);
    }

    // ── tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compaction_reserve_enforced() {
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let compacted = Arc::new(Mutex::new(false));
        let compacted_clone = compacted.clone();
        let op = ReactOperator::new(
            provider,
            tools,
            Arc::new(NullStateReader),
            ReactConfig {
                default_max_tokens: 1, // tiny limit forces compaction
                compaction_reserve_pct: 0.20,
                ..Default::default()
            },
        )
        .with_compactor(move |msgs: &[Message]| {
            *compacted_clone.lock().unwrap() = true;
            msgs.to_vec()
        });
        op.execute(simple_input("Hi")).await.unwrap();
        assert!(*compacted.lock().unwrap(), "compactor should have been called");
    }

    // ── tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn max_tool_calls_exits_with_budget_exhausted() {
        // max_tool_calls = 3; model always requests tool calls.
        // After the 3rd tool call, exit with BudgetExhausted.
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({})),
            tool_use_response("t2", "echo", json!({})),
            tool_use_response("t3", "echo", json!({})),
            simple_text_response("never reached"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 10,
                        max_tool_calls: Some(3),
                        ..Default::default()
                    },
                );
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::BudgetExhausted);
        // 3 tool calls were made
        assert_eq!(output.metadata.sub_dispatches.len(), 3);
    }

    #[tokio::test]
    async fn max_repeat_calls_detects_stuck() {
        // max_repeat_calls = 2; model always calls same tool with same args.
        // After 2 consecutive identical calls, exit with Custom("stuck_detected").
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({"x": 1})),
            tool_use_response("t2", "echo", json!({"x": 1})),
            simple_text_response("never reached"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 10,
                        max_repeat_calls: Some(2),
                        ..Default::default()
                    },
                );
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(
            output.exit_reason,
            ExitReason::Custom("stuck_detected".into())
        );
    }

    #[tokio::test]
    async fn max_repeat_calls_different_args_no_trigger() {
        // max_repeat_calls = 2; model alternates args → no stuck detection.
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({"x": 1})),
            tool_use_response("t2", "echo", json!({"x": 2})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 10,
                        max_repeat_calls: Some(2),
                        ..Default::default()
                    },
                );
        let output = op.execute(simple_input("run")).await.unwrap();
        // No stuck detection — completes normally
        assert_eq!(output.exit_reason, ExitReason::Complete);
    }

    #[tokio::test]
    async fn both_limits_none_current_behavior() {
        // Regression: both max_tool_calls=None and max_repeat_calls=None.
        // Behavior unchanged — completes normally.
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({})),
            tool_use_response("t2", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        max_tool_calls: None,
                        max_repeat_calls: None,
                        ..Default::default()
                    },
                );
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.metadata.sub_dispatches.len(), 2);
    }

    // ── tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn model_selector_overrides_model() {
        // Selector returns Some("big-model") when messages.len() > 1 (after first turn),
        // None otherwise. Verify the provider sees the correct model each call.
        let models_seen = std::sync::Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let provider = RecordingProvider {
            inner: MockProvider::new(vec![
                tool_use_response("t1", "echo", json!({})),
                simple_text_response("Done"),
            ]),
            models_seen: models_seen.clone(),
        };
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_model: "default-model".into(),
                        ..Default::default()
                    },
                )
        .with_model_selector(|req: &ProviderRequest| {
            if req.messages.len() > 1 {
                Some("big-model".to_string())
            } else {
                None
            }
        });
        op.execute(simple_input("run")).await.unwrap();
        let seen = models_seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "expected 2 provider calls");
        // First call: 1 message → selector returns None → uses default
        assert_eq!(seen[0], Some("default-model".to_string()));
        // Second call: messages.len() > 1 → selector returns big-model
        assert_eq!(seen[1], Some("big-model".to_string()));
    }

    #[tokio::test]
    async fn no_model_selector_model_unchanged() {
        // Regression: without model_selector, model stays as configured.
        let models_seen = std::sync::Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let provider = RecordingProvider {
            inner: MockProvider::new(vec![simple_text_response("Hi")]),
            models_seen: models_seen.clone(),
        };
        let op = ReactOperator::new(
                    provider,
                    ToolRegistry::new(),
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_model: "my-model".into(),
                        ..Default::default()
                    },
                );
        op.execute(simple_input("Hi")).await.unwrap();
        let seen = models_seen.lock().unwrap().clone();
        assert_eq!(seen, vec![Some("my-model".to_string())]);
    }

    struct BudgetCollector {
        events: Arc<Mutex<Vec<BudgetEvent>>>,
    }

    impl BudgetEventSink for BudgetCollector {
        fn on_budget_event(&self, event: BudgetEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn budget_sink_receives_step_limit_reached() {
        // max_tool_calls = 2; model returns 2 tool calls then the limit fires.
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({})),
            tool_use_response("t2", "echo", json!({})),
            simple_text_response("never reached"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let events = Arc::new(Mutex::new(Vec::<BudgetEvent>::new()));
        let sink = Arc::new(BudgetCollector {
            events: events.clone(),
        });
        let op = ReactOperator::new(
                    provider,
                    tools,
                    Arc::new(NullStateReader),
                    ReactConfig {
                        default_max_turns: 10,
                        max_tool_calls: Some(2),
                        ..Default::default()
                    },
                )
        .with_budget_sink(sink);
        let output = op.execute(simple_input("run")).await.unwrap();
        assert_eq!(output.exit_reason, ExitReason::BudgetExhausted);
        let collected = events.lock().unwrap().clone();
        assert!(
            collected
                .iter()
                .any(|e| matches!(e, BudgetEvent::StepLimitReached { .. })),
            "expected StepLimitReached in {:?}",
            collected
        );
    }

    struct CompactionCollector {
        events: Arc<Mutex<Vec<CompactionEvent>>>,
    }

    impl CompactionEventSink for CompactionCollector {
        fn on_compaction_event(&self, event: CompactionEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn compaction_sink_receives_quality_event_on_success() {
        let provider = MockProvider::new(vec![
            tool_use_response("t1", "echo", json!({})),
            simple_text_response("Done"),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let events = Arc::new(Mutex::new(Vec::<CompactionEvent>::new()));
        let sink = Arc::new(CompactionCollector {
            events: events.clone(),
        });
        let op = ReactOperator::new(
            provider,
            tools,
            Arc::new(NullStateReader),
            ReactConfig {
                default_max_tokens: 1,
                ..Default::default()
            },
        )
        .with_compactor(|msgs: &[Message]| msgs.to_vec())
        .with_compaction_sink(sink);
        op.execute(simple_input("run")).await.unwrap();
        let collected = events.lock().unwrap().clone();
        assert!(
            collected
                .iter()
                .any(|e| matches!(e, CompactionEvent::CompactionQuality { .. })),
            "expected CompactionQuality in {:?}",
            collected
        );
    }

    // ── ContextCommand tests ───────────────────────────────────────────

    #[allow(dead_code)]
    /// Steering source that returns an arbitrary list of SteeringCommand.
    struct MockContextSteering {
        commands: Mutex<Vec<SteeringCommand>>,
    }
    #[allow(dead_code)]
    impl MockContextSteering {
        fn new(commands: Vec<SteeringCommand>) -> Self {
            Self {
                commands: Mutex::new(commands),
            }
        }
    }
    impl SteeringSource for MockContextSteering {
        fn drain(&self) -> Vec<SteeringCommand> {
            self.commands.lock().unwrap().drain(..).collect()
        }
    }

    fn make_user_msg(text: &str) -> Message {
        Message::new(L0Role::User, Content::text(text))
    }

    fn make_pinned_msg(text: &str) -> Message {
        Message::pinned(L0Role::User, Content::text(text))
    }

    #[test]
    fn test_pin_command() {
        let mut msgs = vec![make_user_msg("first"), make_user_msg("second")];
        apply_context_commands(&mut msgs, vec![ContextCommand::Pin { message_index: 0 }]);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Pinned);
        assert_eq!(msgs[1].meta.policy, layer0::CompactionPolicy::Normal);
    }

    #[test]
    fn test_pin_out_of_bounds_is_noop() {
        let mut msgs = vec![make_user_msg("only")];
        apply_context_commands(&mut msgs, vec![ContextCommand::Pin { message_index: 99 }]);
        // No panic, message unchanged
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Normal);
    }

    #[test]
    fn test_drop_oldest_skips_pinned() {
        // [normal, pinned, normal, normal] — drop 2 oldest non-pinned
        let mut msgs = vec![
            make_user_msg("m0"),
            make_pinned_msg("pinned"),
            make_user_msg("m2"),
            make_user_msg("m3"),
        ];
        apply_context_commands(&mut msgs, vec![ContextCommand::DropOldest { count: 2 }]);
        // m0 and m2 dropped (oldest non-pinned), pinned and m3 remain
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Pinned);
        assert_eq!(msgs[1].content.as_text().unwrap(), "m3");
    }

    #[test]
    fn test_drop_oldest_count_exceeds_droppable() {
        // Only 1 non-pinned; drop count=5 should not panic
        let mut msgs = vec![make_pinned_msg("pinned"), make_user_msg("normal")];
        apply_context_commands(&mut msgs, vec![ContextCommand::DropOldest { count: 5 }]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Pinned);
    }

    #[test]
    fn test_clear_working_keeps_pinned() {
        let mut msgs = vec![
            make_user_msg("normal1"),
            make_pinned_msg("pinned"),
            make_user_msg("normal2"),
        ];
        apply_context_commands(&mut msgs, vec![ContextCommand::ClearWorking]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Pinned);
    }

    #[test]
    fn test_save_load_snapshot_round_trip() {
        let original = vec![make_pinned_msg("pinned"), make_user_msg("normal")];
        let path =
            std::env::temp_dir().join(format!("neuron_test_snapshot_{}.json", std::process::id()));
        let mut msgs = original.clone();
        // Save
        apply_context_commands(
            &mut msgs,
            vec![ContextCommand::SaveSnapshot { path: path.clone() }],
        );
        assert!(path.exists(), "snapshot file should exist after save");
        // Corrupt the buffer
        msgs.clear();
        msgs.push(make_user_msg("corrupted"));
        // Load
        apply_context_commands(
            &mut msgs,
            vec![ContextCommand::LoadSnapshot { path: path.clone() }],
        );
        // Verify restored
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].meta.policy, layer0::CompactionPolicy::Pinned);
        assert_eq!(msgs[1].meta.policy, layer0::CompactionPolicy::Normal);
        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_snapshot_missing_file_is_noop() {
        let mut msgs = vec![make_user_msg("existing")];
        let path = std::path::PathBuf::from("/nonexistent/path/snapshot.json");
        apply_context_commands(&mut msgs, vec![ContextCommand::LoadSnapshot { path }]);
        // Buffer unchanged
        assert_eq!(msgs.len(), 1);
    }
    // ── ContextSnapshot tests ──────────────────────────────────────────────

    #[test]
    fn context_snapshot_empty_before_execute() {
        let provider = MockProvider::new(vec![]);
        let op = make_op(provider);
        let snap = op.context_snapshot();
        assert!(snap.messages.is_empty());
        assert_eq!(snap.token_count, 0);
        assert_eq!(snap.pinned_count, 0);
        assert_eq!(snap.last_compaction_removed, 0);
    }

    #[tokio::test]
    async fn context_snapshot_reflects_messages_after_turn() {
        let provider = MockProvider::new(vec![simple_text_response("Hello!")]);
        let op = make_op(provider);
        // Before execute: empty
        assert!(op.context_snapshot().messages.is_empty());
        op.execute(simple_input("Hi")).await.unwrap();
        // After execute: snapshot holds at least the initial user message
        let snap = op.context_snapshot();
        assert!(
            !snap.messages.is_empty(),
            "expected non-empty context after execute, got: {:?}",
            snap.messages
        );
    }

    #[test]
    fn context_snapshot_pinned_count() {
        let provider = MockProvider::new(vec![]);
        let op = make_op(provider);
        {
            let mut ctx = op.current_context.lock().unwrap();
            ctx.push(make_user_msg("normal"));
            ctx.push(make_pinned_msg("pinned1"));
            ctx.push(make_pinned_msg("pinned2"));
        }
        let snap = op.context_snapshot();
        assert_eq!(snap.messages.len(), 3);
        assert_eq!(snap.pinned_count, 2);
    }

    #[test]
    fn context_snapshot_last_compaction_removed_zero_initially() {
        let provider = MockProvider::new(vec![]);
        let op = make_op(provider);
        assert_eq!(op.context_snapshot().last_compaction_removed, 0);
    }

    #[test]
    fn context_snapshot_clone_and_debug() {
        let provider = MockProvider::new(vec![]);
        let op = make_op(provider);
        let snap = op.context_snapshot();
        let cloned = snap.clone();
        assert_eq!(cloned.messages.len(), snap.messages.len());
        // Debug must not panic
        let _ = format!("{:?}", snap);
    }

    #[test]
    fn context_snapshot_serde_round_trip() {
        let provider = MockProvider::new(vec![]);
        let op = make_op(provider);
        let snap = op.context_snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let back: ContextSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.messages.len(), snap.messages.len());
        assert_eq!(back.token_count, snap.token_count);
        assert_eq!(back.pinned_count, snap.pinned_count);
        assert_eq!(back.last_compaction_removed, snap.last_compaction_removed);
    }

    // ── Mock Orchestrator ─────────────────────────────────────────────────

    struct RecordingOrchestrator {
        dispatched: Mutex<Vec<String>>,
    }

    impl RecordingOrchestrator {
        fn new() -> Self {
            Self {
                dispatched: Mutex::new(vec![]),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.dispatched.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Orchestrator for RecordingOrchestrator {
        async fn dispatch(
            &self,
            agent: &AgentId,
            _input: OperatorInput,
        ) -> Result<OperatorOutput, OrchError> {
            self.dispatched.lock().unwrap().push(agent.to_string());
            Ok(OperatorOutput::new(
                Content::text(r#"{"result":"from_orch"}"#),
                ExitReason::Complete,
            ))
        }

        async fn dispatch_many(
            &self,
            tasks: Vec<(AgentId, OperatorInput)>,
        ) -> Vec<Result<OperatorOutput, OrchError>> {
            let mut out = Vec::new();
            for (agent, input) in tasks {
                out.push(self.dispatch(&agent, input).await);
            }
            out
        }

        async fn signal(
            &self,
            _target: &WorkflowId,
            _signal: SignalPayload,
        ) -> Result<(), OrchError> {
            Ok(())
        }

        async fn query(
            &self,
            _target: &WorkflowId,
            _query: layer0::orchestrator::QueryPayload,
        ) -> Result<serde_json::Value, OrchError> {
            Ok(serde_json::Value::Null)
        }
    }

    // ── Orchestrator integration tests ────────────────────────────────────

    #[tokio::test]
    async fn orchestrator_dispatch_routes_through_orch() {
        let orch = Arc::new(RecordingOrchestrator::new());
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({"msg": "hello"})),
            simple_text_response("Done."),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = make_op_with_tools(provider, tools)
            .with_orchestrator(Arc::clone(&orch) as Arc<dyn Orchestrator>);

        let output = op.execute(simple_input("Test")).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Done.");

        // Verify routing: the orchestrator recorded the dispatch, not ToolDyn::call()
        let calls = orch.calls();
        assert_eq!(calls.len(), 1, "expected exactly one orchestrator dispatch");
        assert_eq!(calls[0], "echo");
    }

    #[tokio::test]
    async fn default_orchestrator_dispatches_via_tool_registry() {
        // new() auto-creates a ToolRegistryOrchestrator from the tools arg.
        // Verify that tool dispatch works without explicit with_orchestrator().
        let provider = MockProvider::new(vec![
            tool_use_response("tu_1", "echo", json!({"msg": "hello"})),
            simple_text_response("Done via orch."),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        let op = make_op_with_tools(provider, tools);

        let output = op.execute(simple_input("Test")).await.unwrap();

        assert_eq!(output.exit_reason, ExitReason::Complete);
        assert_eq!(output.message.as_text().unwrap(), "Done via orch.");

        // One sub-dispatch recorded, marked successful
        assert_eq!(output.metadata.sub_dispatches.len(), 1);
        assert_eq!(output.metadata.sub_dispatches[0].name, "echo");
        assert!(output.metadata.sub_dispatches[0].success);
    }
}
