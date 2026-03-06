#![deny(missing_docs)]
//! Anthropic API provider for neuron-turn.
//!
//! Implements the [`neuron_turn::Provider`] trait for Anthropic's Messages API.

mod types;

use neuron_auth::{AuthProvider, AuthRequest};
use neuron_turn::provider::{Provider, ProviderError};
use neuron_turn::types::*;
use rust_decimal::Decimal;
use std::sync::Arc;
use types::*;

/// Credential source — resolved per request.
#[derive(Clone)]
enum ApiKeySource {
    /// Static key provided at construction time.
    Static(String),
    /// Environment variable name resolved via [`std::env::var`] at each call.
    EnvVar(String),
    /// [`AuthProvider`] called at each request; supports token refresh.
    Auth {
        /// The provider to call.
        provider: Arc<dyn AuthProvider>,
        /// Audience string forwarded to [`AuthProvider::provide`].
        audience: String,
    },
}

/// Anthropic API provider.
pub struct AnthropicProvider {
    api_key_source: ApiKeySource,
    client: reqwest::Client,
    api_url: String,
    api_version: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key_source: ApiKeySource::Static(api_key.into()),
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    /// Create a provider that reads its API key from an environment variable at each request.
    ///
    /// The variable is resolved via `std::env::var` at every call to `complete()`.  
    /// Returns `ProviderError::AuthFailed` if the variable is unset or empty — the error
    /// message contains the variable *name* only, never its value.
    pub fn from_env_var(var_name: impl Into<String>) -> Self {
        Self {
            api_key_source: ApiKeySource::EnvVar(var_name.into()),
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    /// Create a provider that authenticates via a [`neuron_auth::AuthProvider`].
    ///
    /// The provider is called at **every** request, so token refresh is
    /// transparent. Use this with `PiAuthProvider` or `OmpAuthProvider`
    /// from `neuron-extras`.
    ///
    /// OAuth tokens (`sk-ant-oat*`) returned by the provider are sent as
    /// `Authorization: Bearer` with the required `anthropic-beta:
    /// oauth-2025-04-20` header. Regular API keys use `x-api-key`.
    pub fn with_auth(provider: Arc<dyn AuthProvider>) -> Self {
        Self {
            api_key_source: ApiKeySource::Auth {
                provider,
                audience: "anthropic".into(),
            },
            client: reqwest::Client::new(),
            api_url: "https://api.anthropic.com/v1/messages".into(),
            api_version: "2023-06-01".into(),
        }
    }

    #[cfg(test)]
    async fn resolve_api_key(&self) -> Result<String, ProviderError> {
        resolve_key(&self.api_key_source).await
    }

    /// Override the API URL (for testing or proxies).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    fn build_request(&self, request: &ProviderRequest) -> AnthropicRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "claude-haiku-4-5-20251001".into());
        let max_tokens = request.max_tokens.unwrap_or(4096);

        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user".into(),
                    Role::Assistant => "assistant".into(),
                    Role::System => "user".into(), // System messages go in the system field
                },
                content: parts_to_anthropic_content(&m.content),
            })
            .collect();

        let tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect();

        AnthropicRequest {
            model,
            max_tokens,
            messages,
            system: request.system.clone(),
            tools,
        }
    }
}

/// Parse a raw [`AnthropicResponse`] into a [`ProviderResponse`].
fn parse_anthropic_response(
    response: AnthropicResponse,
) -> Result<ProviderResponse, ProviderError> {
    let content: Vec<ContentPart> = response
        .content
        .iter()
        .map(anthropic_block_to_content_part)
        .collect();

    let stop_reason = match response.stop_reason.as_str() {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::ContentFilter,
        _ => StopReason::EndTurn,
    };

    let usage = TokenUsage {
        input_tokens: response.usage.input_tokens,
        output_tokens: response.usage.output_tokens,
        cache_read_tokens: response.usage.cache_read_input_tokens,
        cache_creation_tokens: response.usage.cache_creation_input_tokens,
    };

    // Cost calculation for Haiku: $0.25/MTok input, $1.25/MTok output
    let input_cost = Decimal::from(response.usage.input_tokens) * Decimal::new(25, 8);
    let output_cost = Decimal::from(response.usage.output_tokens) * Decimal::new(125, 8);
    let cost = input_cost + output_cost;

    Ok(ProviderResponse {
        content,
        stop_reason,
        usage,
        model: response.model,
        cost: Some(cost),
        truncated: None,
    })
}

