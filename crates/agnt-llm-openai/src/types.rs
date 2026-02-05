//! OpenAI Responses API wire types.
//!
//! These are the raw JSON shapes sent to / received from the API.
//! They are intentionally separate from the agnt-llm public types.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub input: Vec<InputItem>,
    pub stream: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAITool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: Role,
        content: Vec<InputContent>,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Developer,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    InputText { text: String },
    InputImage { url: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OpenAITool {
    #[serde(rename = "function")]
    Function {
        name: String,
        description: String,
        parameters: serde_json::Value,
        strict: bool,
    },
}

// ---------------------------------------------------------------------------
// SSE event types (only the ones we care about for streaming)
// ---------------------------------------------------------------------------

/// Parsed from the `data:` payload of each SSE event, keyed by `event:` type.
#[derive(Debug, Deserialize)]
pub struct ResponseObject {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub status: String,
    pub usage: Option<UsageObject>,
}

#[derive(Debug, Deserialize)]
pub struct UsageObject {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub input_tokens_details: Option<InputTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub struct OutputTokensDetails {
    pub reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct InputTokensDetails {
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct OutputItemAdded {
    pub item: OutputItem,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message {
        #[allow(dead_code)]
        id: String,
    },
    FunctionCall {
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        call_id: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct OutputTextDelta {
    pub delta: String,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCallArgumentsDelta {
    pub delta: String,
}

#[derive(Debug, Deserialize)]
pub struct OutputItemDone {
    pub item: OutputItemComplete,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItemComplete {
    Message {
        #[allow(dead_code)]
        id: String,
    },
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct ResponseCompleted {
    pub response: ResponseObject,
}
