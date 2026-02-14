use std::path::{Path, PathBuf};
use std::sync::Arc;

use agnt_llm::stream::{FinishReason, StreamEvent, Usage};
use agnt_llm::{LanguageModel, Message, RequestBuilder, ToolDefinition};
use handlebars::Handlebars;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::event::AgentEvent;
use crate::tool::{ErasedTool, Tool};
use crate::tools::{BashTool, EditTool, ReadTool, SkillTool};

// ---------------------------------------------------------------------------
// Agent state (shared between handle and spawned task)
// ---------------------------------------------------------------------------

struct AgentState {
    messages: Vec<Message>,
    tools: Vec<Box<dyn ErasedTool>>,
    agents_md: Option<String>,
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

/// Callback invoked on every `RequestBuilder` before it is built.
/// Use this to inject provider-specific options (e.g. reasoning effort/summary).
type ConfigureRequest = dyn Fn(&mut RequestBuilder) + Send + Sync;

/// The core agent. Holds a language model, conversation history, and
/// registered tools. UI-agnostic — communicates via [`AgentEvent`]s.
pub struct Agent {
    model: Arc<LanguageModel>,
    system_prompt: Option<String>,
    state: Arc<Mutex<AgentState>>,
    /// Optional callback applied to every outgoing request.
    configure_request: Option<Arc<ConfigureRequest>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationState {
    pub messages: Vec<Message>,
}

impl Agent {
    /// Create a new agent backed by the given model.
    pub fn new(model: LanguageModel) -> Self {
        Self {
            model: Arc::new(model),
            system_prompt: None,
            state: Arc::new(Mutex::new(AgentState {
                messages: Vec::new(),
                tools: Vec::new(),
                agents_md: None,
            })),
            configure_request: None,
        }
    }

    /// Create an agent with the default coding tools (read, edit, skill, bash)
    /// and a system prompt that turns it into a coding assistant.
    ///
    /// `cwd` is the working directory that file and bash tools operate in.
    pub fn with_defaults(model: LanguageModel, cwd: PathBuf) -> Self {
        let workspace_root = find_workspace_root(&cwd);
        let agents_md = load_agents_md(&workspace_root);
        let skills_dir = workspace_root.join(".agents").join("skills");

        let mut agent = Self::new(model);
        agent.system(system_prompt(&cwd, &workspace_root));

        {
            let mut s = agent.state.lock();
            s.agents_md = agents_md;
        }

        agent.tool(ReadTool { cwd: cwd.clone() });
        agent.tool(EditTool { cwd: cwd.clone() });
        agent.tool(SkillTool::new(skills_dir));
        agent.tool(BashTool { cwd });

        agent
    }

    /// Set the system prompt.
    pub fn system(&mut self, prompt: impl Into<String>) -> &mut Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set a callback that configures every outgoing request.
    ///
    /// Use this to inject provider-specific options that should apply to
    /// every generation call (e.g. reasoning effort, reasoning summary).
    ///
    /// ```ignore
    /// use agnt_llm_openai::{OpenAIRequestExt, ReasoningSummaryMode};
    ///
    /// agent.configure_request(|req| {
    ///     req.reasoning_summary(ReasoningSummaryMode::Detailed);
    /// });
    /// ```
    pub fn configure_request(
        &mut self,
        f: impl Fn(&mut RequestBuilder) + Send + Sync + 'static,
    ) -> &mut Self {
        self.configure_request = Some(Arc::new(f));
        self
    }

    /// Register a tool the model can call.
    pub fn tool(&mut self, tool: impl Tool) -> &mut Self {
        self.state.lock().tools.push(Box::new(tool));
        self
    }

    /// Access the conversation history (completed messages only).
    pub fn messages(&self) -> Vec<Message> {
        self.state.lock().messages.clone()
    }

    /// Snapshot conversation state that can be persisted and later restored.
    pub fn conversation_state(&self) -> ConversationState {
        ConversationState {
            messages: self.messages(),
        }
    }

    /// Replace in-memory conversation state with a previously saved snapshot.
    pub fn restore_conversation_state(&self, state: ConversationState) {
        self.state.lock().messages = state.messages;
    }

