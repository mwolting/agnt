//! Provider factory trait and configuration options.

use agnt_llm::LanguageModelProvider;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::ResolvedAuth;
use crate::error::Error;

/// Options passed to a [`ProviderFactory`] when constructing a provider.
#[derive(Debug, Clone)]
pub struct ProviderOptions {
    /// The provider identifier (e.g. `"openai"`).
    pub id: String,
    /// Base API endpoint. `None` means provider default.
    pub api_endpoint: Option<String>,
    /// Provider-specific factory options from provider registration.
    pub(crate) factory_options: Option<Value>,
    /// Resolved auth payload for this provider.
    pub auth: ResolvedAuth,
}

impl ProviderOptions {
    /// Deserialize provider-specific factory options into a typed payload.
    pub fn factory_options_as<T>(&self) -> Result<Option<T>, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        match &self.factory_options {
            Some(value) => serde_json::from_value(value.clone()).map(Some),
            None => Ok(None),
        }
    }
}

/// A factory that can construct a [`LanguageModelProvider`] from
/// [`ProviderOptions`].
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
