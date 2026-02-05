use crate::request::{ReasoningPart, ToolCallPart};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An event emitted during streaming generation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of text output.
    TextDelta(String),

    /// A text (message) output item is complete. Carries provider-specific
    /// metadata such as the message item ID needed for roundtripping.
    TextDone { metadata: HashMap<String, String> },

    /// A new tool call started.
    ToolCallBegin {
        index: usize,
        id: String,
        name: String,
    },

    /// A delta of tool call arguments (raw JSON string fragment).
    ToolCallDelta {
        index: usize,
        arguments_delta: String,
    },

    /// A tool call is complete and ready to execute.
    ToolCallEnd { index: usize, call: ToolCallPart },

    /// A chunk of reasoning summary text.
    ReasoningDelta(String),

    /// A reasoning item is complete.
    ReasoningDone(ReasoningPart),

    /// Generation is complete.
    Finish {
        reason: FinishReason,
        usage: Option<Usage>,
    },

    /// An error occurred mid-stream.
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Other(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}
