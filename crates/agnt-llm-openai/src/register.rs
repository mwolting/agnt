//! Registry integration for the OpenAI provider.

use agnt_llm_registry::{ProviderOptions, Registry};

use crate::{provider, OpenAIConfig};

/// The npm packages this crate can serve.
const COMPATIBLE_PACKAGES: &[&str] = &["@ai-sdk/openai"];

/// Register this provider with the given [`Registry`] for all compatible npm
/// packages (`@ai-sdk/openai`).
///
/// After calling this, any model in the models.dev spec whose effective npm
/// package is `@ai-sdk/openai` will be routed through this crate.
///
/// ```ignore
/// let mut registry = Registry::new();
/// agnt_llm_openai::register(&mut registry);
/// registry.fetch_spec().await?;
/// let model = registry.model("opencode", "gpt-5.2-codex")?;
/// ```
pub fn register(registry: &mut Registry) {
    for &npm in COMPATIBLE_PACKAGES {
        registry.add_factory(npm, factory);
    }
}

fn factory(
    options: ProviderOptions,
) -> Result<agnt_llm::LanguageModelProvider, agnt_llm_registry::Error> {
    Ok(provider(OpenAIConfig {
        api_key: options.api_key.unwrap_or_default(),
        base_url: options
            .api_endpoint
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
    }))
}
