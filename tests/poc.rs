//! Proof of Concept: composability patterns without live API keys.
//!
//! Demonstrates the four core composability patterns that the neuron
//! architecture enables:
//!
//! 1. **Provider swap** — Same operator, different LLM backend
//! 2. **State swap** — Same workflow logic, different state backend
//! 3. **Operator swap** — Same input, different operator implementation
//! 4. **Multi-agent orchestration** — Orchestrator dispatches to multiple agents
//!
//! All tests run without API keys by using mock/test implementations.

use layer0::content::Content;
use layer0::effect::Scope;
use layer0::id::AgentId;
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, TriggerType};
use layer0::orchestrator::Orchestrator;
use layer0::state::StateStore;
use layer0::test_utils::EchoOperator;
use neuron_context_engine::{Context, ReactLoopConfig, react_loop};
use neuron_op_single_shot::{SingleShotConfig, SingleShotOperator};
use neuron_orch_local::LocalOrch;
use neuron_state_fs::FsStore;
use neuron_state_memory::MemoryStore;
use neuron_tool::ToolRegistry;
use neuron_turn::infer::InferResponse;
use neuron_turn::test_utils::{TestProvider, make_text_response};
use neuron_turn::types::*;
use rust_decimal::Decimal;
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers for building InferResponse with specific fields
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build an InferResponse with custom token counts, model, and cost.
fn make_response(
    text: &str,
    input_tokens: u64,
    output_tokens: u64,
    model: &str,
    cost: Decimal,
) -> InferResponse {
    InferResponse {
        content: Content::text(text),
        tool_calls: vec![],
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        },
        model: model.into(),
        cost: Some(cost),
        truncated: None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ContextEngineOperator: A wrapper for testing react_loop with Operator trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Wraps react_loop in an Operator implementation for test composability.
struct ContextEngineOperator<P: neuron_turn::provider::Provider> {
    provider: P,
    config: ReactLoopConfig,
    tools: ToolRegistry,
    tool_ctx: neuron_tool::ToolCallContext,
}

impl<P: neuron_turn::provider::Provider> ContextEngineOperator<P> {
    fn new(provider: P, tools: ToolRegistry, config: ReactLoopConfig) -> Self {
        Self {
            provider,
            config,
            tools,
            tool_ctx: neuron_tool::ToolCallContext::new(layer0::id::AgentId::from("test")),
        }
    }
}

#[async_trait::async_trait]
impl<P: neuron_turn::provider::Provider> Operator for ContextEngineOperator<P> {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, layer0::OperatorError> {
        let mut ctx = Context::new();
        // Inject the user input as the first message
        ctx.inject_message(layer0::context::Message::new(
            layer0::context::Role::User,
            input.message,
        ))
        .await
        .map_err(|e| layer0::OperatorError::NonRetryable(e.to_string()))?;

        react_loop(
            &mut ctx,
            &self.provider,
            &self.tools,
            &self.tool_ctx,
            &self.config,
        )
        .await
        .map_err(|e| layer0::OperatorError::NonRetryable(e.to_string()))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn simple_input(text: &str) -> OperatorInput {
    OperatorInput::new(Content::text(text), TriggerType::User)
}

fn react_config() -> ReactLoopConfig {
    ReactLoopConfig {
        system_prompt: "You are a helpful assistant.".into(),
        model: Some("mock-model".into()),
        max_tokens: Some(256),
        temperature: None,
    }
}

fn make_react_operator(provider: TestProvider) -> ContextEngineOperator<TestProvider> {
    ContextEngineOperator::new(provider, ToolRegistry::new(), react_config())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pattern 1: Provider Swap
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn provider_swap_same_config_different_backend() {
    // The SAME ReactLoopConfig, ToolRegistry, and context strategy.
    // Only the provider instance differs.
    let config = react_config();
    let tools = ToolRegistry::new();

    // Provider A: returns "Hello from A" with 25 input tokens
    let provider_a = TestProvider::with_responses(vec![make_response(
        "Hello from provider A",
        25,
        10,
        "mock-model",
        Decimal::new(1, 4),
    )]);
    let op_a: ContextEngineOperator<TestProvider> =
        ContextEngineOperator::new(provider_a, tools, config);

    let config_b = react_config();
    let tools_b = ToolRegistry::new();

    // Provider B: returns "Hello from B" with 30 input tokens
    let provider_b = TestProvider::with_responses(vec![make_response(
        "Hello from provider B",
        30,
        15,
        "mock-model-b",
        Decimal::new(2, 4),
    )]);
    let op_b: ContextEngineOperator<TestProvider> =
        ContextEngineOperator::new(provider_b, tools_b, config_b);

    // Execute the same input through both
    let input_a = simple_input("Greet me");
    let input_b = simple_input("Greet me");

    let output_a = op_a.execute(input_a).await.unwrap();
    let output_b = op_b.execute(input_b).await.unwrap();

    // Both produce OperatorOutput with the same structure
    assert_eq!(output_a.exit_reason, ExitReason::Complete);
    assert_eq!(output_b.exit_reason, ExitReason::Complete);

    // But different content from different providers
    assert_eq!(output_a.message.as_text().unwrap(), "Hello from provider A");
    assert_eq!(output_b.message.as_text().unwrap(), "Hello from provider B");

    // Different token counts from different backends
    assert_eq!(output_a.metadata.tokens_in, 25);
    assert_eq!(output_b.metadata.tokens_in, 30);

    // Both can be used as dyn Operator (object-safe)
    let dyn_a: Arc<dyn Operator> =
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("dyn A"),
        ])));
    let dyn_b: Arc<dyn Operator> = Arc::new(ContextEngineOperator::new(
        TestProvider::with_responses(vec![make_text_response("dyn B")]),
        ToolRegistry::new(),
        react_config(),
    ));

    let out_a = dyn_a.execute(simple_input("test")).await.unwrap();
    let out_b = dyn_b.execute(simple_input("test")).await.unwrap();
    assert_eq!(out_a.exit_reason, ExitReason::Complete);
    assert_eq!(out_b.exit_reason, ExitReason::Complete);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pattern 2: State Swap
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn state_swap_memory_vs_filesystem() {
    let scope = Scope::Global;
    let key = "agent:preferences";
    let value = serde_json::json!({
        "language": "en",
        "verbosity": "concise",
        "tools_enabled": true
    });

    // Backend A: In-memory store
    let memory_store = MemoryStore::new();

    // Backend B: Filesystem store (tempdir)
    let tmpdir = tempfile::tempdir().unwrap();
    let fs_store = FsStore::new(tmpdir.path());

    // The SAME workflow logic applied to both backends:
    // Write, read back, verify, list, delete, verify gone.

    async fn state_workflow(
        store: &dyn StateStore,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) {
        // Write
        store.write(scope, key, value.clone()).await.unwrap();

        // Read back
        let read_value = store.read(scope, key).await.unwrap();
        assert_eq!(
            read_value,
            Some(value.clone()),
            "read should return written value"
        );

        // List keys
        let keys = store.list(scope, "agent:").await.unwrap();
        assert!(
            keys.contains(&key.to_string()),
            "list should contain our key"
        );

        // Write another key
        store
            .write(scope, "agent:history", serde_json::json!(["event1"]))
            .await
            .unwrap();
        let keys = store.list(scope, "agent:").await.unwrap();
        assert_eq!(keys.len(), 2, "should have 2 keys with prefix agent:");

        // Delete
        store.delete(scope, key).await.unwrap();
        let deleted = store.read(scope, key).await.unwrap();
        assert_eq!(deleted, None, "deleted key should return None");

        // Search (both return empty for now)
        let results = store.search(scope, "preferences", 5).await.unwrap();
        let _ = results; // Both return empty vec, that's fine
    }

    // Run the same workflow through both backends
    state_workflow(&memory_store, &scope, key, value.clone()).await;
    state_workflow(&fs_store, &scope, key, value).await;
}

#[tokio::test]
async fn state_swap_scope_isolation() {
    // Verify that scope isolation works identically across backends
    let global = Scope::Global;
    let session = Scope::Session(layer0::SessionId::new("test-session"));

    let memory_store = MemoryStore::new();
    let tmpdir = tempfile::tempdir().unwrap();
    let fs_store = FsStore::new(tmpdir.path());

    async fn verify_isolation(store: &dyn StateStore) {
        let global = Scope::Global;
        let session = Scope::Session(layer0::SessionId::new("test-session"));

        store
            .write(&global, "key", serde_json::json!("global_value"))
            .await
            .unwrap();
        store
            .write(&session, "key", serde_json::json!("session_value"))
            .await
            .unwrap();

        let global_val = store.read(&global, "key").await.unwrap();
        let session_val = store.read(&session, "key").await.unwrap();

        assert_eq!(global_val, Some(serde_json::json!("global_value")));
        assert_eq!(session_val, Some(serde_json::json!("session_value")));
    }

    verify_isolation(&memory_store).await;
    verify_isolation(&fs_store).await;

    // Suppress unused warnings
    let _ = global;
    let _ = session;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pattern 3: Operator Swap
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn operator_swap_react_vs_single_shot() {
    let provider_response = make_response("Hello, world!", 20, 8, "mock-model", Decimal::new(5, 5));

    // Operator A: ContextEngineOperator (multi-turn with tools, hooks, state)
    let react_op = make_react_operator(TestProvider::with_responses(vec![
        provider_response.clone(),
    ]));

    // Operator B: SingleShotOperator (single call, no tools)
    let single_shot_op = SingleShotOperator::new(
        TestProvider::with_responses(vec![provider_response]),
        SingleShotConfig {
            system_prompt: "You are a helpful assistant.".into(),
            default_model: "mock-model".into(),
            default_max_tokens: 256,
        },
    );

    // Same input through both operators
    let input = simple_input("Say hello");

    let react_output = react_op.execute(input.clone()).await.unwrap();
    let ss_output = single_shot_op.execute(input).await.unwrap();

    // Both produce OperatorOutput with identical structure
    assert_eq!(react_output.exit_reason, ExitReason::Complete);
    assert_eq!(ss_output.exit_reason, ExitReason::Complete);

    // Same message content (same mock response)
    assert_eq!(react_output.message.as_text().unwrap(), "Hello, world!");
    assert_eq!(ss_output.message.as_text().unwrap(), "Hello, world!");

    // Both have valid metadata
    assert!(react_output.metadata.tokens_in > 0);
    assert!(ss_output.metadata.tokens_in > 0);

    // Single-shot always uses exactly 1 turn
    assert_eq!(ss_output.metadata.turns_used, 1);
    assert_eq!(react_output.metadata.turns_used, 1); // also 1 when no tools used

    // Both can be used as dyn Operator (object-safe)
    let operators: Vec<Arc<dyn Operator>> = vec![
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("from react"),
        ]))),
        Arc::new(SingleShotOperator::new(
            TestProvider::with_responses(vec![make_text_response("from single-shot")]),
            SingleShotConfig::default(),
        )),
        Arc::new(EchoOperator), // layer0 test-utils echo operator
    ];

    for (i, op) in operators.iter().enumerate() {
        let output = op.execute(simple_input("test")).await.unwrap();
        assert_eq!(
            output.exit_reason,
            ExitReason::Complete,
            "operator {i} should complete"
        );
        assert!(
            output.message.as_text().is_some(),
            "operator {i} should produce text"
        );
    }
}

