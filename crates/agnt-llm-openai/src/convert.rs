//! Converts between agnt-llm generic types and OpenAI Responses API wire format.

use agnt_llm::request::{
    AssistantPart, GenerateRequest, Message, SystemPart, ToolChoice, UserPart,
};

use crate::types::{InputContent, InputItem, OpenAIRequest, OpenAITool, ReasoningConfig, Role};

pub fn to_openai_request(model_id: &str, req: &GenerateRequest) -> OpenAIRequest {
    // The Responses API takes `instructions` separately (system message).
    // We pull the first system message out and put the rest in `input`.
    let mut instructions: Option<String> = None;
    let mut input: Vec<InputItem> = Vec::new();

    for msg in &req.messages {
        match msg {
            Message::System { parts } => {
                // Concatenate system parts into instructions
                let text: String = parts
                    .iter()
                    .map(|p| match p {
                        SystemPart::Text(t) => t.text.as_str(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                // Use the last system message as instructions
                instructions = Some(text);
            }
            Message::User { parts } => {
                let content: Vec<InputContent> = parts
                    .iter()
                    .map(|p| match p {
                        UserPart::Text(t) => InputContent::InputText {
                            text: t.text.clone(),
                        },
                        UserPart::Image(img) => InputContent::InputImage {
                            url: img.url.clone(),
                        },
                    })
                    .collect();
                input.push(InputItem::Message {
                    role: Role::User,
                    content,
                });
            }
            Message::Assistant { parts } => {
                // For the Responses API, assistant text goes as a message,
                // and tool calls become separate function_call output items
                // that were already returned in a previous turn.
                let text_content: Vec<InputContent> = parts
                    .iter()
                    .filter_map(|p| match p {
                        AssistantPart::Text(t) => Some(InputContent::InputText {
                            text: t.text.clone(),
                        }),
                        AssistantPart::ToolCall(_) => None,
                    })
                    .collect();
                if !text_content.is_empty() {
                    input.push(InputItem::Message {
                        role: Role::Assistant,
                        content: text_content,
                    });
                }
            }
            Message::Tool { parts } => {
                for part in parts {
                    input.push(InputItem::FunctionCallOutput {
                        call_id: part.tool_call_id.clone(),
                        output: part.content.clone(),
                    });
                }
            }
        }
    }

    let tools: Vec<OpenAITool> = req
        .tools
        .iter()
        .map(|t| OpenAITool::Function {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.to_json_schema(),
            strict: true,
        })
        .collect();

    let tool_choice = match &req.options.tool_choice {
        ToolChoice::Auto => None, // omit = auto
        ToolChoice::None => Some(serde_json::json!("none")),
        ToolChoice::Required => Some(serde_json::json!("required")),
        ToolChoice::Tool(name) => Some(serde_json::json!({
            "type": "function",
            "name": name,
        })),
    };

    let reasoning = req
        .metadata
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(|effort| ReasoningConfig {
            effort: effort.to_string(),
        });

    OpenAIRequest {
        model: model_id.to_string(),
        input,
        stream: true,
        instructions,
        max_output_tokens: req.options.max_tokens,
        temperature: req.options.temperature,
        top_p: req.options.top_p,
        tools,
        tool_choice,
        reasoning,
    }
}