/// Returns `true` if `key` is an Anthropic OAuth token.
///
/// OAuth tokens use prefix `sk-ant-oat` and require `Authorization: Bearer`
/// plus the `anthropic-beta: oauth-2025-04-20` header. Standard API keys use
/// the `x-api-key` header.
fn is_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}

/// Resolve the credential from any [`ApiKeySource`] variant.
async fn resolve_key(source: &ApiKeySource) -> Result<String, ProviderError> {
    match source {
        ApiKeySource::Static(key) => Ok(key.clone()),
        ApiKeySource::EnvVar(var_name) => {
            let key = std::env::var(var_name).map_err(|_| {
                ProviderError::AuthFailed(format!("env var '{}' not set or not unicode", var_name))
            })?;
            if key.is_empty() {
                return Err(ProviderError::AuthFailed(format!(
                    "env var '{}' is empty",
                    var_name
                )));
            }
            Ok(key)
        }
        ApiKeySource::Auth { provider, audience } => {
            let req = AuthRequest::new().with_audience(audience.as_str());
            let token = provider
                .provide(&req)
                .await
                .map_err(|e| ProviderError::AuthFailed(format!("auth provider: {e}")))?;
            Ok(token.with_bytes(|b| String::from_utf8_lossy(b).into_owned()))
        }
    }
}

impl Provider for AnthropicProvider {
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> impl std::future::Future<Output = Result<ProviderResponse, ProviderError>> + Send {
        // Clone the parts we need to move into the async block without
        // holding a reference to `self`.
        let source = self.api_key_source.clone();
        let api_request = self.build_request(&request);
        let client = self.client.clone();
        let api_url = self.api_url.clone();
        let api_version = self.api_version.clone();

        async move {
            let key = resolve_key(&source).await?;

            // OAuth tokens require Bearer auth + the oauth beta header.
            // Standard API keys use x-api-key.
            let mut builder = client.post(&api_url);
            if is_oauth_token(&key) {
                builder = builder
                    .header("Authorization", format!("Bearer {key}"))
                    .header("anthropic-beta", "oauth-2025-04-20");
            } else {
                builder = builder.header("x-api-key", key);
            }
            let http_request = builder
                .header("anthropic-version", &api_version)
                .header("content-type", "application/json")
                .json(&api_request);

            let http_response =
                http_request
                    .send()
                    .await
                    .map_err(|e| ProviderError::TransientError {
                        message: e.to_string(),
                        status: None,
                    })?;

            let status = http_response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited);
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = http_response.text().await.unwrap_or_default();
                return Err(ProviderError::AuthFailed(body));
            }
            if !status.is_success() {
                let body = http_response.text().await.unwrap_or_default();
                return Err(map_error_response(status, &body));
            }

            let api_response: AnthropicResponse = http_response
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;

            parse_anthropic_response(api_response)
        }
    }
}

/// Map a non-success HTTP response to an appropriate [`ProviderError`].
///
/// - 500, 502, 503 (server errors) → [`ProviderError::TransientError`]
/// - Body containing content-filter signals → [`ProviderError::ContentBlocked`]
/// - All other non-success responses → [`ProviderError::TransientError`]
fn map_error_response(status: reqwest::StatusCode, body: &str) -> ProviderError {
    let status_u16 = status.as_u16();
    // Check for Anthropic content-filter signals in the response body.
    if body.contains("content_filter") || body.contains("content policy") {
        return ProviderError::ContentBlocked {
            message: body.to_string(),
        };
    }
    ProviderError::TransientError {
        message: format!("HTTP {status}: {body}"),
        status: Some(status_u16),
    }
}

fn parts_to_anthropic_content(parts: &[ContentPart]) -> AnthropicContent {
    if parts.len() == 1
        && let ContentPart::Text { text } = &parts[0]
    {
        return AnthropicContent::Text(text.clone());
    }
    AnthropicContent::Blocks(parts.iter().map(content_part_to_anthropic_block).collect())
}

fn content_part_to_anthropic_block(part: &ContentPart) -> AnthropicContentBlock {
    match part {
        ContentPart::Text { text } => AnthropicContentBlock::Text { text: text.clone() },
        ContentPart::ToolUse { id, name, input } => AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ContentPart::Image { source, media_type } => AnthropicContentBlock::Image {
            source: match source {
                ImageSource::Base64 { data } => AnthropicImageSource::Base64 { data: data.clone() },
                ImageSource::Url { url } => AnthropicImageSource::Url { url: url.clone() },
            },
            media_type: media_type.clone(),
        },
    }
}