#[tokio::test]
async fn operator_swap_echo_operator() {
    // EchoOperator from layer0::test_utils simply echoes back the input.
    // Proves that the Operator trait is simple enough for trivial impls.
    let echo: Arc<dyn Operator> = Arc::new(EchoOperator);

    let input = simple_input("This exact text should come back");
    let output = echo.execute(input).await.unwrap();

    assert_eq!(output.exit_reason, ExitReason::Complete);
    assert_eq!(
        output.message.as_text().unwrap(),
        "This exact text should come back"
    );
    assert_eq!(output.metadata.tokens_in, 0); // EchoOperator uses default metadata
    assert!(output.effects.is_empty());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pattern 4: Multi-Agent Orchestration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn multi_agent_dispatch_single() {
    let mut orch = LocalOrch::new();

    // Register agents with different capabilities
    let summarizer: Arc<dyn Operator> =
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("Summary: the user greeted us."),
        ])));
    let classifier: Arc<dyn Operator> = Arc::new(SingleShotOperator::new(
        TestProvider::with_responses(vec![make_text_response("category: greeting")]),
        SingleShotConfig::default(),
    ));
    let echo: Arc<dyn Operator> = Arc::new(EchoOperator);

    orch.register(AgentId::new("summarizer"), summarizer);
    orch.register(AgentId::new("classifier"), classifier);
    orch.register(AgentId::new("echo"), echo);

    // Dispatch to individual agents
    let summary = orch
        .dispatch(&AgentId::new("summarizer"), simple_input("Hello there!"))
        .await
        .unwrap();
    assert_eq!(summary.exit_reason, ExitReason::Complete);
    assert_eq!(
        summary.message.as_text().unwrap(),
        "Summary: the user greeted us."
    );

    let classification = orch
        .dispatch(&AgentId::new("classifier"), simple_input("Hello there!"))
        .await
        .unwrap();
    assert_eq!(classification.exit_reason, ExitReason::Complete);
    assert_eq!(
        classification.message.as_text().unwrap(),
        "category: greeting"
    );

    let echoed = orch
        .dispatch(&AgentId::new("echo"), simple_input("Hello there!"))
        .await
        .unwrap();
    assert_eq!(echoed.message.as_text().unwrap(), "Hello there!");
}

