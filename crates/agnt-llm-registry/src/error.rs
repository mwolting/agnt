/// Errors produced by the LLM registry.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A provider with the given name was not found in the registry.
    #[error("provider not found: {0}")]
    ProviderNotFound(String),

    /// A model with the given ID was not found for the specified provider.
    #[error("model not found: {provider}:{model}")]
    ModelNotFound { provider: String, model: String },

    /// Failed to fetch the models.dev spec.
    #[error("failed to fetch models.dev spec: {0}")]
    Fetch(Box<dyn std::error::Error + Send + Sync>),

    /// Failed to parse the models.dev spec.
    #[error("failed to parse models.dev spec: {0}")]
    Parse(#[from] serde_json::Error),

    /// The provider factory returned an error during construction.
    #[error("provider factory error: {0}")]
    Factory(Box<dyn std::error::Error + Send + Sync>),

    /// Required environment variable is not set.
    #[error("missing environment variable: {0}")]
    MissingEnvVar(String),
}
