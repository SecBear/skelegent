//! Agent Card and related discovery types from the A2A protocol spec.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// AgentCard
// ---------------------------------------------------------------------------

/// Describes an agent's identity, capabilities, and connectivity.
///
/// This is the primary discovery document exchanged between A2A participants.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Human-readable name of the agent.
    pub name: String,
    /// Human-readable description of the agent's purpose.
    pub description: String,
    /// Network interfaces at which the agent can be reached.
    pub supported_interfaces: Vec<AgentInterface>,
    /// Optional provider metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    /// Semantic version of the agent.
    pub version: String,
    /// URL to the agent's documentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    /// Declared capabilities of the agent.
    pub capabilities: AgentCapabilities,
    /// Named security schemes the agent supports.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub security_schemes: HashMap<String, SecurityScheme>,
    /// Security requirements — each entry maps a scheme name to required scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_requirements: Vec<SecurityRequirement>,
    /// Default content types the agent accepts as input (e.g. `"text"`, `"image"`).
    pub default_input_modes: Vec<String>,
    /// Default content types the agent can produce (e.g. `"text"`, `"image"`).
    pub default_output_modes: Vec<String>,
    /// Skills the agent exposes.
    pub skills: Vec<AgentSkill>,
    /// Optional icon URL for UI presentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

impl AgentCard {
    /// Start building an [`AgentCard`] with the required name and description.
    pub fn builder(name: impl Into<String>, description: impl Into<String>) -> AgentCardBuilder {
        AgentCardBuilder {
            name: name.into(),
            description: description.into(),
            supported_interfaces: Vec::new(),
            provider: None,
            version: None,
            documentation_url: None,
            capabilities: AgentCapabilities::default(),
            security_schemes: HashMap::new(),
            security_requirements: Vec::new(),
            default_input_modes: Vec::new(),
            default_output_modes: Vec::new(),
            skills: Vec::new(),
            icon_url: None,
        }
    }
}

// ---------------------------------------------------------------------------
// AgentInterface
// ---------------------------------------------------------------------------

/// A network endpoint where the agent is reachable.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterface {
    /// URL of the endpoint.
    pub url: String,
    /// Protocol binding identifier (e.g. `"jsonrpc/http"`).
    pub protocol_binding: String,
    /// Optional tenant identifier for multi-tenant deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Version of the A2A protocol supported at this interface.
    pub protocol_version: String,
}

// ---------------------------------------------------------------------------
// AgentProvider
// ---------------------------------------------------------------------------

/// Metadata about the organization that provides the agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    /// URL of the provider's website.
    pub url: String,
    /// Human-readable organization name.
    pub organization: String,
}

// ---------------------------------------------------------------------------
// AgentCapabilities
// ---------------------------------------------------------------------------

/// Declared capabilities of an agent.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    /// Whether the agent supports push notifications for task updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,
    /// Protocol extensions the agent supports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<AgentExtension>,
    /// Whether the agent publishes an extended agent card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_agent_card: Option<bool>,
}

// ---------------------------------------------------------------------------
// AgentExtension
// ---------------------------------------------------------------------------

/// An A2A protocol extension supported by the agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtension {
    /// URI identifying the extension specification.
    pub uri: String,
    /// Human-readable description of the extension.
    pub description: String,
    /// Whether callers must understand this extension.
    pub required: bool,
    /// Optional extension-specific parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

// ---------------------------------------------------------------------------
// AgentSkill
// ---------------------------------------------------------------------------

/// A discrete skill that an agent can perform.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Unique identifier for the skill.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Tags for categorization and discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Example prompts or inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// Content types this skill accepts as input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,
    /// Content types this skill can produce.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}