#[tokio::test]
async fn multi_agent_parallel_dispatch() {
    let mut orch = LocalOrch::new();

    // Register multiple agents
    orch.register(
        AgentId::new("agent_a"),
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("Result from A"),
        ]))),
    );
    orch.register(
        AgentId::new("agent_b"),
        Arc::new(SingleShotOperator::new(
            TestProvider::with_responses(vec![make_text_response("Result from B")]),
            SingleShotConfig::default(),
        )),
    );
    orch.register(AgentId::new("agent_c"), Arc::new(EchoOperator));

    // Parallel dispatch to all three
    let tasks = vec![
        (AgentId::new("agent_a"), simple_input("Task for A")),
        (AgentId::new("agent_b"), simple_input("Task for B")),
        (AgentId::new("agent_c"), simple_input("Task for C")),
    ];

    let results = orch.dispatch_many(tasks).await;

    // All three should succeed
    assert_eq!(results.len(), 3);
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "agent {i} should succeed");
    }

    let outputs: Vec<OperatorOutput> = results.into_iter().map(|r| r.unwrap()).collect();

    assert_eq!(outputs[0].message.as_text().unwrap(), "Result from A");
    assert_eq!(outputs[1].message.as_text().unwrap(), "Result from B");
    assert_eq!(outputs[2].message.as_text().unwrap(), "Task for C"); // echo
}

