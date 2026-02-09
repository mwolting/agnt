//! Registry integration for the OpenAI provider.

use std::collections::HashMap;

use agnt_llm_registry::{
    ApiKeyAuth, AuthMethod, ModelSource, ModelSpec, OAuthPkceAuth, ProviderOptions,
    ProviderRegistration, Registry,
};
use serde::{Deserialize, Serialize};

use crate::{OpenAIConfig, provider};

/// The npm packages this crate can serve.
const COMPATIBLE_PACKAGES: &[&str] = &["@ai-sdk/openai"];

/// Register this provider with the given [`Registry`] for all compatible npm
/// packages (`@ai-sdk/openai`).
///
/// After calling this, any model in the models.dev spec whose effective npm
/// package is `@ai-sdk/openai` will be routed through this crate.
pub fn register(registry: &mut Registry) {
    for &npm in COMPATIBLE_PACKAGES {
        registry.add_factory(npm, factory);
    }

    let mut registration = ProviderRegistration::new("openai", "OpenAI");
    registration.npm_packages = COMPATIBLE_PACKAGES.iter().map(|s| s.to_string()).collect();
    registration.api_endpoint = Some("https://api.openai.com/v1".to_string());
    registration.auth_method = AuthMethod::ApiKey(ApiKeyAuth {
        env: vec!["OPENAI_API_KEY".to_string()],
    });
    registration
        .set_factory_options(&OpenAIProviderBehavior::default())
        .expect("OpenAI provider behavior should serialize");
    registration.model_source = ModelSource::ModelsDev;
    registry.add_registration(registration);
}

/// Register an OAuth-based OpenAI provider with static model metadata.
pub fn register_oauth_provider(
    registry: &mut Registry,
    provider_id: impl Into<String>,
    provider_name: impl Into<String>,
    oauth: OAuthPkceAuth,
    models: Vec<ModelSpec>,
    api_endpoint: Option<String>,
) {
    register_oauth_provider_with_behavior(
        registry,
        provider_id,
        provider_name,
        oauth,
        models,
        api_endpoint,
        OpenAIProviderBehavior::default(),
    );
}

/// Like [`register_oauth_provider`] but with explicit per-provider transport
/// behavior (e.g. `store=false`, custom headers, reasoning include fields).
pub fn register_oauth_provider_with_behavior(
    registry: &mut Registry,
    provider_id: impl Into<String>,
    provider_name: impl Into<String>,
    oauth: OAuthPkceAuth,
    models: Vec<ModelSpec>,
    api_endpoint: Option<String>,
    behavior: OpenAIProviderBehavior,
) {
    for &npm in COMPATIBLE_PACKAGES {
        registry.add_factory(npm, factory);
    }

    let mut registration = ProviderRegistration::new(provider_id, provider_name);
    registration.npm_packages = COMPATIBLE_PACKAGES.iter().map(|s| s.to_string()).collect();
    registration.api_endpoint = api_endpoint;
    registration.auth_method = AuthMethod::OAuthPkce(oauth);
    registration
        .set_factory_options(&behavior)
        .expect("OpenAI provider behavior should serialize");
    registration.model_source = ModelSource::Static(models);
    registry.add_registration(registration);
}

fn factory(
    options: ProviderOptions,
) -> Result<agnt_llm::LanguageModelProvider, agnt_llm_registry::Error> {
    let behavior = options
        .factory_options_as::<OpenAIProviderBehavior>()
        .map_err(|err| agnt_llm_registry::Error::Factory(Box::new(err)))?
        .unwrap_or_default();
    let auth_token = options
        .auth
        .get("access_token")
        .or_else(|| options.auth.get("api_key"))
        .unwrap_or_default()
        .to_string();

    Ok(provider(OpenAIConfig {
        auth_token,
        base_url: options
            .api_endpoint
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
        response_store: behavior.response_store,
        include_reasoning_encrypted_content: behavior.include_reasoning_encrypted_content,
        extra_headers: behavior.extra_headers,
        include_chatgpt_account_id_header: behavior.include_chatgpt_account_id_header,
    }))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAIProviderBehavior {
    pub response_store: Option<bool>,
    pub include_reasoning_encrypted_content: bool,
    pub include_chatgpt_account_id_header: bool,
    pub extra_headers: HashMap<String, String>,
}