impl AgentSkill {
    /// Create a new skill with the required fields.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            tags: Vec::new(),
            examples: Vec::new(),
            input_modes: Vec::new(),
            output_modes: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Security types
// ---------------------------------------------------------------------------

/// An authentication/authorization scheme supported by the agent.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityScheme {
    /// API key–based authentication.
    #[serde(rename = "api_key")]
    ApiKey {
        /// Human-readable description of the scheme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// Where the key is transmitted (`"header"`, `"query"`, `"cookie"`).
        location: String,
        /// Name of the header, query parameter, or cookie.
        name: String,
    },
    /// HTTP authentication (e.g. Bearer, Basic).
    #[serde(rename = "http_auth")]
    HttpAuth {
        /// Human-readable description of the scheme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// HTTP auth scheme name (e.g. `"bearer"`).
        scheme: String,
        /// Format hint for bearer tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_format: Option<String>,
    },
    /// OAuth 2.0 authentication.
    #[serde(rename = "oauth2")]
    Oauth2 {
        /// Human-readable description of the scheme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// OAuth 2.0 flow configuration.
        flows: OAuthFlows,
    },
    /// OpenID Connect discovery-based authentication.
    #[serde(rename = "open_id_connect")]
    OpenIdConnect {
        /// Human-readable description of the scheme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// OpenID Connect discovery URL.
        open_id_connect_url: String,
    },
    /// Mutual TLS authentication.
    #[serde(rename = "mtls")]
    Mtls {
        /// Human-readable description of the scheme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// OAuth 2.0 flow configurations.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OAuthFlows {
    /// Authorization code grant flow.
    #[serde(rename = "authorization_code")]
    AuthorizationCode {
        /// Authorization endpoint URL.
        authorization_url: String,
        /// Token endpoint URL.
        token_url: String,
        /// Optional refresh URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_url: Option<String>,
        /// Available scopes mapped to their descriptions.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        scopes: HashMap<String, String>,
    },
    /// Client credentials grant flow.
    #[serde(rename = "client_credentials")]
    ClientCredentials {
        /// Token endpoint URL.
        token_url: String,
        /// Optional refresh URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_url: Option<String>,
        /// Available scopes mapped to their descriptions.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        scopes: HashMap<String, String>,
    },
    /// Device authorization grant flow.
    #[serde(rename = "device_code")]
    DeviceCode {
        /// Device authorization endpoint URL.
        device_authorization_url: String,
        /// Token endpoint URL.
        token_url: String,
        /// Available scopes mapped to their descriptions.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        scopes: HashMap<String, String>,
    },
}

/// A security requirement referencing named [`SecurityScheme`]s and their scopes.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    /// Map of scheme name to required scopes for that scheme.
    pub schemes: HashMap<String, Vec<String>>,
}

// ---------------------------------------------------------------------------
// AgentCardBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing [`AgentCard`] instances.
///
/// # Panics
///
/// [`build`](AgentCardBuilder::build) panics if:
/// - `version` has not been set
/// - `supported_interfaces` is empty
#[derive(Debug)]
pub struct AgentCardBuilder {
    name: String,
    description: String,
    supported_interfaces: Vec<AgentInterface>,
    provider: Option<AgentProvider>,
    version: Option<String>,
    documentation_url: Option<String>,
    capabilities: AgentCapabilities,
    security_schemes: HashMap<String, SecurityScheme>,
    security_requirements: Vec<SecurityRequirement>,
    default_input_modes: Vec<String>,
    default_output_modes: Vec<String>,
    skills: Vec<AgentSkill>,
    icon_url: Option<String>,
}