#[tokio::test]
async fn multi_agent_with_state_storage() {
    // Full workflow: orchestrate agents, collect results, store in state.
    let mut orch = LocalOrch::new();
    let state = MemoryStore::new();

    orch.register(
        AgentId::new("researcher"),
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("Research findings: Rust is fast and safe."),
        ]))),
    );
    orch.register(
        AgentId::new("writer"),
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("Draft: Rust combines speed with memory safety."),
        ]))),
    );

    // Step 1: Dispatch research task
    let research = orch
        .dispatch(
            &AgentId::new("researcher"),
            simple_input("Research Rust programming"),
        )
        .await
        .unwrap();

    // Step 2: Store research results
    let scope = Scope::Session(layer0::SessionId::new("workflow-1"));
    state
        .write(
            &scope,
            "research_result",
            serde_json::json!({
                "text": research.message.as_text().unwrap(),
                "tokens_used": research.metadata.tokens_in + research.metadata.tokens_out,
            }),
        )
        .await
        .unwrap();

    // Step 3: Dispatch writing task
    let draft = orch
        .dispatch(
            &AgentId::new("writer"),
            simple_input("Write about Rust based on research"),
        )
        .await
        .unwrap();

    // Step 4: Store draft
    state
        .write(
            &scope,
            "draft",
            serde_json::json!({
                "text": draft.message.as_text().unwrap(),
                "exit_reason": format!("{:?}", draft.exit_reason),
            }),
        )
        .await
        .unwrap();

    // Step 5: Verify state contains both results
    let stored_research = state
        .read(&scope, "research_result")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored_research["text"].as_str().unwrap(),
        "Research findings: Rust is fast and safe."
    );

    let stored_draft = state.read(&scope, "draft").await.unwrap().unwrap();
    assert_eq!(
        stored_draft["text"].as_str().unwrap(),
        "Draft: Rust combines speed with memory safety."
    );
    assert_eq!(stored_draft["exit_reason"].as_str().unwrap(), "Complete");

    // List all workflow artifacts
    let keys = state.list(&scope, "").await.unwrap();
    assert_eq!(keys.len(), 2);
}

