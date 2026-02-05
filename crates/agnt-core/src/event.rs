use agnt_llm::stream::Usage;

/// Events emitted by the agent during a generation turn.
///
/// A frontend consumes these to update its UI. The events form a protocol:
///
/// ```text
/// UserMessage
/// (TextDelta)* AssistantMessage
/// (ToolCallBegin ToolCallDelta* ToolCallReady ToolResult)* ← tool loop
/// (TextDelta)* AssistantMessage                            ← final answer
/// TurnComplete
/// ```
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// The user's message was recorded in conversation history.
    UserMessage { content: String },

    /// A chunk of assistant text arrived.
    TextDelta { delta: String },

    /// The assistant's complete text for this generation step.
    AssistantMessage { content: String },

    /// A tool call started streaming.
    ToolCallBegin { id: String, name: String },

    /// A chunk of tool call arguments (JSON fragment).
    ToolCallDelta { id: String, delta: String },

    /// Tool call arguments are fully assembled; tool is about to execute.
    ToolCallReady {
        id: String,
        name: String,
        arguments: String,
    },

    /// A tool has finished executing.
    ToolResult {
        id: String,
        name: String,
        result: String,
    },

    /// The entire turn is complete (no more tool loops).
    TurnComplete { usage: Usage },

    /// An error occurred during the turn.
    Error { error: String },
}
