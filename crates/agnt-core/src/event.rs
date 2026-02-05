use agnt_llm::stream::Usage;

// ---------------------------------------------------------------------------
// Display types — tool-agnostic rendering protocol
// ---------------------------------------------------------------------------

/// How to display a tool invocation (the input side) to the user.
#[derive(Debug, Clone)]
pub struct ToolCallDisplay {
    /// Short summary, e.g. "Read src/main.rs", "Run `cargo build`".
    pub title: String,
    /// Optional expanded content (e.g. the command, the file content to write).
    pub body: Option<DisplayBody>,
}

/// How to display a tool result (the output side) to the user.
#[derive(Debug, Clone)]
pub struct ToolResultDisplay {
    /// Short summary, e.g. "55 lines", "exit code 0".
    pub title: String,
    /// Optional expanded content (e.g. file contents, command output).
    pub body: Option<DisplayBody>,
}

/// Structured content for display. Frontends can use this to apply
/// syntax highlighting, diff rendering, etc.
#[derive(Debug, Clone)]
pub enum DisplayBody {
    /// Plain text.
    Text(String),
    /// Code with an optional language hint for syntax highlighting.
    Code {
        language: Option<String>,
        content: String,
    },
}

// ---------------------------------------------------------------------------
// Agent events — the render-oriented protocol from agent to UI
// ---------------------------------------------------------------------------

/// Events emitted by the agent during a generation turn.
///
/// A frontend consumes these to update its UI. The events form a protocol:
///
/// ```text
/// UserMessage
/// (TextDelta)*
/// (ToolCallStart ToolCallDone)* ← tool loop
/// (TextDelta)*                  ← final answer after tools
/// TurnComplete
/// ```
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// The user's message was recorded in conversation history.
    UserMessage { content: String },

    /// A chunk of assistant text arrived.
    TextDelta { delta: String },

    /// A tool call has been fully parsed and is about to execute.
    /// Contains a rendered display of the tool's input.
    ToolCallStart {
        id: String,
        display: ToolCallDisplay,
    },

    /// A tool has finished executing. Contains a rendered display of the result.
    ToolCallDone {
        id: String,
        display: ToolResultDisplay,
    },

    /// The entire turn is complete (no more tool loops).
    TurnComplete { usage: Usage },

    /// An error occurred during the turn.
    Error { error: String },
}
