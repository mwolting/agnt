//! # agnt-llm-registry
//!
//! A provider registry for language models, backed by the
//! [models.dev](https://models.dev) specification.
//!
//! This crate lets you:
//!
//! - **Register named LLM providers** with factory closures that construct
//!   [`agnt_llm::LanguageModelProvider`] instances on demand.
//! - **List models** and their capabilities using the models.dev spec.
//! - **Dynamically register providers** by convention — any provider in the
//!   models.dev spec can be wired up with minimal code.
//!
//! # Quick start
//!
//! ```ignore
//! use agnt_llm_registry::{Registry, ProviderOptions};
//!
//! let mut registry = Registry::new();
//!
//! // Register a provider with a factory closure.
//! // The closure receives ProviderOptions with resolved auth and api_endpoint.
//! registry.add_provider("openai", |options: ProviderOptions| {
//!     Ok(agnt_llm_openai::provider(OpenAIConfig {
//!         auth_token: options
//!             .auth
//!             .get("access_token")
//!             .or_else(|| options.auth.get("api_key"))
//!             .unwrap_or_default()
//!             .to_string(),
//!         base_url: options.api_endpoint
//!             .unwrap_or_else(|| "https://api.openai.com/v1".into()),
//!     }))
//! });
//!
//! // Optionally load the models.dev spec for model metadata
//! registry.fetch_spec().await?;
//!
//! // List available models for a provider
//! let models = registry.list_models("openai");
//!
//! // Get a model handle
//! let model = registry.model("openai", "gpt-4.1-nano")?;
//! // or: let model = registry.model_from_string("openai:gpt-4.1-nano")?;
//! ```

pub mod auth;
pub mod error;
pub mod factory;
pub mod model_source;
pub mod provider;
pub mod registry;
pub mod spec;

pub use auth::{ApiKeyAuth, AuthMethod, AuthRequest, AuthResolver, OAuthPkceAuth, ResolvedAuth};
pub use error::Error;
pub use factory::{ProviderFactory, ProviderOptions};
pub use model_source::{ModelLoader, ModelSource};
pub use provider::ProviderRegistration;
pub use registry::{AvailableProvider, KnownProvider, Registry};
pub use spec::{
    Modalities, ModelCost, ModelLimit, ModelProviderOverride, ModelSpec, ModelsDevSpec,
    ProviderSpec,
};
