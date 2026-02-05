use crate::model::LanguageModel;

/// A concrete, type-erased language model provider.
///
/// Wraps a [`LanguageModelProviderBackend`] behind a `Box<dyn ...>` so that
/// callers never need generic parameters — you can swap providers freely.
pub struct LanguageModelProvider {
    inner: Box<dyn LanguageModelProviderBackend>,
}

impl LanguageModelProvider {
    /// Wrap any backend implementation into a provider.
    pub fn new(backend: impl LanguageModelProviderBackend + 'static) -> Self {
        Self {
            inner: Box::new(backend),
        }
    }

    /// The provider name (e.g. `"openai"`, `"anthropic"`).
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Create a model handle for the given model ID.
    pub fn model(&self, model_id: &str) -> LanguageModel {
        self.inner.model(model_id)
    }
}

/// Trait that provider crates implement.
pub trait LanguageModelProviderBackend: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self, model_id: &str) -> LanguageModel;
}
