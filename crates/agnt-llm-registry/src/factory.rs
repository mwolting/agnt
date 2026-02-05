//! Provider factory trait and configuration options.

use agnt_llm::LanguageModelProvider;

use crate::error::Error;

/// Options passed to a [`ProviderFactory`] when constructing a provider.
///
/// These are derived from the models.dev spec and the process environment.
#[derive(Debug, Clone)]
pub struct ProviderOptions {
    /// The provider identifier (e.g. `"openai"`).
    pub id: String,

    /// API key resolved from the environment (first matching env var found).
    /// `None` if no env var was specified or none were set.
    pub api_key: Option<String>,

    /// Base API endpoint. Comes from the models.dev `"api"` field when present.
    /// `None` means the provider should use its built-in default.
    pub api_endpoint: Option<String>,
}

/// A factory that can construct a [`LanguageModelProvider`] from
/// [`ProviderOptions`].
///
/// Implement this trait for concrete provider integrations, or use closures via
/// [`Registry::add_provider`].
pub trait ProviderFactory: Send + Sync {
    /// Create a provider instance from the given options.
    fn create(&self, options: ProviderOptions) -> Result<LanguageModelProvider, Error>;
}

/// Blanket impl: any `Fn(ProviderOptions) -> Result<LanguageModelProvider, Error>`
/// is a factory.
impl<F> ProviderFactory for F
where
    F: Fn(ProviderOptions) -> Result<LanguageModelProvider, Error> + Send + Sync,
{
    fn create(&self, options: ProviderOptions) -> Result<LanguageModelProvider, Error> {
        (self)(options)
    }
}
