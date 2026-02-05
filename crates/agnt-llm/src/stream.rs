use crate::request::ToolCallPart;
use serde::{Deserialize, Serialize};

/// An event emitted during streaming generation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of text output.
    TextDelta(String),

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
