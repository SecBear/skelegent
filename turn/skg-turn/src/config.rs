//! Configuration for Turn.

/// Static configuration for a Turn instance.
///
/// Per-request overrides come from `OperatorInput.config` (layer0's `OperatorConfig`).
/// This struct holds the defaults.
pub struct TurnConfig {
    /// Base system prompt for this turn implementation.
    pub system_prompt: String,

    /// Default model identifier.
    pub default_model: String,

    /// Default maximum output tokens per provider call.
    pub default_max_tokens: u32,

    /// Default maximum ReAct loop iterations.
    pub default_max_turns: u32,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            system_prompt: "You are a helpful assistant.".into(),
            default_model: String::new(),
            default_max_tokens: 4096,
            default_max_turns: 25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = TurnConfig::default();
        assert_eq!(config.system_prompt, "You are a helpful assistant.");
        assert!(config.default_model.is_empty());
        assert_eq!(config.default_max_tokens, 4096);
        assert_eq!(config.default_max_turns, 25);
    }

    #[test]
    fn custom_config_values() {
        let config = TurnConfig {
            system_prompt: "Custom prompt".into(),
            default_model: "gpt-4o".into(),
            default_max_tokens: 2048,
            default_max_turns: 10,
        };
        assert_eq!(config.system_prompt, "Custom prompt");
        assert_eq!(config.default_model, "gpt-4o");
        assert_eq!(config.default_max_tokens, 2048);
        assert_eq!(config.default_max_turns, 10);
    }
}
