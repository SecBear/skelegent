//! Streaming inference types.
//!
//! [`InferStream`] wraps a `Stream<Item = Result<StreamEvent, ProviderError>>` and is
//! returned by [`crate::provider::Provider::infer_stream`]. Consumers poll it via
//! `StreamExt::next()` from `futures_util` or `tokio_stream`.
//!
//! Cancellation is implicit: dropping an `InferStream` drops the inner stream, which
//! cancels the underlying HTTP request at the next `.await` point.

use crate::infer::InferResponse;
use crate::provider::ProviderError;
use crate::types::TokenUsage;
use futures_core::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A single incremental event from a streaming inference call.
#[non_exhaustive]
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

    /// Incremental thinking/reasoning text.
    ThinkingDelta(String),

    /// Streaming is complete. Contains the fully-accumulated response.
    ///
    /// After this event, no more events will be emitted. Every provider
    /// implementation **must** emit exactly one `Done` event as the last item.
    Done(InferResponse),
}

/// A streaming inference response.
///
/// Wraps a `Stream<Item = Result<StreamEvent, ProviderError>>` for real-time
/// consumption. The stream ends with a [`StreamEvent::Done`] event that carries
/// the fully-accumulated [`InferResponse`].
///
/// `InferStream` is [`Unpin`] because `Pin<Box<T>>` is always `Unpin` regardless
/// of `T`. You can call `StreamExt::next()` on it directly without `pin_mut!`.
///
/// # Example
///
/// ```rust,ignore
/// use futures_util::StreamExt;
///
/// let mut stream = provider.infer_stream(request).await?;
/// while let Some(event) = stream.next().await {
///     match event? {
///         StreamEvent::TextDelta(text) => print!("{text}"),
///         StreamEvent::Done(response) => { /* use final response */ }
///         _ => {}
///     }
/// }
/// ```
pub struct InferStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>,
}

impl InferStream {
    /// Create from any `Send + 'static` stream of `StreamEvent` results.
    pub fn new(
        stream: impl Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Box::pin(stream),
        }
    }
}

impl Stream for InferStream {
    type Item = Result<StreamEvent, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Create an `InferStream` that emits a single [`StreamEvent::Done`] wrapping `response`.
///
/// This is used by [`crate::provider::Provider::infer_stream`]'s default implementation
/// for providers that do not support native token-by-token streaming.
pub fn single_response_stream(response: InferResponse) -> InferStream {
    struct OnceStream {
        event: Option<Result<StreamEvent, ProviderError>>,
    }

    impl Stream for OnceStream {
        type Item = Result<StreamEvent, ProviderError>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.event.take())
        }
    }

    InferStream::new(OnceStream {
        event: Some(Ok(StreamEvent::Done(response))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Verify that `single_response_stream` yields exactly one `Done` event then `None`.
    #[tokio::test]
    async fn single_response_stream_yields_done() {
        use futures_util::StreamExt;

        use layer0::content::Content;
        let response = crate::infer::InferResponse {
            content: Content::text("hello"),
            tool_calls: vec![],
            stop_reason: crate::types::StopReason::EndTurn,
            usage: crate::types::TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let mut stream = single_response_stream(response);

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, StreamEvent::Done(ref r) if r.text().unwrap() == "hello"));

        // Stream must be exhausted after Done.
        assert!(stream.next().await.is_none());
    }

    /// Default `infer_stream` impl (via TestProvider) wraps the response in Done.
    #[tokio::test]
    async fn infer_stream_default_impl_wraps_response() {
        use crate::provider::Provider;
        use crate::test_utils::TestProvider;
        use futures_util::StreamExt;

        let provider = TestProvider::new();
        provider.respond_with_text("hello world");

        let request = crate::infer::InferRequest::new(vec![]);
        let mut stream = provider.infer_stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        match event {
            StreamEvent::Done(resp) => {
                assert_eq!(resp.text().unwrap(), "hello world");
            }
            other => panic!("expected Done, got {other:?}"),
        }

        // Exactly one item: Done.
        assert!(stream.next().await.is_none());
    }

    /// Verify InferStream can be collected into a Vec (tests Stream trait impl).
    #[tokio::test]
    async fn stream_to_response_collect() {
        use futures_util::StreamExt;

        use layer0::content::Content;
        let response = crate::infer::InferResponse {
            content: Content::text("collected"),
            tool_calls: vec![],
            stop_reason: crate::types::StopReason::EndTurn,
            usage: crate::types::TokenUsage::default(),
            model: "test".into(),
            cost: None,
            truncated: None,
        };
        let stream = single_response_stream(response);
        let events: Vec<_> = stream.collect().await;

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Ok(StreamEvent::Done(_))));
    }
}
