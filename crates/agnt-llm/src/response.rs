use crate::error::Error;
use crate::request::ToolCallPart;
use crate::stream::{FinishReason, StreamEvent, Usage};
use futures::Stream;
use std::pin::Pin;
use tokio_stream::StreamExt;

/// A live streaming response from a language model.
///
/// Consume it event-by-event via [`events()`](Response::events), or collect
/// the full result with [`into_result()`](Response::into_result).
pub struct Response {
    inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, Error>> + Send>>,
}

impl Response {
    pub fn new(stream: impl Stream<Item = Result<StreamEvent, Error>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(stream),
        }
    }

    /// Consume the response as an async stream of events.
    pub fn events(self) -> Pin<Box<dyn Stream<Item = Result<StreamEvent, Error>> + Send>> {
        self.inner
    }

    /// Collect the full streamed response into a single result.
    pub async fn into_result(self) -> Result<GenerateResult, Error> {
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCallPart> = Vec::new();
        let mut finish_reason = None;
        let mut usage = None;

        let mut stream = self.inner;
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                }
                StreamEvent::ToolCallEnd { call, .. } => {
                    tool_calls.push(call);
                }
                StreamEvent::Finish {
                    reason,
                    usage: u,
                } => {
                    finish_reason = Some(reason);
                    usage = u;
                }
                StreamEvent::Error(message) => {
                    return Err(Error::Other(message));
                }
                // ToolCallBegin / ToolCallDelta are intermediate; we only
                // care about the fully-assembled ToolCallEnd.
                _ => {}
            }
        }

        Ok(GenerateResult {
            text,
            tool_calls,
            finish_reason: finish_reason.unwrap_or(FinishReason::Stop),
            usage: usage.unwrap_or_default(),
        })
    }
}

/// The collected result of a language model generation.
#[derive(Debug, Clone)]
pub struct GenerateResult {
    pub text: String,
    pub tool_calls: Vec<ToolCallPart>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}