impl AgentCardBuilder {
    /// Set the semantic version of the agent.
    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = Some(v.into());
        self
    }

    /// Add a supported interface endpoint.
    pub fn interface(
        mut self,
        url: impl Into<String>,
        protocol: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.supported_interfaces.push(AgentInterface {
            url: url.into(),
            protocol_binding: protocol.into(),
            tenant: None,
            protocol_version: version.into(),
        });
        self
    }

    /// Set the agent provider metadata.
    pub fn provider(mut self, org: impl Into<String>, url: impl Into<String>) -> Self {
        self.provider = Some(AgentProvider {
            organization: org.into(),
            url: url.into(),
        });
        self
    }

    /// Add a skill to the agent card.
    pub fn skill(mut self, skill: AgentSkill) -> Self {
        self.skills.push(skill);
        self
    }

    /// Declare whether the agent supports streaming.
    pub fn streaming(mut self, enabled: bool) -> Self {
        self.capabilities.streaming = Some(enabled);
        self
    }

    /// Declare whether the agent supports push notifications.
    pub fn push_notifications(mut self, enabled: bool) -> Self {
        self.capabilities.push_notifications = Some(enabled);
        self
    }

    /// Add a default input content mode.
    pub fn input_mode(mut self, mode: impl Into<String>) -> Self {
        self.default_input_modes.push(mode.into());
        self
    }

    /// Add a default output content mode.
    pub fn output_mode(mut self, mode: impl Into<String>) -> Self {
        self.default_output_modes.push(mode.into());
        self
    }

    /// Set the documentation URL.
    pub fn documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Set the icon URL.
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon_url = Some(url.into());
        self
    }

    /// Consume the builder and produce an [`AgentCard`].
    ///
    /// # Panics
    ///
    /// Panics if `version` is unset or `supported_interfaces` is empty.
    pub fn build(self) -> AgentCard {
        let version = self
            .version
            .expect("AgentCardBuilder: `version` is required");
        assert!(
            !self.supported_interfaces.is_empty(),
            "AgentCardBuilder: at least one interface is required"
        );

        AgentCard {
            name: self.name,
            description: self.description,
            supported_interfaces: self.supported_interfaces,
            provider: self.provider,
            version,
            documentation_url: self.documentation_url,
            capabilities: self.capabilities,
            security_schemes: self.security_schemes,
            security_requirements: self.security_requirements,
            default_input_modes: self.default_input_modes,
            default_output_modes: self.default_output_modes,
            skills: self.skills,
            icon_url: self.icon_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_produces_valid_card() {
        let card = AgentCard::builder("test-agent", "A test agent")
            .version("1.0.0")
            .interface("https://example.com/a2a", "jsonrpc/http", "0.2.1")
            .provider("Acme Corp", "https://acme.example.com")
            .streaming(true)
            .input_mode("text")
            .output_mode("text")
            .skill(AgentSkill::new("greet", "Greeter", "Says hello"))
            .build();

        assert_eq!(card.name, "test-agent");
        assert_eq!(card.version, "1.0.0");
        assert_eq!(card.supported_interfaces.len(), 1);
        assert_eq!(card.skills.len(), 1);
        assert_eq!(card.capabilities.streaming, Some(true));
    }

    #[test]
    #[should_panic(expected = "version")]
    fn builder_panics_without_version() {
        AgentCard::builder("x", "y")
            .interface("http://localhost", "jsonrpc/http", "0.2.1")
            .build();
    }

    #[test]
    #[should_panic(expected = "interface")]
    fn builder_panics_without_interfaces() {
        AgentCard::builder("x", "y").version("1.0.0").build();
    }

    #[test]
    fn card_roundtrip_json() {
        let card = AgentCard::builder("roundtrip", "RT agent")
            .version("0.1.0")
            .interface("https://rt.example.com", "jsonrpc/http", "0.2.1")
            .build();

        let json = serde_json::to_string(&card).unwrap();
        let deser: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "roundtrip");
        assert_eq!(deser.version, "0.1.0");
    }

    #[test]
    fn security_scheme_serde_tag() {
        let scheme = SecurityScheme::ApiKey {
            description: None,
            location: "header".into(),
            name: "X-Api-Key".into(),
        };
        let json = serde_json::to_value(&scheme).unwrap();
        assert_eq!(json["type"], "api_key");
        assert_eq!(json["location"], "header");
    }

    #[test]
    fn oauth_flows_serde_tag() {
        let flow = OAuthFlows::AuthorizationCode {
            authorization_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            refresh_url: None,
            scopes: HashMap::new(),
        };
        let json = serde_json::to_value(&flow).unwrap();
        assert_eq!(json["type"], "authorization_code");
    }
}
