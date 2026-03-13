# Testing

skelegent is designed for testability. Every protocol trait is object-safe, so you can create mock implementations for any component. Layer 0 provides test utilities, and the workspace includes patterns for unit, integration, and object-safety testing.

## test-utils feature in layer0

Layer 0 provides test utilities behind the `test-utils` feature flag:

```toml
[dev-dependencies]
layer0 = { version = "0.4", features = ["test-utils"] }
```

This module includes mock implementations of the protocol traits that are useful for testing code that depends on `dyn Operator`, `dyn StateStore`, etc.

## Object-safety tests

A critical property of Layer 0 traits is object safety. Every trait must work behind `Box<dyn Trait>` and be `Send + Sync`. The workspace enforces this with compile-time tests:

```rust
fn _assert_send_sync<T: Send + Sync>() {}

#[test]
fn operator_is_object_safe_and_send_sync() {
    _assert_send_sync::<Box<dyn layer0::Operator>>();
}

#[test]
fn state_store_is_object_safe_and_send_sync() {
    _assert_send_sync::<Box<dyn layer0::StateStore>>();
}

#[test]
fn dispatcher_is_object_safe_and_send_sync() {

    _assert_send_sync::<Box<dyn layer0::Dispatcher>>();

}



#[test]

fn signalable_is_object_safe_and_send_sync() {

    _assert_send_sync::<Box<dyn layer0::Signalable>>();

}



#[test]

fn queryable_is_object_safe_and_send_sync() {

    _assert_send_sync::<Box<dyn layer0::Queryable>>();

}

#[test]
fn environment_is_object_safe_and_send_sync() {
    _assert_send_sync::<Box<dyn layer0::Environment>>();
}

#[test]
fn dispatch_middleware_is_object_safe_and_send_sync() {
    _assert_send_sync::<Box<dyn layer0::DispatchMiddleware>>();
}
```

These tests cost nothing at runtime -- they are purely compile-time assertions. If someone accidentally makes a trait non-object-safe, the test fails to compile.

The same pattern is used for non-Layer-0 traits:

```rust
#[test]
fn tool_dyn_is_object_safe() {
    _assert_send_sync::<std::sync::Arc<dyn skg_tool::ToolDyn>>();
}
```

## Serde roundtrip tests

All Layer 0 message types must serialize and deserialize correctly. The workspace tests this with roundtrip assertions:

```rust
use layer0::operator::{OperatorInput, TriggerType};
use layer0::content::Content;

#[test]
fn operator_input_roundtrips() {
    let input = OperatorInput::new(Content::text("hello"), TriggerType::User);
    let json = serde_json::to_string(&input).unwrap();
    let roundtripped: OperatorInput = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtripped.message, input.message);
}
```

## Mock providers for operator testing

To test operators without making real API calls, create a mock `Provider`:

```rust
use skg_turn::provider::{Provider, ProviderError};
use skg_turn::infer::{InferRequest, InferResponse};
use std::future::Future;

struct MockProvider {
    responses: Vec<InferResponse>,
}

impl Provider for MockProvider {
    fn infer(
        &self,
        _request: InferRequest,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send {
        let response = self.responses[0].clone(); // simplified
        async move { Ok(response) }
    }
}
```

Then construct a `Context` and call `react_loop` with the mock provider:

```rust,no_run
use skg_context_engine::{Context, react_loop, ReactLoopConfig};
use skg_tool::{ToolRegistry, ToolCallContext};
use layer0::context::{Message, Role};

let mut ctx = Context::new("You are a helpful assistant.");
ctx.inject_message(Message::new(Role::User, "Hello"));

let tools = ToolRegistry::new();
let tool_ctx = ToolCallContext::empty();
let config = ReactLoopConfig::default();

// Now test without network calls
react_loop(&mut ctx, &mock_provider, &tools, &tool_ctx, &config).await.unwrap();
```

## Mock tools

Create test tools by implementing `ToolDyn`:

```rust
use skg_tool::{ToolDyn, ToolError};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;

struct AlwaysSucceedTool;

impl ToolDyn for AlwaysSucceedTool {
    fn name(&self) -> &str { "test_tool" }
    fn description(&self) -> &str { "Always succeeds" }
    fn input_schema(&self) -> Value { json!({"type": "object"}) }
    fn call(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        Box::pin(async move { Ok(json!({"result": "ok"})) })
    }
}
```

## Testing state stores

Both `MemoryStore` and `FsStore` implement `StateStore`, so you can write generic tests:

```rust,no_run
use layer0::state::StateStore;
use layer0::effect::Scope;
use serde_json::json;

async fn test_crud(store: &dyn StateStore) {
    let scope = Scope::Global;

    // Write and read back
    store.write(&scope, "key", json!("value")).await.unwrap();
    let val = store.read(&scope, "key").await.unwrap();
    assert_eq!(val, Some(json!("value")));

    // Delete
    store.delete(&scope, "key").await.unwrap();
    let val = store.read(&scope, "key").await.unwrap();
    assert_eq!(val, None);
}
```

Use `MemoryStore` for fast unit tests. Use `FsStore` with `tempfile::TempDir` for integration tests that exercise filesystem behavior.

## Running the test suite

```bash
# Run all tests
cargo test

# Run tests with test-utils
cargo test --features test-utils -p layer0

# Run tests for a specific crate
cargo test -p skg-context-engine

# Verify no clippy warnings
cargo clippy -- -D warnings
```
