use crate::request::GenerateRequest;
use crate::response::Response;

/// A concrete, type-erased language model handle.
///
/// Wraps a [`LanguageModelBackend`] so callers never need generics.
pub struct LanguageModel {
    inner: Box<dyn LanguageModelBackend>,
}

impl LanguageModel {
    /// Wrap any backend implementation into a model.
    pub fn new(backend: impl LanguageModelBackend + 'static) -> Self {
        Self {
            inner: Box::new(backend),
        }
    }

    /// The model identifier (e.g. `"gpt-5"`, `"claude-opus-4-6"`).
    pub fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    /// The provider name this model belongs to.
    pub fn provider(&self) -> &str {
        self.inner.provider()
    }

    /// Generate a streaming response.
    pub fn generate(&self, request: impl Into<GenerateRequest>) -> Response {
        self.inner.generate(request.into())
    }
}

/// Trait that provider crates implement for a specific model.
pub trait LanguageModelBackend: Send + Sync {
    fn model_id(&self) -> &str;
    fn provider(&self) -> &str;
    fn generate(&self, request: GenerateRequest) -> Response;
}
