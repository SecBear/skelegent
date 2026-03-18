//! Integration tests: real Anthropic API calls through `skg-turn::Provider`.
//!
//! All tests are `#[ignore]` — they require `ANTHROPIC_API_KEY` in the environment
//! and make billable API calls. Run explicitly with:
//!
//! ```sh
//! cargo test -p skg-provider-anthropic --test integration -- --ignored
//! ```

use layer0::content::Content;
use layer0::context::{Message, Role};
use skg_provider_anthropic::AnthropicProvider;
use skg_turn::infer::InferRequest;
use skg_turn::provider::Provider;

/// Smoke test: send a single user message to Claude Haiku and verify we get text back.
#[tokio::test]
#[ignore] // Requires ANTHROPIC_API_KEY environment variable
async fn real_haiku_simple_completion() {
    let provider = AnthropicProvider::from_env_var("ANTHROPIC_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("Reply with exactly: hello world"),
    )])
    .with_model("claude-haiku-4-5-20251001")
    .with_max_tokens(64);

    let response = provider
        .infer(request)
        .await
        .expect("Anthropic API call failed");

    let text = response.text().expect("response should contain text");
    assert!(!text.is_empty(), "response text must not be empty");
    assert!(
        response.usage.output_tokens > 0,
        "should report token usage"
    );
}

/// Verify that `AnthropicProvider` can be used behind `Arc<dyn Provider>`.
/// This is a compile-time check — the Provider trait uses RPITIT and is NOT
/// object-safe, so this test verifies the concrete type works through the trait.
#[tokio::test]
#[ignore] // Requires ANTHROPIC_API_KEY environment variable
async fn anthropic_provider_infer_via_trait() {
    let provider = AnthropicProvider::from_env_var("ANTHROPIC_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("What is 2 + 2? Answer with just the number."),
    )])
    .with_model("claude-haiku-4-5-20251001")
    .with_max_tokens(16);

    // Call through the trait method explicitly.
    let response = Provider::infer(&provider, request)
        .await
        .expect("infer via trait should succeed");

    assert!(
        response.text().is_some(),
        "response should have text content"
    );
}

/// System prompt is respected.
#[tokio::test]
#[ignore] // Requires ANTHROPIC_API_KEY environment variable
async fn anthropic_system_prompt() {
    let provider = AnthropicProvider::from_env_var("ANTHROPIC_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("What is your name?"),
    )])
    .with_system("You are TestBot. Always introduce yourself as TestBot.")
    .with_model("claude-haiku-4-5-20251001")
    .with_max_tokens(64);

    let response = provider
        .infer(request)
        .await
        .expect("Anthropic API call failed");

    let text = response.text().expect("response should contain text");
    assert!(
        text.to_lowercase().contains("testbot"),
        "system prompt should be respected; got: {text}"
    );
}
