//! Phase 2 tests — in-memory implementations prove the traits work.
//! Run with: cargo test --features test-utils --test phase2

#![cfg(feature = "test-utils")]

use layer0::dispatch_context::DispatchContext;
use layer0::id::{DispatchId, OperatorId};
use layer0::test_utils::{EchoOperator, InMemoryStore, LocalEnvironment, LocalOrchestrator};
use layer0::*;
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;

fn simple_input(msg: &str) -> OperatorInput {
    OperatorInput::new(Content::text(msg), layer0::operator::TriggerType::User)
}

fn test_ctx(name: &str) -> DispatchContext {
    DispatchContext::new(DispatchId::new(name), OperatorId::new(name))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EchoOperator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn echo_operator_returns_input_as_output() {
    let turn = EchoOperator;
    let input = simple_input("hello echo");
    let output = turn
        .execute(
            input,
            &DispatchContext::new(DispatchId::new("t"), OperatorId::new("echo")),
        )
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("hello echo"));
    assert_eq!(output.exit_reason, ExitReason::Complete);
}

#[tokio::test]
async fn echo_operator_metadata_is_default() {
    let turn = EchoOperator;
    let input = simple_input("test");
    let output = turn
        .execute(
            input,
            &DispatchContext::new(DispatchId::new("t"), OperatorId::new("echo")),
        )
        .await
        .unwrap();
    assert_eq!(output.metadata.tokens_in, 0);
    assert_eq!(output.metadata.cost, Decimal::ZERO);
    assert!(output.effects.is_empty());
}

