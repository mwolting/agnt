use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agnt_llm::stream::{FinishReason, StreamEvent, Usage};
use agnt_llm::{LanguageModel, Message, ToolDefinition};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::event::AgentEvent;
use crate::tool::{ErasedTool, Tool};
use crate::tools::{BashTool, EditTool, ReadTool, WriteTool};

// ---------------------------------------------------------------------------
// Agent state (shared between handle and spawned task)
// ---------------------------------------------------------------------------

struct AgentState {
    messages: Vec<Message>,
    tools: Vec<Box<dyn ErasedTool>>,
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

/// The core agent. Holds a language model, conversation history, and
/// registered tools. UI-agnostic — communicates via [`AgentEvent`]s.
pub struct Agent {
    model: Arc<LanguageModel>,
    system_prompt: Option<String>,
    state: Arc<Mutex<AgentState>>,
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
            })),
        }
    }

    /// Create an agent with the default coding tools (read, write, edit, bash)
    /// and a system prompt that turns it into a coding assistant.
    ///
    /// `cwd` is the working directory that file and bash tools operate in.
    pub fn with_defaults(model: LanguageModel, cwd: PathBuf) -> Self {
        let mut agent = Self::new(model);
        agent.system(system_prompt(&cwd));

        agent.tool(ReadTool { cwd: cwd.clone() });
        agent.tool(WriteTool { cwd: cwd.clone() });
        agent.tool(EditTool { cwd: cwd.clone() });
        agent.tool(BashTool { cwd });

        agent
    }

    /// Set the system prompt.
    pub fn system(&mut self, prompt: impl Into<String>) -> &mut Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Register a tool the model can call.
    pub fn tool(&mut self, tool: impl Tool) -> &mut Self {
        self.state.lock().unwrap().tools.push(Box::new(tool));
        self
    }

    /// Access the conversation history (completed messages only).
    pub fn messages(&self) -> Vec<Message> {
        self.state.lock().unwrap().messages.clone()
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

        tokio::spawn(async move {
            generation_loop(model, state, system_prompt, content, tx).await;
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
    content: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    // 1. Record user message
    {
        let mut s = state.lock().unwrap();
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
            let s = state.lock().unwrap();
            let mut req = agnt_llm::request();
            if let Some(ref system) = system_prompt {
                req.system(system.as_str());
            }
            req.messages(s.messages.clone());

            let tool_defs: Vec<ToolDefinition> = s.tools.iter().map(|t| t.definition()).collect();
            req.tools(tool_defs);

            req.build()
        };

        // Stream the response. We collect AssistantParts in arrival order
        // so interleaved reasoning/text/tool-calls are preserved exactly.
        let mut stream = model.generate(request).events();
        let mut parts: Vec<agnt_llm::AssistantPart> = Vec::new();
        let mut text = String::new();
        let mut tool_calls: Vec<agnt_llm::ToolCallPart> = Vec::new();
        let mut finish_reason = FinishReason::Stop;

        // Helper: flush accumulated text deltas into a Text part.
        macro_rules! flush_text {
            ($parts:expr, $text:expr) => {
                if !$text.is_empty() {
                    $parts.push(agnt_llm::AssistantPart::Text(agnt_llm::TextPart {
                        text: std::mem::take(&mut $text),
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
                Ok(StreamEvent::ReasoningDelta(_)) => {
                    // Could forward to UI. For now, silently wait for
                    // ReasoningDone which carries the full part.
                }
                Ok(StreamEvent::ReasoningDone(part)) => {
                    flush_text!(parts, text);
                    parts.push(agnt_llm::AssistantPart::Reasoning(part));
                }
                Ok(StreamEvent::ToolCallBegin { id, name, .. }) => {
                    if tx
                        .send(AgentEvent::ToolCallBegin {
                            id: id.clone(),
                            name: name.clone(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamEvent::ToolCallDelta {
                    arguments_delta, ..
                }) => {
                    // We don't track index→id mapping here; the full call
                    // comes in ToolCallEnd. Just forward the delta.
                    if tx
                        .send(AgentEvent::ToolCallDelta {
                            id: String::new(),
                            delta: arguments_delta,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(StreamEvent::ToolCallEnd { call, .. }) => {
                    if tx
                        .send(AgentEvent::ToolCallReady {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
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

        // Notify UI of completed assistant text (concatenate all Text parts)
        let full_text: String = parts
            .iter()
            .filter_map(|p| match p {
                agnt_llm::AssistantPart::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        if !full_text.is_empty()
            && tx
                .send(AgentEvent::AssistantMessage {
                    content: full_text,
                })
                .await
                .is_err()
        {
            return;
        }

        // Record the assistant message with parts in arrival order
        {
            let mut s = state.lock().unwrap();
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

        // Execute tool calls and add results to history
        for tc in &tool_calls {
            // Get a 'static future while holding the lock, then drop the
            // lock and await.
            let fut = {
                let s = state.lock().unwrap();
                let tool = s.tools.iter().find(|t| t.definition().name == tc.name);
                match tool {
                    Some(t) => t.call_erased(&tc.arguments),
                    None => Box::pin(async move {
                        Err(agnt_llm::Error::Other(format!(
                            "unknown tool: {}",
                            tc.name
                        )))
                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, agnt_llm::Error>> + Send>>,
                }
                // lock drops here
            };

            let result = fut.await;

            let result_text = match result {
                Ok(text) => text,
                Err(e) => format!("tool error: {e}"),
            };

            if tx
                .send(AgentEvent::ToolResult {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    result: result_text.clone(),
                })
                .await
                .is_err()
            {
                return;
            }

            // Add tool result to conversation
            {
                let mut s = state.lock().unwrap();
                s.messages
                    .push(Message::tool_result(&tc.id, &result_text));
            }
        }

        // Loop back to generate again with tool results in context
    }
}

// ---------------------------------------------------------------------------
// Default system prompt
// ---------------------------------------------------------------------------

fn system_prompt(cwd: &std::path::Path) -> String {
    format!(
        r#"You are an expert coding assistant. You help the user by reading, writing, editing, and running code in their project.

Working directory: {cwd}

You have four tools:

- **read**: Read a file. Give a path relative to the working directory.
- **write**: Write (or overwrite) a file. Give a relative path and the full content. Parent directories are created automatically.
- **edit**: Replace text in a file. Give a relative path, the exact `old` text to find, and the `new` replacement. The `old` text must appear exactly once.
- **bash**: Run a shell command. The command runs in the working directory. Returns stdout, stderr, and exit code.

Guidelines:
- Before editing a file, read it first so you have the exact content to match against.
- Use edit for surgical changes; use write only when creating new files or replacing the entire content.
- When running commands, prefer non-interactive invocations.
- Be concise in your explanations. Focus on what changed and why.
- If a command fails, read the error and try to fix it."#,
        cwd = cwd.display()
    )
}
