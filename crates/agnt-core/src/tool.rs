use agnt_llm::{Describe, ToolDefinition};
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;

use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};

// ---------------------------------------------------------------------------
// ToolOutput — typed return values that know how to serialize for the LLM
// ---------------------------------------------------------------------------

/// A tool's return value. Knows how to serialize itself into the text that
/// gets sent back to the LLM as the tool result in conversation history.
///
/// Implement this for structured tool outputs (e.g. `ReadOutput` with
/// separate `path` and `content` fields). For tools that just return a
/// plain string, the blanket `impl ToolOutput for String` handles it.
pub trait ToolOutput: Send {
    /// Serialize this result into the text the LLM will see.
    fn to_llm(&self) -> String;
}

impl ToolOutput for String {
    fn to_llm(&self) -> String {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// Tool trait
// ---------------------------------------------------------------------------

/// A callable tool with typed input and output.
///
/// The `Input` type must implement [`Describe`] (for schema generation),
/// [`DeserializeOwned`] (for parsing the model's JSON arguments), and
/// [`Clone`] (so the framework can pass `&Input` to render methods after
/// `call()` consumes the value).
///
/// The `Output` type must implement [`ToolOutput`] so the framework knows how
/// to serialize the result for the LLM. Use `String` as the output type for
/// simple tools.
///
/// Tools must be `Clone` so the erasure layer can clone them before calling
/// `async fn call` — this avoids the borrow-across-await problem without
/// requiring manual `Box::pin`.
///
/// ## Rendering
///
/// Tools control how they appear in the UI via three render methods, all
/// with sensible defaults:
///
/// - [`render_input`](Tool::render_input) — how the invocation looks to the user
/// - [`render_output`](Tool::render_output) — how the result looks to the user
/// - [`render_llm_output`](Tool::render_llm_output) — what text goes into conversation history
///
/// # Example
///
/// ```ignore
/// #[derive(Clone)]
/// struct ReadFile { cwd: PathBuf }
///
/// impl Tool for ReadFile {
///     type Input = ReadFileInput;
///     type Output = String; // simple case: just use String
///
///     fn name(&self) -> &str { "read_file" }
///     fn description(&self) -> &str { "Read a file from disk" }
///
///     async fn call(&self, input: ReadFileInput) -> Result<String, agnt_llm::Error> {
///         let content = std::fs::read_to_string(&input.path)
///             .map_err(|e| agnt_llm::Error::Other(e.to_string()))?;
///         Ok(content)
///     }
/// }
/// ```
pub trait Tool: Clone + Send + Sync + 'static {
    type Input: Describe + DeserializeOwned + Clone + Send;
    type Output: ToolOutput + Send;

    fn name(&self) -> &str;
    fn description(&self) -> &str;

    fn call(
        &self,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, agnt_llm::Error>> + Send;

    /// How to display the tool invocation to the user.
    ///
    /// Override to show e.g. "Read src/main.rs" instead of the raw tool name.
    /// Default: tool name as the title, no body.
    fn render_input(&self, _input: &Self::Input) -> ToolCallDisplay {
        ToolCallDisplay {
            title: self.name().to_string(),
            body: None,
        }
    }

    /// How to display the tool result to the user.
    ///
    /// Override to show e.g. syntax-highlighted file contents, coloured
    /// exit codes, diff output.
    /// Default: tool name as title, raw LLM text as plain text body.
    fn render_output(&self, input: &Self::Input, output: &Self::Output) -> ToolResultDisplay {
        ToolResultDisplay {
            title: self.name().to_string(),
            body: Some(DisplayBody::Text(self.render_llm_output(input, output))),
        }
    }

    /// What text goes into conversation history as the tool result.
    ///
    /// Override per-tool to e.g. add context, truncate, or reformat the
    /// output before the LLM sees it.
    /// Default: delegates to [`ToolOutput::to_llm()`].
    fn render_llm_output(&self, _input: &Self::Input, output: &Self::Output) -> String {
        output.to_llm()
    }
}

// ---------------------------------------------------------------------------
// Type erasure
// ---------------------------------------------------------------------------

/// The result of executing a prepared tool call.
pub(crate) struct ToolExecResult {
    /// The text that goes into conversation history for the LLM.
    pub llm_output: String,
    /// How the result should be displayed to the user.
    pub output_display: ToolResultDisplay,
}

/// A parsed, ready-to-execute tool call. Holds the input display (which the
/// agent can emit immediately) and a future that does the actual work.
pub(crate) struct PreparedToolCall {
    /// How the invocation should be displayed to the user — available
    /// immediately, before execution.
    pub input_display: ToolCallDisplay,
    /// The future that executes the tool and produces the result.
    pub future: Pin<Box<dyn Future<Output = Result<ToolExecResult, agnt_llm::Error>> + Send>>,
}

/// Object-safe, type-erased wrapper around a [`Tool`].
///
/// The two-phase interface (`prepare` then await) lets the agent emit
/// a `ToolCallStart` event with the input display *before* executing.
pub(crate) trait ErasedTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    /// Parse arguments and produce a [`PreparedToolCall`].
    ///
    /// This is synchronous — it parses JSON and calls `render_input`, but
    /// does **not** execute the tool. The caller can inspect `input_display`
    /// immediately, then `.await` the `future` when ready.
    fn prepare(&self, arguments: &str) -> Result<PreparedToolCall, agnt_llm::Error>;
}

impl<T: Tool> ErasedTool for T {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: T::Input::describe(),
        }
    }

    fn prepare(&self, arguments: &str) -> Result<PreparedToolCall, agnt_llm::Error> {
        let input: T::Input =
            serde_json::from_str(arguments).map_err(|e| agnt_llm::Error::Other(e.to_string()))?;

        let input_display = self.render_input(&input);

        // Clone self + input so the future is 'static.
        let this = self.clone();
        let input_for_call = input.clone();
        let future = Box::pin(async move {
            let output = this.call(input_for_call.clone()).await?;
            let llm_output = this.render_llm_output(&input_for_call, &output);
            let output_display = this.render_output(&input_for_call, &output);
            Ok(ToolExecResult {
                llm_output,
                output_display,
            })
        });

        Ok(PreparedToolCall {
            input_display,
            future,
        })
    }
}
