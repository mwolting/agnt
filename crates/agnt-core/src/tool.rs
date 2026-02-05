use agnt_llm::{Describe, ToolDefinition};
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;

/// A callable tool with typed input. Implement this trait to register tools
/// with the agent.
///
/// The `Input` type must implement [`Describe`] (for schema generation) and
/// [`DeserializeOwned`] (for parsing the model's JSON arguments).
///
/// Tools must be `Clone` so the erasure layer can clone them before calling
/// `async fn call` — this avoids the borrow-across-await problem without
/// requiring manual `Box::pin`.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone)]
/// struct ReadFile;
///
/// impl Tool for ReadFile {
///     type Input = ReadFileInput;
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
    type Input: Describe + DeserializeOwned + Send;

    fn name(&self) -> &str;
    fn description(&self) -> &str;

    fn call(
        &self,
        input: Self::Input,
    ) -> impl Future<Output = Result<String, agnt_llm::Error>> + Send;
}

// ---------------------------------------------------------------------------
// Type erasure
// ---------------------------------------------------------------------------

/// Object-safe, type-erased wrapper around a [`Tool`].
///
/// The returned future from `call_erased` is `'static` — it does not borrow
/// `self`, which allows callers to drop locks before awaiting.
pub(crate) trait ErasedTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    fn call_erased(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, agnt_llm::Error>> + Send>>;
}

impl<T: Tool> ErasedTool for T {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: T::Input::describe(),
        }
    }

    fn call_erased(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, agnt_llm::Error>> + Send>> {
        let parsed: Result<T::Input, _> = serde_json::from_str(arguments);
        // Clone self so the future is 'static and doesn't borrow from the
        // tool registry. This is why Tool requires Clone.
        let this = self.clone();
        Box::pin(async move {
            let input = parsed.map_err(|e| agnt_llm::Error::Other(e.to_string()))?;
            this.call(input).await
        })
    }
}
