//! Streaming inference types and trait.
//!
//! [`StreamProvider`] extends [`Provider`] with streaming inference.
//! [`StreamEvent`] represents incremental chunks from the model.
//! The caller accumulates events into a final [`InferResponse`].

use crate::infer::InferResponse;
use crate::provider::ProviderError;
use crate::types::{TokenUsage, ToolSchema};
use layer0::context::Message;
use std::future::Future;

/// A single incremental event from a streaming inference call.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of text content from the model.
    TextDelta(String),

    /// The model started a tool call. Subsequent `ToolCallDelta` events
    /// carry the input JSON incrementally.
    ToolCallStart {
        /// Index of this tool call in the response (for parallel tool calls).
        index: usize,
        /// Provider-assigned unique ID.
        id: String,
        /// Name of the tool to invoke.
        name: String,
    },

    /// Incremental JSON input for an in-progress tool call.
    ToolCallDelta {
        /// Index of the tool call this delta belongs to.
        index: usize,
        /// JSON fragment to append.
        json_delta: String,
    },

    /// Token usage update (may arrive mid-stream or at the end).
    Usage(TokenUsage),

    /// Streaming is complete. Contains the fully-accumulated response.
    ///
    /// After this event, no more events will be emitted.
    Done(InferResponse),
}

/// Request for a streaming inference call.
///
/// Same shape as [`InferRequest`], but used with [`StreamProvider::infer_stream`].
#[derive(Debug, Clone)]
pub struct StreamRequest {
    /// Model to use. `None` = provider default.
    pub model: Option<String>,

    /// Conversation messages in layer0 format.
    pub messages: Vec<Message>,

    /// Available tools the model may call.
    pub tools: Vec<ToolSchema>,

    /// Maximum output tokens.
    pub max_tokens: Option<u32>,

    /// Sampling temperature.
    pub temperature: Option<f64>,

    /// System prompt.
    pub system: Option<String>,

    /// Provider-specific config passthrough.
    pub extra: serde_json::Value,
}

impl From<crate::infer::InferRequest> for StreamRequest {
    fn from(req: crate::infer::InferRequest) -> Self {
        Self {
            model: req.model,
            messages: req.messages,
            tools: req.tools,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            system: req.system,
            extra: req.extra,
        }
    }
}

/// Callback-based streaming interface for providers.
///
/// Providers that support streaming implement this trait alongside [`Provider`].
/// The streaming model uses a callback instead of `Stream` to avoid pinning
/// complexity and `async-stream` dependencies at the trait boundary.
///
/// ```ignore
/// use neuron_turn::stream::{StreamProvider, StreamEvent, StreamRequest};
///
/// async fn example(provider: &impl StreamProvider) {
///     let request = StreamRequest { /* ... */ };
///     let response = provider.infer_stream(request, |event| {
///         match event {
///             StreamEvent::TextDelta(text) => print!("{text}"),
///             StreamEvent::Done(response) => println!("\nDone!"),
///             _ => {}
///         }
///     }).await.unwrap();
/// }
/// ```
pub trait StreamProvider: crate::provider::Provider {
    /// Run streaming inference.
    ///
    /// The `on_event` callback is called for each streaming chunk. The final
    /// [`StreamEvent::Done`] event contains the accumulated [`InferResponse`].
    ///
    /// Returns the complete [`InferResponse`] when streaming finishes.
    fn infer_stream(
        &self,
        request: StreamRequest,
        on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
    ) -> impl Future<Output = Result<InferResponse, ProviderError>> + Send;
}

/// Blanket fallback: any Provider can "stream" by doing a single non-streaming
/// call and emitting the result as one `Done` event.
///
/// This means `stream_react_loop` works with non-streaming providers — they
/// just don't get incremental output. No separate code path needed.
pub async fn infer_stream_fallback<P: crate::provider::Provider>(
    provider: &P,
    request: StreamRequest,
    on_event: impl Fn(StreamEvent) + Send + Sync + 'static,
) -> Result<InferResponse, ProviderError> {
    let infer_request = crate::infer::InferRequest {
        model: request.model,
        messages: request.messages,
        tools: request.tools,
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        system: request.system,
        extra: request.extra,
    };

    let response = provider.infer(infer_request).await?;

    // Emit text as a single delta if present
    if let Some(text) = response.text() {
        on_event(StreamEvent::TextDelta(text.to_string()));
    }

    // Emit tool call starts
    for (i, call) in response.tool_calls.iter().enumerate() {
        on_event(StreamEvent::ToolCallStart {
            index: i,
            id: call.id.clone(),
            name: call.name.clone(),
        });
        on_event(StreamEvent::ToolCallDelta {
            index: i,
            json_delta: call.input.to_string(),
        });
    }

    // Emit usage
    on_event(StreamEvent::Usage(response.usage.clone()));

    // Emit done
    on_event(StreamEvent::Done(response.clone()));

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn stream_request_from_infer_request() {
        let infer = crate::infer::InferRequest::new(vec![]);
        let stream: StreamRequest = infer.into();
        assert!(stream.messages.is_empty());
        assert!(stream.model.is_none());
    }

    #[test]
    fn stream_event_text_delta() {
        let event = StreamEvent::TextDelta("hello".into());
        assert!(matches!(event, StreamEvent::TextDelta(s) if s == "hello"));
    }

    #[test]
    fn stream_event_tool_call_start() {
        let event = StreamEvent::ToolCallStart {
            index: 0,
            id: "tc_1".into(),
            name: "search".into(),
        };
        assert!(matches!(event, StreamEvent::ToolCallStart { index: 0, .. }));
    }

    #[tokio::test]
    async fn fallback_emits_text_and_done() {
        use crate::test_utils::TestProvider;

        let provider = TestProvider::new();
        provider.respond_with_text("hello world");

        let request = StreamRequest {
            model: None,
            messages: vec![],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            extra: serde_json::Value::Null,
        };

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let events_clone = Arc::clone(&events);

        let response = infer_stream_fallback(&provider, request, move |event| {
            let label = match &event {
                StreamEvent::TextDelta(t) => format!("text:{t}"),
                StreamEvent::Done(_) => "done".into(),
                StreamEvent::Usage(_) => "usage".into(),
                _ => "other".into(),
            };
            events_clone.lock().unwrap().push(label);
        })
        .await
        .unwrap();

        assert_eq!(response.text().unwrap(), "hello world");

        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|e| e.starts_with("text:")));
        assert!(captured.iter().any(|e| e == "done"));
        assert!(captured.iter().any(|e| e == "usage"));
    }
}
