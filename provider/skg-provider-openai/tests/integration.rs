//! Integration tests: real OpenAI API calls through `skg-turn::Provider`.
//!
//! All tests are `#[ignore]` — they require `OPENAI_API_KEY` in the environment
//! and make billable API calls. Run explicitly with:
//!
//! ```sh
//! cargo test -p skg-provider-openai --test integration -- --ignored
//! ```

use layer0::content::Content;
use layer0::context::{Message, Role};
use skg_provider_openai::OpenAIProvider;
use skg_turn::infer::InferRequest;
use skg_turn::provider::Provider;

/// Smoke test: send a single user message to GPT-4o-mini and verify we get text back.
#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn real_gpt4o_mini_simple_completion() {
    let provider = OpenAIProvider::from_env_var("OPENAI_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("Reply with exactly: hello world"),
    )])
    .with_model("gpt-4o-mini")
    .with_max_tokens(64);

    let response = provider
        .infer(request)
        .await
        .expect("OpenAI API call failed");

    let text = response.text().expect("response should contain text");
    assert!(!text.is_empty(), "response text must not be empty");
    assert!(
        response.usage.output_tokens > 0,
        "should report token usage"
    );
}

/// Verify that `OpenAIProvider` can be used through the `Provider` trait.
/// Provider uses RPITIT (not object-safe), so this calls the trait method on
/// the concrete type.
#[tokio::test]
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn openai_provider_infer_via_trait() {
    let provider = OpenAIProvider::from_env_var("OPENAI_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("What is 2 + 2? Answer with just the number."),
    )])
    .with_model("gpt-4o-mini")
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
#[ignore] // Requires OPENAI_API_KEY environment variable
async fn openai_system_prompt() {
    let provider = OpenAIProvider::from_env_var("OPENAI_API_KEY");

    let request = InferRequest::new(vec![Message::new(
        Role::User,
        Content::text("What is your name?"),
    )])
    .with_system("You are TestBot. Always introduce yourself as TestBot.")
    .with_model("gpt-4o-mini")
    .with_max_tokens(64);

    let response = provider
        .infer(request)
        .await
        .expect("OpenAI API call failed");

    let text = response.text().expect("response should contain text");
    assert!(
        text.to_lowercase().contains("testbot"),
        "system prompt should be respected; got: {text}"
    );
}