#[tokio::test]
async fn multi_agent_missing_agent_handled_gracefully() {
    let mut orch = LocalOrch::new();
    orch.register(AgentId::new("echo"), Arc::new(EchoOperator));

    // Dispatch to a mix of existing and missing agents
    let tasks = vec![
        (AgentId::new("echo"), simple_input("exists")),
        (AgentId::new("nonexistent"), simple_input("missing")),
    ];

    let results = orch.dispatch_many(tasks).await;
    assert_eq!(results.len(), 2);
    assert!(results[0].is_ok());
    assert!(results[1].is_err());

    // The error is an OrchError::AgentNotFound
    match results[1].as_ref().unwrap_err() {
        layer0::OrchError::AgentNotFound(name) => {
            assert_eq!(name, "nonexistent");
        }
        other => panic!("expected AgentNotFound, got {:?}", other),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Composition: combining patterns
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn combined_all_patterns() {
    // This test combines all four patterns in a single workflow:
    // 1. Provider swap: two agents use different mock providers
    // 2. State swap: results stored in both memory and filesystem
    // 3. Operator swap: one agent uses ContextEngineOperator, another uses SingleShot
    // 4. Orchestration: LocalOrch dispatches to both agents

    let mut orch = LocalOrch::new();

    // Agent 1: ContextEngineOperator with TestProvider (provider A)
    let agent_react: Arc<dyn Operator> =
        Arc::new(make_react_operator(TestProvider::with_responses(vec![
            make_text_response("Analysis: topic is interesting."),
        ])));

    // Agent 2: SingleShotOperator with TestProvider (provider B)
    let agent_ss: Arc<dyn Operator> = Arc::new(SingleShotOperator::new(
        TestProvider::with_responses(vec![make_text_response("Rating: 8/10")]),
        SingleShotConfig {
            system_prompt: "Rate the topic.".into(),
            default_model: "mock-b".into(),
            default_max_tokens: 128,
        },
    ));

    orch.register(AgentId::new("analyst"), agent_react);
    orch.register(AgentId::new("rater"), agent_ss);

    // Parallel dispatch (orchestration pattern)
    let tasks = vec![
        (AgentId::new("analyst"), simple_input("Evaluate Rust")),
        (AgentId::new("rater"), simple_input("Evaluate Rust")),
    ];
    let results = orch.dispatch_many(tasks).await;

    let analysis = results[0].as_ref().unwrap();
    let rating = results[1].as_ref().unwrap();

    // State swap: store in both memory and filesystem
    let memory = MemoryStore::new();
    let tmpdir = tempfile::tempdir().unwrap();
    let filesystem = FsStore::new(tmpdir.path());

    let scope = Scope::Global;

    async fn store_results(
        store: &dyn StateStore,
        scope: &Scope,
        analysis: &OperatorOutput,
        rating: &OperatorOutput,
    ) {
        store
            .write(
                scope,
                "analysis",
                serde_json::json!(analysis.message.as_text().unwrap()),
            )
            .await
            .unwrap();
        store
            .write(
                scope,
                "rating",
                serde_json::json!(rating.message.as_text().unwrap()),
            )
            .await
            .unwrap();

        // Verify both stored correctly
        let a = store.read(scope, "analysis").await.unwrap().unwrap();
        let r = store.read(scope, "rating").await.unwrap().unwrap();
        assert_eq!(a.as_str().unwrap(), "Analysis: topic is interesting.");
        assert_eq!(r.as_str().unwrap(), "Rating: 8/10");
    }

    store_results(&memory, &scope, analysis, rating).await;
    store_results(&filesystem, &scope, analysis, rating).await;
}
