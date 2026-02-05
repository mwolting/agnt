//! Converts between agnt-llm generic types and OpenAI Responses API wire format.

use agnt_llm::request::{
    AssistantPart, GenerateRequest, Message, SystemPart, ToolChoice, UserPart,
};

use crate::types::{
    InputContent, InputItem, OpenAIRequest, OpenAITool, ReasoningConfig, ReasoningSummary, Role,
};

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
                // For the Responses API, assistant text uses "output_text"
                // content type (not "input_text"), and tool calls / reasoning
                // become separate input items.
                let text_content: Vec<InputContent> = parts
                    .iter()
                    .filter_map(|p| match p {
                        AssistantPart::Text(t) => Some(InputContent::OutputText {
                            text: t.text.clone(),
                        }),
                        _ => None,
                    })
                    .collect();
                if !text_content.is_empty() {
                    input.push(InputItem::Message {
                        role: Role::Assistant,
                        content: text_content,
                    });
                }
                // Emit reasoning and tool calls as separate input items.
                // Order matters: reasoning must precede the function_call
                // items it produced.
                for part in parts {
                    match part {
                        AssistantPart::Reasoning(r) => {
                            let item_id = r
                                .metadata
                                .get("openai:item_id")
                                .cloned()
                                .unwrap_or_default();
                            let encrypted_content =
                                r.metadata.get("openai:encrypted_content").cloned();
                            let summary: Vec<ReasoningSummary> = r
                                .text
                                .iter()
                                .map(|t| ReasoningSummary::SummaryText { text: t.clone() })
                                .collect();
                            input.push(InputItem::Reasoning {
                                id: item_id,
                                summary,
                                encrypted_content,
                            });
                        }
                        AssistantPart::ToolCall(tc) => {
                            let item_id = tc
                                .metadata
                                .get("openai:item_id")
                                .cloned()
                                .unwrap_or_else(|| tc.id.clone());
                            input.push(InputItem::FunctionCall {
                                id: item_id,
                                call_id: tc.id.clone(),
                                name: tc.name.clone(),
                                arguments: tc.arguments.clone(),
                            });
                        }
                        AssistantPart::Text(_) => {} // handled above
                    }
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
