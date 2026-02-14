//! Converts between agnt-llm generic types and OpenAI Responses API wire format.

use agnt_llm::request::{
    AssistantPart, GenerateRequest, Message, SystemPart, ToolChoice, UserPart,
};

use crate::OpenAIConfig;
use crate::types::{
    InputContent, InputItem, OpenAIRequest, OpenAITool, ReasoningConfig, ReasoningSummary, Role,
};

pub fn to_openai_request(
    model_id: &str,
    req: &GenerateRequest,
    config: &OpenAIConfig,
) -> OpenAIRequest {
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
                    id: None,
                    role: Role::User,
                    content,
                });
            }
            Message::Assistant { parts } => {
                // The Responses API requires output items in their original
                // order. Reasoning must precede its associated content
                // (function_call or message). We walk the parts in arrival
                // order, batching consecutive Text parts into a single
                // assistant Message item and emitting Reasoning / ToolCall
                // as separate items.
                let mut text_buf: Vec<InputContent> = Vec::new();
                // Track the message item ID from the first TextPart that has one.
                let mut text_item_id: Option<String> = None;

                // Helper closure: flush accumulated text parts into an
                // assistant message input item.
                let flush_text = |buf: &mut Vec<InputContent>,
                                  id: &mut Option<String>,
                                  out: &mut Vec<InputItem>| {
                    if !buf.is_empty() {
                        out.push(InputItem::Message {
                            id: id.take(),
                            role: Role::Assistant,
                            content: std::mem::take(buf),
                        });
                    }
                };

                for part in parts {
                    match part {
                        AssistantPart::Text(t) => {
                            // Capture the message item ID if we haven't yet.
                            if text_item_id.is_none() {
                                text_item_id = t.metadata.get("openai:item_id").cloned();
                            }
                            text_buf.push(InputContent::OutputText {
                                text: t.text.clone(),
                            });
                        }
                        AssistantPart::Reasoning(r) => {
                            // Flush any preceding text before the reasoning item.
                            flush_text(&mut text_buf, &mut text_item_id, &mut input);

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
                            // Flush any preceding text before the tool call item.
                            flush_text(&mut text_buf, &mut text_item_id, &mut input);

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
                    }
                }

                // Flush any trailing text.
                flush_text(&mut text_buf, &mut text_item_id, &mut input);
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
            strict: false,
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

    let reasoning_effort = req
        .metadata
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let reasoning_summary = req
        .metadata
        .get("reasoning_summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let reasoning = if reasoning_effort.is_some() || reasoning_summary.is_some() {
        Some(ReasoningConfig {
            effort: reasoning_effort,
            summary: reasoning_summary,
        })
    } else {
        None
    };

    OpenAIRequest {
        model: model_id.to_string(),
        input,
        stream: true,
        store: config.response_store,
        include: if config.include_reasoning_encrypted_content {
            vec!["reasoning.encrypted_content".to_string()]
        } else {
            Vec::new()
        },
        instructions,
        max_output_tokens: req.options.max_tokens,
        temperature: req.options.temperature,
        top_p: req.options.top_p,
        tools,
        tool_choice,
        reasoning,
    }
}