fn anthropic_block_to_content_part(block: &AnthropicContentBlock) -> ContentPart {
    match block {
        AnthropicContentBlock::Text { text } => ContentPart::Text { text: text.clone() },
        AnthropicContentBlock::ToolUse { id, name, input } => ContentPart::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        AnthropicContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentPart::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        AnthropicContentBlock::Image { source, media_type } => ContentPart::Image {
            source: match source {
                AnthropicImageSource::Base64 { data } => ImageSource::Base64 { data: data.clone() },
                AnthropicImageSource::Url { url } => ImageSource::Url { url: url.clone() },
            },
            media_type: media_type.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_simple_request() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: Some("claude-haiku-4-5-20251001".into()),
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "Hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: Some(256),
            temperature: None,
            system: Some("Be helpful.".into()),
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "claude-haiku-4-5-20251001");
        assert_eq!(api_request.max_tokens, 256);
        assert_eq!(api_request.messages.len(), 1);
        assert_eq!(api_request.messages[0].role, "user");
        assert_eq!(api_request.system, Some("Be helpful.".into()));
    }

    #[test]
    fn parse_simple_response() {
        let _provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "Hello!".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "end_turn".into(),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = parse_anthropic_response(api_response).unwrap();
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 5);
        assert!(response.cost.is_some());
        assert_eq!(response.content.len(), 1);
    }

    #[test]
    fn parse_tool_use_response() {
        let _provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "bash".into(),
                input: json!({"command": "ls"}),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "tool_use".into(),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 30,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = parse_anthropic_response(api_response).unwrap();
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentPart::ToolUse { name, .. } => assert_eq!(name, "bash"),
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn tool_schema_serializes() {
        let tool = AnthropicTool {
            name: "get_weather".into(),
            description: "Get current weather".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "get_weather");
    }

    #[test]
    fn parse_cache_tokens() {
        let _provider = AnthropicProvider::new("test-key");
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "Cached.".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "end_turn".into(),
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 10,
                cache_read_input_tokens: Some(50),
                cache_creation_input_tokens: Some(25),
            },
        };

        let response = parse_anthropic_response(api_response).unwrap();
        assert_eq!(response.usage.cache_read_tokens, Some(50));
        assert_eq!(response.usage.cache_creation_tokens, Some(25));
    }

    #[test]
    fn default_model_is_haiku() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![ProviderMessage {
                role: Role::User,
                content: vec![ContentPart::Text { text: "Hi".into() }],
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn default_max_tokens_is_4096() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.max_tokens, 4096);
    }

    #[test]
    fn tool_result_in_request() {
        let provider = AnthropicProvider::new("test-key");
        let request = ProviderRequest {
            model: None,
            messages: vec![
                ProviderMessage {
                    role: Role::Assistant,
                    content: vec![ContentPart::ToolUse {
                        id: "tu_1".into(),
                        name: "bash".into(),
                        input: json!({"cmd": "ls"}),
                    }],
                },
                ProviderMessage {
                    role: Role::User,
                    content: vec![ContentPart::ToolResult {
                        tool_use_id: "tu_1".into(),
                        content: "file.txt".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: json!(null),
        };

        let api_request = provider.build_request(&request);
        assert_eq!(api_request.messages.len(), 2);
        assert_eq!(api_request.messages[0].role, "assistant");
        assert_eq!(api_request.messages[1].role, "user");
    }

    #[test]
    fn parse_response_refusal_maps_to_content_filter() {
        let api_response = AnthropicResponse {
            content: vec![AnthropicContentBlock::Text {
                text: "I cannot help with that.".into(),
            }],
            model: "claude-haiku-4-5-20251001".into(),
            stop_reason: "refusal".into(),
            usage: AnthropicUsage {
                input_tokens: 5,
                output_tokens: 8,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };
        let resp = parse_anthropic_response(api_response).expect("refusal should be Ok");
        assert_eq!(resp.stop_reason, StopReason::ContentFilter);
        assert_eq!(resp.usage.input_tokens, 5);
        assert_eq!(resp.usage.output_tokens, 8);
        assert_eq!(resp.content.len(), 1);
        assert!(resp.cost.is_some());
    }

    #[test]
    fn map_error_500_returns_transient() {
        let status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
        let err = map_error_response(status, "internal server error");
        assert!(matches!(
            err,
            ProviderError::TransientError {
                status: Some(500),
                ..
            }
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_error_503_returns_transient() {
        let status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
        let err = map_error_response(status, "service unavailable");
        assert!(matches!(
            err,
            ProviderError::TransientError {
                status: Some(503),
                ..
            }
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_error_content_filter_body_returns_blocked() {
        let status = reqwest::StatusCode::BAD_REQUEST;
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"content_filter triggered"}}"#;
        let err = map_error_response(status, body);
        assert!(matches!(err, ProviderError::ContentBlocked { .. }));
        assert!(!err.is_retryable());
    }
}

#[cfg(test)]
mod tests_credential {
    use super::*;
    use async_trait::async_trait;
    use neuron_auth::{AuthError, AuthToken};

    // ── Static / env var ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn new_uses_static_key() {
        let p = AnthropicProvider::new("sk-static");
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-static");
    }

    #[tokio::test]
    async fn from_env_var_resolves_when_set() {
        let var = "NEURON_ANTHROPIC_TEST_CRED_A";
        unsafe {
            std::env::set_var(var, "sk-from-env");
        }
        let p = AnthropicProvider::from_env_var(var);
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-from-env");
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn from_env_var_missing_returns_auth_failed() {
        let var = "NEURON_ANTHROPIC_TEST_CRED_MISSING_ZZZ";
        unsafe {
            std::env::remove_var(var);
        }
        let p = AnthropicProvider::from_env_var(var);
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        assert!(
            err.to_string().contains(var),
            "error should name the variable"
        );
    }

    #[tokio::test]
    async fn from_env_var_empty_returns_auth_failed() {
        let var = "NEURON_ANTHROPIC_TEST_CRED_EMPTY_ZZZ";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = AnthropicProvider::from_env_var(var);
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
        assert!(
            err.to_string().contains(var),
            "error should name the variable"
        );
        unsafe {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn error_message_does_not_contain_secret_value() {
        let var = "NEURON_ANTHROPIC_TEST_CRED_REDACT_ZZZ";
        let secret = "sk-must-not-appear-in-any-error-message";
        unsafe {
            std::env::set_var(var, "");
        }
        let p = AnthropicProvider::from_env_var(var);
        let msg = p.resolve_api_key().await.unwrap_err().to_string();
        assert!(msg.contains(var));
        assert!(!msg.contains(secret));
        unsafe {
            std::env::set_var(var, secret);
        }
        assert_eq!(p.resolve_api_key().await.unwrap(), secret);
        unsafe {
            std::env::remove_var(var);
        }
    }

    // ── OAuth token detection ────────────────────────────────────────────────

    #[test]
    fn oauth_token_prefix_detected() {
        assert!(is_oauth_token("sk-ant-oat01-abc"));
        assert!(is_oauth_token("sk-ant-oat-xyz"));
        assert!(!is_oauth_token("sk-ant-api123"));
        assert!(!is_oauth_token("sk-ant-api03-abc"));
        assert!(!is_oauth_token(""));
    }

    // ── Auth provider variant ────────────────────────────────────────────────

    /// Minimal stub that returns a pre-set token.
    struct StubAuth {
        token: String,
        expected_audience: String,
    }

    #[async_trait]
    impl neuron_auth::AuthProvider for StubAuth {
        async fn provide(
            &self,
            request: &neuron_auth::AuthRequest,
        ) -> Result<AuthToken, AuthError> {
            assert_eq!(
                request.audience.as_deref().unwrap_or(""),
                self.expected_audience,
            );
            Ok(AuthToken::permanent(self.token.as_bytes().to_vec()))
        }
    }

    #[tokio::test]
    async fn with_auth_resolves_via_provider() {
        let stub = Arc::new(StubAuth {
            token: "sk-ant-test".into(),
            expected_audience: "anthropic".into(),
        });
        let p = AnthropicProvider::with_auth(stub);
        assert_eq!(p.resolve_api_key().await.unwrap(), "sk-ant-test");
    }

    #[tokio::test]
    async fn with_auth_oauth_token_detected() {
        let stub = Arc::new(StubAuth {
            token: "sk-ant-oat01-mytoken".into(),
            expected_audience: "anthropic".into(),
        });
        let p = AnthropicProvider::with_auth(stub);
        let key = p.resolve_api_key().await.unwrap();
        assert!(is_oauth_token(&key), "token should be detected as OAuth");
    }

    #[tokio::test]
    async fn with_auth_provider_error_maps_to_auth_failed() {
        struct FailAuth;
        #[async_trait]
        impl neuron_auth::AuthProvider for FailAuth {
            async fn provide(&self, _: &neuron_auth::AuthRequest) -> Result<AuthToken, AuthError> {
                Err(AuthError::ScopeUnavailable("no anthropic key".into()))
            }
        }
        let p = AnthropicProvider::with_auth(Arc::new(FailAuth));
        let err = p.resolve_api_key().await.unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed(_)));
    }
}
