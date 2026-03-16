//! Typed request/response for embedding operations.
//!
//! These types define the contract for [`Provider::embed()`]. Providers that
//! support embedding override the default method; providers that don't inherit
//! an error return.

use crate::types::TokenUsage;
use serde::{Deserialize, Serialize};

/// Request to embed one or more texts into vector space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRequest {
    /// Texts to embed. Each text becomes one embedding vector.
    pub texts: Vec<String>,
    /// Model override (e.g. `"text-embedding-3-small"`). `None` = provider default.
    pub model: Option<String>,
    /// Requested dimensions. Provider may truncate or ignore.
    pub dimensions: Option<usize>,
}

impl EmbedRequest {
    /// Create an embed request from a list of texts.
    pub fn new(texts: Vec<String>) -> Self {
        Self {
            texts,
            model: None,
            dimensions: None,
        }
    }

    /// Set the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the requested dimensions.
    pub fn with_dimensions(mut self, dimensions: usize) -> Self {
        self.dimensions = Some(dimensions);
        self
    }
}

/// A single embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embedding {
    /// The embedding vector.
    pub vector: Vec<f32>,
}

/// Response from an embedding operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    /// One embedding per input text, in order.
    pub embeddings: Vec<Embedding>,
    /// Model that produced the embeddings.
    pub model: String,
    /// Token usage for this call.
    pub usage: TokenUsage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_request_builder() {
        let req = EmbedRequest::new(vec!["hello".into(), "world".into()])
            .with_model("text-embedding-3-small")
            .with_dimensions(256);
        assert_eq!(req.texts.len(), 2);
        assert_eq!(req.model.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(req.dimensions, Some(256));
    }

    #[test]
    fn embed_response_serde_round_trip() {
        let resp = EmbedResponse {
            embeddings: vec![
                Embedding {
                    vector: vec![0.1, 0.2, 0.3],
                },
                Embedding {
                    vector: vec![0.4, 0.5, 0.6],
                },
            ],
            model: "test-model".into(),
            usage: TokenUsage::default(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: EmbedResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.embeddings.len(), 2);
        assert_eq!(back.embeddings[0].vector, vec![0.1, 0.2, 0.3]);
        assert_eq!(back.model, "test-model");
    }

    #[test]
    fn embed_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EmbedRequest>();
        assert_send_sync::<EmbedResponse>();
        assert_send_sync::<Embedding>();
    }
}
