//! Opens an SSE connection to the OpenAI Responses API and maps events
//! to the agnt-llm `StreamEvent` type.

use crate::ProviderState;
use crate::types::{
    FunctionCallArgumentsDelta, OpenAIRequest, OutputItem, OutputItemAdded, OutputItemComplete,
    OutputItemDone, OutputTextDelta, ReasoningSummaryTextDelta, ResponseCompleted,
};
use agnt_llm::error::Error;
use agnt_llm::request::{ReasoningPart, ToolCallPart};
use agnt_llm::stream::{FinishReason, StreamEvent, Usage};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use eventsource_stream::Eventsource;
use futures::Stream;
use std::sync::Arc;
use tokio_stream::StreamExt;

pub fn open(
    state: Arc<ProviderState>,
    body: OpenAIRequest,
) -> impl Stream<Item = Result<StreamEvent, Error>> + Send {
    async_stream::try_stream! {
        // Fire the HTTP request
        let url = format!("{}/responses", state.config.base_url);
        let mut req = state
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", state.config.auth_token));

        if state.config.include_chatgpt_account_id_header
            && let Some(account_id) = extract_chatgpt_account_id(&state.config.auth_token)
        {
            req = req.header("chatgpt-account-id", account_id);
        }
        for (k, v) in &state.config.extra_headers {
            req = req.header(k, v);
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(Box::new(e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            Err(Error::Api {
                code: status.as_str().to_string(),
                message: body_text,
                metadata: Default::default(),
            })?;
            unreachable!();
        }

        let mut sse = resp.bytes_stream().eventsource();
        let mut mapper = EventMapper::new();

        while let Some(event) = sse.next().await {
            match event {
                Ok(event) => {
                    if let Some(stream_event) = mapper.map_event(&event.event, &event.data)? {
                        yield stream_event;
                    }
                }
                Err(e) => {
                    Err(Error::Sse(e.to_string()))?;
                }
            }
        }
    }
}

fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = parts[1];
    let decoded = URL_SAFE_NO_PAD.decode(payload.as_bytes()).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(ToString::to_string)
}

// ---------------------------------------------------------------------------
// Event mapper (stateful — tracks tool call indices)
// ---------------------------------------------------------------------------

struct EventMapper {
    /// Counter for tool call indices we expose to the consumer.
    tool_call_index: usize,
    /// Maps OpenAI output item ID -> our tool call index.
    id_to_index: std::collections::HashMap<String, usize>,
    /// Whether we saw any tool calls (to determine finish reason).
    has_tool_calls: bool,
    /// Tracks the current reasoning item ID (set on output_item.added).
    current_reasoning_id: Option<String>,
    /// Tracks the current message item ID (set on output_item.added).
    current_message_id: Option<String>,
}

impl EventMapper {
    fn new() -> Self {
        Self {
            tool_call_index: 0,
            id_to_index: std::collections::HashMap::new(),
            has_tool_calls: false,
            current_reasoning_id: None,
            current_message_id: None,
        }
    }

    fn map_event(&mut self, event_type: &str, data: &str) -> Result<Option<StreamEvent>, Error> {
        match event_type {
            "response.output_text.delta" => {
                let parsed: OutputTextDelta = serde_json::from_str(data)?;
                Ok(Some(StreamEvent::TextDelta(parsed.delta)))
            }

            "response.output_item.added" => {
                let parsed: OutputItemAdded = serde_json::from_str(data)?;
                match parsed.item {
                    OutputItem::Reasoning { id } => {
                        self.current_reasoning_id = Some(id);
                        Ok(None)
                    }
                    OutputItem::Message { id } => {
                        self.current_message_id = Some(id);
                        Ok(None)
                    }
                    OutputItem::FunctionCall { id, name, call_id } => {
                        let index = self.tool_call_index;
                        self.tool_call_index += 1;
                        self.id_to_index.insert(id, index);
                        self.has_tool_calls = true;
                        Ok(Some(StreamEvent::ToolCallBegin {
                            index,
                            id: call_id,
                            name,
                        }))
                    }
                    _ => Ok(None),
                }
            }

            "response.reasoning_summary_text.delta" => {
                let parsed: ReasoningSummaryTextDelta = serde_json::from_str(data)?;
                Ok(Some(StreamEvent::ReasoningDelta(parsed.delta)))
            }

            "response.function_call_arguments.delta" => {
                let parsed: FunctionCallArgumentsDelta = serde_json::from_str(data)?;
                let index = self.tool_call_index.saturating_sub(1);
                Ok(Some(StreamEvent::ToolCallDelta {
                    index,
                    arguments_delta: parsed.delta,
                }))
            }

            "response.output_item.done" => {
                let parsed: OutputItemDone = serde_json::from_str(data)?;
                match parsed.item {
                    OutputItemComplete::Reasoning {
                        id,
                        summary,
                        encrypted_content,
                    } => {
                        self.current_reasoning_id = None;
                        let text = summary.first().map(|s| match s {
                            crate::types::ReasoningSummary::SummaryText { text } => text.clone(),
                        });
                        let mut metadata = std::collections::HashMap::new();
                        metadata.insert("openai:item_id".to_string(), id);
                        if let Some(ec) = encrypted_content {
                            metadata.insert("openai:encrypted_content".to_string(), ec);
                        }
                        Ok(Some(StreamEvent::ReasoningDone(ReasoningPart {
                            text,
                            metadata,
                        })))
                    }
                    OutputItemComplete::Message { id, .. } => {
                        self.current_message_id = None;
                        let mut metadata = std::collections::HashMap::new();
                        metadata.insert("openai:item_id".to_string(), id);
                        Ok(Some(StreamEvent::TextDone { metadata }))
                    }
                    OutputItemComplete::FunctionCall {
                        id,
                        call_id,
                        name,
                        arguments,
                    } => {
                        let index = self.id_to_index.get(&id).copied().unwrap_or(0);
                        let mut metadata = std::collections::HashMap::new();
                        metadata.insert("openai:item_id".to_string(), id);
                        Ok(Some(StreamEvent::ToolCallEnd {
                            index,
                            call: ToolCallPart {
                                id: call_id,
                                name,
                                arguments,
                                metadata,
                            },
                        }))
                    }
                    _ => Ok(None),
                }
            }

            "response.completed" => {
                let parsed: ResponseCompleted = serde_json::from_str(data)?;
                let usage = parsed.response.usage.map(|u| Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    reasoning_tokens: u.output_tokens_details.and_then(|d| d.reasoning_tokens),
                    cached_tokens: u.input_tokens_details.and_then(|d| d.cached_tokens),
                });
                let reason = if self.has_tool_calls {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                };
                Ok(Some(StreamEvent::Finish { reason, usage }))
            }

            "error" => Ok(Some(StreamEvent::Error(data.to_string()))),

            // Events we don't need: response.created, response.in_progress,
            // response.output_text.done, response.content_part.added/done,
            // response.reasoning_summary_part.added/done,
            // response.reasoning_summary_text.done, etc.
            _ => Ok(None),
        }
    }
}
