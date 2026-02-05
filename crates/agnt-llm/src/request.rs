use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Top-level request
// ---------------------------------------------------------------------------

/// A request to generate a language model response.
#[derive(Default, Debug, Clone)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub options: GenerateOptions,
    /// Provider-specific metadata. Passed through to the backend as-is.
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Knobs that control generation behavior.
#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub stop: Option<Vec<String>>,
    pub tool_choice: ToolChoice,
}

// ---------------------------------------------------------------------------
// Reusable part types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePart {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPart {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Role-specific part enums (composed from reusable parts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SystemPart {
    Text(TextPart),
}

#[derive(Debug, Clone)]
pub enum UserPart {
    Text(TextPart),
    Image(ImagePart),
}

#[derive(Debug, Clone)]
pub enum AssistantPart {
    Text(TextPart),
    ToolCall(ToolCallPart),
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    System { parts: Vec<SystemPart> },
    User { parts: Vec<UserPart> },
    Assistant { parts: Vec<AssistantPart> },
    Tool { parts: Vec<ToolResultPart> },
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Message::System {
            parts: vec![SystemPart::Text(TextPart { text: text.into() })],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Message::User {
            parts: vec![UserPart::Text(TextPart { text: text.into() })],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Message::Assistant {
            parts: vec![AssistantPart::Text(TextPart { text: text.into() })],
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message::Tool {
            parts: vec![ToolResultPart {
                tool_call_id: tool_call_id.into(),
                content: content.into(),
            }],
        }
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// A tool the model can call.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Schema,
}

/// Controls how the model selects tools.
#[derive(Debug, Clone, Default)]
pub enum ToolChoice {
    #[default]
    Auto,
    None,
    Required,
    /// Force calling a specific tool by name.
    Tool(String),
}

// ---------------------------------------------------------------------------
// Schema descriptor — Rust-native, converts to JSON Schema downstream
// ---------------------------------------------------------------------------

/// A Rust-native description of a value's shape, convertible to JSON Schema.
#[derive(Debug, Clone)]
pub enum Schema {
    String {
        description: Option<String>,
        enumeration: Option<Vec<String>>,
    },
    Number {
        description: Option<String>,
    },
    Integer {
        description: Option<String>,
    },
    Boolean {
        description: Option<String>,
    },
    Array {
        description: Option<String>,
        items: Box<Schema>,
    },
    Object {
        description: Option<String>,
        properties: Vec<Property>,
        required: Vec<String>,
    },
    /// Escape hatch: a literal JSON Schema value for cases we don't cover.
    Raw(serde_json::Value),
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub schema: Schema,
}

impl Schema {
    /// Convert to a JSON Schema `serde_json::Value`.
    pub fn to_json_schema(&self) -> serde_json::Value {
        match self {
            Schema::String {
                description,
                enumeration,
            } => {
                let mut obj = serde_json::json!({ "type": "string" });
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                if let Some(e) = enumeration {
                    obj["enum"] = serde_json::json!(e);
                }
                obj
            }
            Schema::Number { description } => {
                let mut obj = serde_json::json!({ "type": "number" });
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                obj
            }
            Schema::Integer { description } => {
                let mut obj = serde_json::json!({ "type": "integer" });
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                obj
            }
            Schema::Boolean { description } => {
                let mut obj = serde_json::json!({ "type": "boolean" });
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                obj
            }
            Schema::Array { description, items } => {
                let mut obj = serde_json::json!({
                    "type": "array",
                    "items": items.to_json_schema(),
                });
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                obj
            }
            Schema::Object {
                description,
                properties,
                required,
            } => {
                let props: serde_json::Map<String, serde_json::Value> = properties
                    .iter()
                    .map(|p| (p.name.clone(), p.schema.to_json_schema()))
                    .collect();
                let mut obj = serde_json::json!({
                    "type": "object",
                    "properties": props,
                });
                if !required.is_empty() {
                    obj["required"] = serde_json::json!(required);
                }
                if let Some(d) = description {
                    obj["description"] = serde_json::json!(d);
                }
                obj
            }
            Schema::Raw(v) => v.clone(),
        }
    }
}