    /// Submit user input and get back a stream of events.
    ///
    /// The returned [`AgentStream`] yields [`AgentEvent`]s as the model
    /// generates a response. If tool calls occur, the agent executes them
    /// automatically and loops until the model produces a final text answer.
    ///
    /// Dropping the `AgentStream` cancels the generation.
    pub fn submit(&self, content: impl Into<String>) -> AgentStream {
        let content = content.into();
        let (tx, rx) = mpsc::channel(64);

        let model = Arc::clone(&self.model);
        let state = Arc::clone(&self.state);
        let system_prompt = self.system_prompt.clone();
        let configure_request = self.configure_request.clone();

        tokio::spawn(async move {
            generation_loop(model, state, system_prompt, configure_request, content, tx).await;
        });

        AgentStream { rx }
    }
}

// ---------------------------------------------------------------------------
// AgentStream
// ---------------------------------------------------------------------------

/// A stream of [`AgentEvent`]s from a single generation turn.
///
/// Implements async iteration via [`next()`](AgentStream::next).
/// Drop to cancel the in-flight generation.
pub struct AgentStream {
    rx: mpsc::Receiver<AgentEvent>,
}

impl AgentStream {
    /// Get the next event, or `None` when the turn is complete.
    pub async fn next(&mut self) -> Option<AgentEvent> {
        self.rx.recv().await
    }
}

// ---------------------------------------------------------------------------
// Generation loop (runs in spawned task)
// ---------------------------------------------------------------------------

async fn generation_loop(
    model: Arc<LanguageModel>,
    state: Arc<Mutex<AgentState>>,
    system_prompt: Option<String>,
    configure_request: Option<Arc<ConfigureRequest>>,
    content: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    // 1. Record user message and inject AGENTS.md once on first turn.
    {
        let mut s = state.lock();
        if s.messages.is_empty()
            && let Some(agents_md) = s.agents_md.take()
        {
            s.messages.push(Message::system(format!(
                "Repository instructions from AGENTS.md:\n\n{agents_md}"
            )));
        }
        s.messages.push(Message::user(&content));
    }
    if tx
        .send(AgentEvent::UserMessage {
            content: content.clone(),
        })
        .await
        .is_err()
    {
        return; // receiver dropped
    }

    let mut cumulative_usage = Usage::default();

    // 2. Generation loop (may iterate for tool calls)
    loop {
        // Build request from current state
        let request = {
            let s = state.lock();
            let mut req = agnt_llm::request();
            if let Some(ref system) = system_prompt {
                req.system(system.as_str());
            }
            req.messages(s.messages.clone());

            let tool_defs: Vec<ToolDefinition> = s.tools.iter().map(|t| t.definition()).collect();
            req.tools(tool_defs);

            // Apply caller-provided request configuration (e.g. reasoning options).
            if let Some(ref configure) = configure_request {
                configure(&mut req);
            }

            req.build()
        };

        // Stream the response. We collect AssistantParts in arrival order
        // so interleaved reasoning/text/tool-calls are preserved exactly.
        let mut stream = model.generate(request).events();
        let mut parts: Vec<agnt_llm::AssistantPart> = Vec::new();
        let mut text = String::new();
        let mut tool_calls: Vec<agnt_llm::ToolCallPart> = Vec::new();
        let mut finish_reason = FinishReason::Stop;

        // Helper: flush accumulated text deltas into a Text part with
        // optional metadata (e.g. the message item ID for roundtripping).
        macro_rules! flush_text {
            ($parts:expr, $text:expr) => {
                flush_text!($parts, $text, std::collections::HashMap::new())
            };
            ($parts:expr, $text:expr, $meta:expr) => {
                if !$text.is_empty() {
                    $parts.push(agnt_llm::AssistantPart::Text(agnt_llm::TextPart {
                        text: std::mem::take(&mut $text),
                        metadata: $meta,
                    }));
                }
            };
        }

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta(delta)) => {
                    text.push_str(&delta);
                    if tx
                        .send(AgentEvent::TextDelta {
                            delta: delta.clone(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamEvent::TextDone { metadata }) => {
                    // The text message item is complete. Flush accumulated
                    // text into a TextPart carrying the metadata (includes
                    // the message item ID needed for roundtripping).
                    flush_text!(parts, text, metadata);
                }
                Ok(StreamEvent::ReasoningDelta(delta)) => {
                    if tx
                        .send(AgentEvent::ReasoningDelta {
                            delta: delta.clone(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamEvent::ReasoningDone(part)) => {
                    flush_text!(parts, text);
                    parts.push(agnt_llm::AssistantPart::Reasoning(part));
                }
                Ok(StreamEvent::ToolCallBegin { .. }) => {
                    // Wire-level detail; we emit ToolCallStart after we have
                    // the complete call in ToolCallEnd.
                }
                Ok(StreamEvent::ToolCallDelta { .. }) => {
                    // Wire-level streaming of arguments; ignored — we wait
                    // for the complete call.
                }
                Ok(StreamEvent::ToolCallEnd { call, .. }) => {
                    flush_text!(parts, text);
                    tool_calls.push(call.clone());
                    parts.push(agnt_llm::AssistantPart::ToolCall(call));
                }
                Ok(StreamEvent::Finish { reason, usage }) => {
                    finish_reason = reason;
                    if let Some(u) = usage {
                        cumulative_usage.input_tokens += u.input_tokens;
                        cumulative_usage.output_tokens += u.output_tokens;
                        if let Some(r) = u.reasoning_tokens {
                            *cumulative_usage.reasoning_tokens.get_or_insert(0) += r;
                        }
                        if let Some(c) = u.cached_tokens {
                            *cumulative_usage.cached_tokens.get_or_insert(0) += c;
                        }
                    }
                }
                Ok(StreamEvent::Error(msg)) => {
                    let _ = tx.send(AgentEvent::Error { error: msg }).await;
                    return;
                }
                Err(e) => {
                    let _ = tx
                        .send(AgentEvent::Error {
                            error: e.to_string(),
                        })
                        .await;
                    return;
                }
            }
        }

        // Flush any trailing text
        flush_text!(parts, text);

        // Record the assistant message with parts in arrival order
        {
            let mut s = state.lock();
            if !parts.is_empty() {
                s.messages.push(Message::Assistant { parts });
            }
        }

        // If no tool calls, we're done
        if finish_reason != FinishReason::ToolCalls || tool_calls.is_empty() {
            let _ = tx
                .send(AgentEvent::TurnComplete {
                    usage: cumulative_usage,
                })
                .await;
            return;
        }

        // Execute tool calls: prepare → emit ToolCallStart → await → emit ToolCallDone
        for tc in &tool_calls {
            // Prepare the tool call (parse args, render input) while holding
            // the lock, then drop the lock before awaiting.
            let prepared = {
                let s = state.lock();
                let tool = s.tools.iter().find(|t| t.definition().name == tc.name);
                match tool {
                    Some(t) => t.prepare(&tc.arguments),
                    None => Err(agnt_llm::Error::Other(format!("unknown tool: {}", tc.name))),
                }
                // lock drops here
            };

            match prepared {
                Ok(prepared) => {
                    let input_display = prepared.input_display.clone();
                    {
                        let mut s = state.lock();
                        set_tool_call_display_start(
                            &mut s.messages,
                            &tc.id,
                            to_tool_call_display_start_part(&input_display),
                        );
                    }

                    // Emit the input display immediately.
                    if tx
                        .send(AgentEvent::ToolCallStart {
                            id: tc.id.clone(),
                            display: input_display,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }

                    // Execute the tool.
                    match prepared.future.await {
                        Ok(result) => {
                            let output_display = result.output_display.clone();
                            {
                                let mut s = state.lock();
                                set_tool_call_display_result(
                                    &mut s.messages,
                                    &tc.id,
                                    to_tool_call_result_part(&output_display),
                                );
                            }

                            // Emit the output display.
                            if tx
                                .send(AgentEvent::ToolCallDone {
                                    id: tc.id.clone(),
                                    display: output_display,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }

                            // Add LLM-formatted result to conversation history.
                            {
                                let mut s = state.lock();
                                s.messages
                                    .push(Message::tool_result(&tc.id, &result.llm_output));
                            }
                        }
                        Err(e) => {
                            let error_text = format!("tool error: {e}");
                            let output_display = crate::event::ToolResultDisplay {
                                title: "error".to_string(),
                                body: Some(crate::event::DisplayBody::Text(error_text.clone())),
                            };
                            {
                                let mut s = state.lock();
                                set_tool_call_display_result(
                                    &mut s.messages,
                                    &tc.id,
                                    to_tool_call_result_part(&output_display),
                                );
                            }

                            if tx
                                .send(AgentEvent::ToolCallDone {
                                    id: tc.id.clone(),
                                    display: output_display,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }

                            // Errors also go into conversation history so the
                            // model can see what went wrong.
                            {
                                let mut s = state.lock();
                                s.messages.push(Message::tool_result(&tc.id, &error_text));
                            }
                        }
                    }
                }
                Err(e) => {
                    // Parsing / preparation failed.
                    let error_text = format!("tool error: {e}");
                    let output_display = crate::event::ToolResultDisplay {
                        title: "error".to_string(),
                        body: Some(crate::event::DisplayBody::Text(error_text.clone())),
                    };
                    {
                        let mut s = state.lock();
                        set_tool_call_display_result(
                            &mut s.messages,
                            &tc.id,
                            to_tool_call_result_part(&output_display),
                        );
                    }

                    if tx
                        .send(AgentEvent::ToolCallDone {
                            id: tc.id.clone(),
                            display: output_display,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }

                    {
                        let mut s = state.lock();
                        s.messages.push(Message::tool_result(&tc.id, &error_text));
                    }
                }
            }
        }

        // Loop back to generate again with tool results in context
    }
}

// ---------------------------------------------------------------------------
// Default system prompt
// ---------------------------------------------------------------------------

const SYSTEM_PROMPT_TEMPLATE: &str = include_str!("../resources/SYSTEM_PROMPT.md");

fn system_prompt(cwd: &Path, workspace_root: &Path) -> String {
    let mut handlebars = Handlebars::new();
    handlebars.set_strict_mode(true);

    let data = serde_json::json!({
        "cwd": cwd.display().to_string(),
        "workspace_root": workspace_root.display().to_string(),
    });

    handlebars
        .render_template(SYSTEM_PROMPT_TEMPLATE, &data)
        .unwrap_or_else(|_| SYSTEM_PROMPT_TEMPLATE.to_string())
}

fn find_workspace_root(cwd: &Path) -> PathBuf {
    let mut current = cwd.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        if !current.pop() {
            return cwd.to_path_buf();
        }
    }
}

fn load_agents_md(workspace_root: &Path) -> Option<String> {
    let path = workspace_root.join("AGENTS.md");
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some(content)
}

fn set_tool_call_display_start(
    messages: &mut [Message],
    tool_call_id: &str,
    display: agnt_llm::ToolCallDisplayPart,
) {
    for message in messages.iter_mut().rev() {
        if let Message::Assistant { parts } = message {
            for part in parts.iter_mut() {
                if let agnt_llm::AssistantPart::ToolCall(call) = part
                    && call.id == tool_call_id
                {
                    if let Some(existing) = call.display.as_mut() {
                        existing.title = display.title;
                        existing.description = display.description;
                    } else {
                        call.display = Some(display);
                    }
                    return;
                }
            }
        }
    }
}

fn set_tool_call_display_result(
    messages: &mut [Message],
    tool_call_id: &str,
    result: agnt_llm::ToolCallResultPart,
) {
    for message in messages.iter_mut().rev() {
        if let Message::Assistant { parts } = message {
            for part in parts.iter_mut() {
                if let agnt_llm::AssistantPart::ToolCall(call) = part
                    && call.id == tool_call_id
                {
                    if let Some(existing) = call.display.as_mut() {
                        existing.result = Some(result);
                    } else {
                        call.display = Some(agnt_llm::ToolCallDisplayPart {
                            title: call.name.clone(),
                            description: None,
                            result: Some(result),
                        });
                    }
                    return;
                }
            }
        }
    }
}

fn to_tool_call_display_start_part(
    display: &crate::event::ToolCallDisplay,
) -> agnt_llm::ToolCallDisplayPart {
    agnt_llm::ToolCallDisplayPart {
        title: display.title.clone(),
        description: display.body.as_ref().map(to_tool_display_body_part),
        result: None,
    }
}

fn to_tool_call_result_part(
    display: &crate::event::ToolResultDisplay,
) -> agnt_llm::ToolCallResultPart {
    agnt_llm::ToolCallResultPart {
        title: display.title.clone(),
        body: display.body.as_ref().map(to_tool_display_body_part),
    }
}

fn to_tool_display_body_part(body: &crate::event::DisplayBody) -> agnt_llm::ToolDisplayBodyPart {
    match body {
        crate::event::DisplayBody::Text(text) => agnt_llm::ToolDisplayBodyPart::Text(text.clone()),
        crate::event::DisplayBody::Code { language, content } => {
            agnt_llm::ToolDisplayBodyPart::Code {
                language: language.clone(),
                content: content.clone(),
            }
        }
    }
}