#[tokio::test]
async fn echo_operator_is_usable_as_dyn_operator() {
    let turn: Box<dyn Operator> = Box::new(EchoOperator);
    let input = simple_input("dynamic dispatch");
    let output = turn
        .execute(
            input,
            &DispatchContext::new(DispatchId::new("t"), OperatorId::new("echo")),
        )
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("dynamic dispatch"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// InMemoryStore
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Helper: cast to &dyn StateStore to avoid ambiguity with blanket StateReader impl
fn as_store(s: &InMemoryStore) -> &dyn StateStore {
    s
}

#[tokio::test]
async fn in_memory_store_write_then_read() {
    let store = InMemoryStore::new();
    let scope = Scope::Global;
    let s = as_store(&store);
    s.write(&scope, "key1", json!("value1")).await.unwrap();
    let val = s.read(&scope, "key1").await.unwrap();
    assert_eq!(val, Some(json!("value1")));
}

#[tokio::test]
async fn in_memory_store_read_missing_returns_none() {
    let store = InMemoryStore::new();
    let val = as_store(&store)
        .read(&Scope::Global, "nonexistent")
        .await
        .unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn in_memory_store_delete() {
    let store = InMemoryStore::new();
    let scope = Scope::Global;
    let s = as_store(&store);
    s.write(&scope, "key1", json!("value1")).await.unwrap();
    s.delete(&scope, "key1").await.unwrap();
    let val = s.read(&scope, "key1").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn in_memory_store_delete_missing_is_noop() {
    let store = InMemoryStore::new();
    // Should not error
    as_store(&store)
        .delete(&Scope::Global, "nonexistent")
        .await
        .unwrap();
}

#[tokio::test]
async fn in_memory_store_list_by_prefix() {
    let store = InMemoryStore::new();
    let scope = Scope::Global;
    let s = as_store(&store);
    s.write(&scope, "notes/a", json!("a")).await.unwrap();
    s.write(&scope, "notes/b", json!("b")).await.unwrap();
    s.write(&scope, "other/c", json!("c")).await.unwrap();

    let mut keys = s.list(&scope, "notes/").await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["notes/a", "notes/b"]);
}

#[tokio::test]
async fn in_memory_store_list_empty_prefix_returns_all() {
    let store = InMemoryStore::new();
    let scope = Scope::Global;
    let s = as_store(&store);
    s.write(&scope, "a", json!(1)).await.unwrap();
    s.write(&scope, "b", json!(2)).await.unwrap();

    let mut keys = s.list(&scope, "").await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["a", "b"]);
}

#[tokio::test]
async fn in_memory_store_scopes_are_isolated() {
    let store = InMemoryStore::new();
    let s1 = Scope::Session(SessionId::new("s1"));
    let s2 = Scope::Session(SessionId::new("s2"));
    let s = as_store(&store);

    s.write(&s1, "key", json!("from s1")).await.unwrap();
    s.write(&s2, "key", json!("from s2")).await.unwrap();

    assert_eq!(s.read(&s1, "key").await.unwrap(), Some(json!("from s1")));
    assert_eq!(s.read(&s2, "key").await.unwrap(), Some(json!("from s2")));
}

#[tokio::test]
async fn in_memory_store_overwrite() {
    let store = InMemoryStore::new();
    let scope = Scope::Global;
    let s = as_store(&store);
    s.write(&scope, "key", json!("v1")).await.unwrap();
    s.write(&scope, "key", json!("v2")).await.unwrap();
    assert_eq!(s.read(&scope, "key").await.unwrap(), Some(json!("v2")));
}

#[tokio::test]
async fn in_memory_store_search_returns_empty() {
    // InMemoryStore doesn't support semantic search — returns empty vec
    let store = InMemoryStore::new();
    let results = as_store(&store)
        .search(&Scope::Global, "anything", 10)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn in_memory_store_is_usable_as_dyn_state_store() {
    let store: Box<dyn StateStore> = Box::new(InMemoryStore::new());
    store.write(&Scope::Global, "k", json!("v")).await.unwrap();
    assert_eq!(
        store.read(&Scope::Global, "k").await.unwrap(),
        Some(json!("v"))
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LocalEnvironment
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn local_environment_passes_through_to_turn() {
    let env = LocalEnvironment::new(Arc::new(EchoOperator));
    let input = simple_input("through environment");
    let spec = EnvironmentSpec::default();
    let output = env.run(&test_ctx("local-env"), input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("through environment"));
    assert_eq!(output.exit_reason, ExitReason::Complete);
}

#[tokio::test]
async fn local_environment_is_usable_as_dyn_environment() {
    let env: Box<dyn Environment> = Box::new(LocalEnvironment::new(Arc::new(EchoOperator)));
    let input = simple_input("dynamic env");
    let spec = EnvironmentSpec::default();
    let output = env.run(&test_ctx("local-env"), input, &spec).await.unwrap();
    assert_eq!(output.message, Content::text("dynamic env"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LocalOrchestrator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[tokio::test]
async fn local_orchestrator_dispatch_to_echo() {
    let mut orch = LocalOrchestrator::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));
    let input = simple_input("dispatch test");
    let output = orch
        .dispatch(&test_ctx("echo"), input)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("dispatch test"));
}

#[tokio::test]
async fn local_orchestrator_dispatch_agent_not_found() {
    let orch = LocalOrchestrator::new();
    let input = simple_input("nobody home");
    let result = orch.dispatch(&test_ctx("missing"), input).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("operator not found")
    );
}

#[tokio::test]
async fn local_orchestrator_is_usable_as_dyn_dispatcher() {
    let mut orch = LocalOrchestrator::new();
    orch.register(OperatorId::new("echo"), Arc::new(EchoOperator));
    let orch: Box<dyn layer0::dispatch::Dispatcher> = Box::new(orch);
    let output = orch
        .dispatch(&test_ctx("echo"), simple_input("dyn"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(output.message, Content::text("dyn"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Integration: compose ALL implementations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Full integration: orchestrator dispatches to two echo agents,
/// results are written to in-memory state, environment wraps the execution.
/// All through trait interfaces.
#[tokio::test]
async fn integration_compose_all_implementations() {
    // 1. Set up orchestrator with two agents
    let mut orch = LocalOrchestrator::new();
    orch.register(OperatorId::new("agent-a"), Arc::new(EchoOperator));
    orch.register(OperatorId::new("agent-b"), Arc::new(EchoOperator));

    // 2. Set up state store
    let store = InMemoryStore::new();
    let s = as_store(&store);

    // 3. Set up environment
    let env = LocalEnvironment::new(Arc::new(EchoOperator));

    // 5. Dispatch two agents through the orchestrator
    let output_a = orch
        .dispatch(&test_ctx("agent-a"), simple_input("task for A"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let output_b = orch
        .dispatch(&test_ctx("agent-b"), simple_input("task for B"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    // Verify both succeeded
    assert_eq!(output_a.message, Content::text("task for A"));
    assert_eq!(output_b.message, Content::text("task for B"));

    // 6. Write results to state store
    let scope = Scope::Workflow(WorkflowId::new("wf-integration"));
    s.write(&scope, "result/agent-a", json!({"message": "task for A"}))
        .await
        .unwrap();
    s.write(&scope, "result/agent-b", json!({"message": "task for B"}))
        .await
        .unwrap();

    // Verify state was persisted
    let keys = {
        let mut k = s.list(&scope, "result/").await.unwrap();
        k.sort();
        k
    };
    assert_eq!(keys, vec!["result/agent-a", "result/agent-b"]);
    assert_eq!(
        s.read(&scope, "result/agent-a").await.unwrap(),
        Some(json!({"message": "task for A"}))
    );

    // 8. Run a turn through the environment (passthrough)
    let env_output = env
        .run(
            &test_ctx("env-wrapped"),
            simple_input("env-wrapped"),
            &EnvironmentSpec::default(),
        )
        .await
        .unwrap();
    assert_eq!(env_output.message, Content::text("env-wrapped"));
}
