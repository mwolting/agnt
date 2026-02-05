//! The core registry: maps provider names to factories and resolves models.

use std::collections::HashMap;

use agnt_llm::{LanguageModel, LanguageModelProvider};

use crate::error::Error;
use crate::factory::{ProviderFactory, ProviderOptions};
use crate::spec::{ModelSpec, ModelsDevSpec, ProviderSpec};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// A provider that is available for use (has credentials and a compatible
/// factory).
#[derive(Debug, Clone)]
pub struct AvailableProvider {
    /// Provider identifier (e.g. `"opencode"`).
    pub id: String,
    /// Human-friendly display name (e.g. `"OpenCode Zen"`).
    pub name: String,
}

/// An entry keyed by npm package name.
struct NpmEntry {
    factory: Box<dyn ProviderFactory>,
    /// Lazily constructed providers keyed by the provider ID that requested
    /// them. A single npm factory can serve multiple provider IDs with
    /// distinct credentials/endpoints (e.g. `@ai-sdk/openai` serves both
    /// `openai` and the openai-routed models inside `opencode`).
    instances: HashMap<String, LanguageModelProvider>,
}

/// Central registry that maps **npm package names** to provider factories and
/// uses the [models.dev](https://models.dev) specification to resolve which
/// factory to use for each model.
///
/// A model's effective npm package is determined by:
/// 1. The model-level `provider.npm` override (if present), or
/// 2. The parent provider's top-level `npm` field.
///
/// # Example
///
/// ```ignore
/// use agnt_llm_registry::Registry;
///
/// let mut registry = Registry::new();
///
/// // Register factories by npm package name
/// registry.add_factory("@ai-sdk/openai", |options| {
///     Ok(agnt_llm_openai::provider(OpenAIConfig {
///         api_key: options.api_key.unwrap_or_default(),
///         base_url: options.api_endpoint
///             .unwrap_or_else(|| "https://api.openai.com/v1".into()),
///     }))
/// });
///
/// // Load spec so we know which npm package each model needs
/// registry.fetch_spec().await?;
///
/// // "opencode:gpt-5.2-codex" has provider.npm = "@ai-sdk/openai",
/// // so it routes through the openai factory automatically.
/// let model = registry.model("opencode", "gpt-5.2-codex")?;
/// ```
pub struct Registry {
    /// Factories keyed by npm package name (e.g. `"@ai-sdk/openai"`).
    factories: HashMap<String, NpmEntry>,
    /// Simple named providers registered directly (no npm routing).
    providers: HashMap<String, ProviderEntry>,
    spec: Option<ModelsDevSpec>,
}

/// A directly-registered provider (non-npm path).
struct ProviderEntry {
    factory: Box<dyn ProviderFactory>,
    instance: Option<LanguageModelProvider>,
}

impl Registry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            providers: HashMap::new(),
            spec: None,
        }
    }

    // -----------------------------------------------------------------------
    // Factory registration (npm-based)
    // -----------------------------------------------------------------------

    /// Register a factory for a given npm package name.
    ///
    /// When a model is requested, the registry resolves the effective npm
    /// package (model-level `provider.npm` override, or the parent provider's
    /// `npm` field) and uses the corresponding factory.
    pub fn add_factory(
        &mut self,
        npm: impl Into<String>,
        factory: impl ProviderFactory + 'static,
    ) {
        self.factories.insert(
            npm.into(),
            NpmEntry {
                factory: Box::new(factory),
                instances: HashMap::new(),
            },
        );
    }

    // -----------------------------------------------------------------------
    // Direct provider registration (backward compat)
    // -----------------------------------------------------------------------

    /// Register a named provider directly (bypasses npm routing).
    pub fn add_provider(
        &mut self,
        name: impl Into<String>,
        factory: impl ProviderFactory + 'static,
    ) {
        self.providers.insert(
            name.into(),
            ProviderEntry {
                factory: Box::new(factory),
                instance: None,
            },
        );
    }

    /// Check whether a provider or factory can serve the given name.
    pub fn has_provider(&self, name: &str) -> bool {
        if self.providers.contains_key(name) {
            return true;
        }
        if let Some(spec) = &self.spec
            && let Some(ps) = spec.get(name)
            && let Some(npm) = &ps.npm
        {
            return self.factories.contains_key(npm.as_str());
        }
        false
    }

    // -----------------------------------------------------------------------
    // Model resolution
    // -----------------------------------------------------------------------

    /// Obtain a [`LanguageModel`] for the given provider and model ID.
    ///
    /// Resolution order:
    /// 1. Look up the model in the spec to find its effective npm package
    ///    (model-level `provider.npm` or parent provider `npm`).
    /// 2. Use the factory registered for that npm package.
    /// 3. Fall back to a directly-registered provider if no npm match.
    pub fn model(&mut self, provider: &str, model_id: &str) -> Result<LanguageModel, Error> {
        // Try npm-routed resolution first.
        if let Some(result) = self.model_via_npm(provider, model_id)? {
            return Ok(result);
        }

        // Fall back to direct provider.
        self.model_via_direct(provider, model_id)
    }

    /// Parse a combined `"provider:model"` string and return the model.
    pub fn model_from_string(&mut self, specifier: &str) -> Result<LanguageModel, Error> {
        let (provider, model_id) = specifier.split_once(':').ok_or_else(|| {
            Error::ProviderNotFound(format!(
                "invalid model specifier '{specifier}', expected 'provider:model'"
            ))
        })?;
        self.model(provider, model_id)
    }

    // -----------------------------------------------------------------------
    // Spec management
    // -----------------------------------------------------------------------

    /// Load the models.dev spec from the remote URL.
    pub async fn fetch_spec(&mut self) -> Result<(), Error> {
        let body = reqwest::get(MODELS_DEV_URL)
            .await
            .map_err(|e| Error::Fetch(Box::new(e)))?
            .text()
            .await
            .map_err(|e| Error::Fetch(Box::new(e)))?;

        let parsed: ModelsDevSpec = serde_json::from_str(&body)?;
        self.spec = Some(parsed);
        Ok(())
    }

    /// Load the models.dev spec from a JSON string.
    pub fn load_spec_from_str(&mut self, json: &str) -> Result<(), Error> {
        let parsed: ModelsDevSpec = serde_json::from_str(json)?;
        self.spec = Some(parsed);
        Ok(())
    }

    /// Load the models.dev spec from a pre-parsed value.
    pub fn load_spec(&mut self, spec: ModelsDevSpec) {
        self.spec = Some(spec);
    }

    /// Return the provider spec for a given provider ID.
    pub fn provider_spec(&self, provider: &str) -> Option<ProviderSpec> {
        self.spec.as_ref()?.get(provider).cloned()
    }

    /// List all available provider IDs from the loaded spec.
    pub fn spec_providers(&self) -> Vec<String> {
        match &self.spec {
            Some(spec) => spec.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// List models for a provider from the loaded spec, filtered to only those
    /// whose effective npm package has a registered factory.
    pub fn list_models(&self, provider: &str) -> Vec<ModelSpec> {
        let spec = match &self.spec {
            Some(s) => s,
            None => return Vec::new(),
        };
        let provider_spec = match spec.get(provider) {
            Some(p) => p,
            None => return Vec::new(),
        };

        provider_spec
            .models
            .values()
            .filter(|model| {
                let npm = model
                    .provider
                    .as_ref()
                    .and_then(|p| p.npm.as_ref())
                    .or(provider_spec.npm.as_ref());
                match npm {
                    Some(n) => self.factories.contains_key(n.as_str()),
                    None => false,
                }
            })
            .cloned()
            .collect()
    }

    /// Get a specific model's spec metadata.
    pub fn model_spec(&self, provider: &str, model_id: &str) -> Option<ModelSpec> {
        self.spec
            .as_ref()?
            .get(provider)?
            .models
            .get(model_id)
            .cloned()
    }

    // -----------------------------------------------------------------------
    // Availability
    // -----------------------------------------------------------------------

    /// Return the list of providers that are currently **available**, meaning:
    ///
    /// 1. At least one of the provider's `env` vars is set in the environment,
    ///    **and**
    /// 2. At least one of the provider's models (accounting for model-level
    ///    `provider.npm` overrides) or the provider-level `npm` itself has a
    ///    registered factory.
    ///
    /// Providers registered directly via [`add_provider`](Self::add_provider)
    /// are always considered available.
    pub fn available_providers(&self) -> Vec<AvailableProvider> {
        let mut result: Vec<AvailableProvider> = self
            .providers
            .keys()
            .map(|name| AvailableProvider {
                id: name.clone(),
                name: self
                    .spec
                    .as_ref()
                    .and_then(|s| s.get(name))
                    .map(|ps| ps.name.clone())
                    .unwrap_or_else(|| name.clone()),
            })
            .collect();

        let spec = match &self.spec {
            Some(s) => s,
            None => return result,
        };

        for (provider_id, provider_spec) in spec {
            // Skip if already covered by a direct registration.
            if self.providers.contains_key(provider_id) {
                continue;
            }

            // Must have at least one env var set.
            let has_env = provider_spec
                .env
                .iter()
                .any(|var| std::env::var(var).is_ok());
            if !has_env {
                continue;
            }

            // Collect the set of npm packages used across all models.
            let has_compatible = self.provider_has_compatible_factory(provider_spec);
            if !has_compatible {
                continue;
            }

            result.push(AvailableProvider {
                id: provider_id.clone(),
                name: provider_spec.name.clone(),
            });
        }

        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    /// Check whether a provider has at least one model (or its default npm)
    /// that maps to a registered factory.
    fn provider_has_compatible_factory(&self, provider_spec: &ProviderSpec) -> bool {
        // Check provider-level npm.
        if let Some(npm) = &provider_spec.npm
            && self.factories.contains_key(npm.as_str())
        {
            return true;
        }

        // Check model-level overrides.
        for model in provider_spec.models.values() {
            if let Some(ref p) = model.provider
                && let Some(ref npm) = p.npm
                && self.factories.contains_key(npm.as_str())
            {
                return true;
            }
        }

        false
    }

    // -----------------------------------------------------------------------
    // Internal: npm-routed resolution
    // -----------------------------------------------------------------------

    /// Try to resolve a model through the npm factory path.
    /// Returns `Ok(None)` if the spec doesn't have info for this
    /// provider/model or no factory is registered for the npm package.
    fn model_via_npm(
        &mut self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<Option<LanguageModel>, Error> {
        // Read spec to determine the effective npm package and provider options.
        let spec = match &self.spec {
            Some(s) => s,
            None => return Ok(None),
        };
        let provider_spec = match spec.get(provider_id) {
            Some(ps) => ps,
            None => return Ok(None),
        };

        // Effective npm: model-level override > provider-level default.
        let model_spec = provider_spec.models.get(model_id);
        let effective_npm = model_spec
            .and_then(|m| m.provider.as_ref())
            .and_then(|p| p.npm.as_ref())
            .or(provider_spec.npm.as_ref());

        let npm = match effective_npm {
            Some(n) => n.clone(),
            None => return Ok(None),
        };

        // Resolve API key from the provider's env vars.
        let api_key = provider_spec
            .env
            .iter()
            .find_map(|var| std::env::var(var).ok());

        // API endpoint from the provider spec.
        let api_endpoint = provider_spec.api.clone();

        let entry = match self.factories.get_mut(&npm) {
            Some(e) => e,
            None => return Ok(None),
        };

        // Cache key: provider_id so that the same npm factory can serve
        // multiple providers with distinct credentials/endpoints.
        let cache_key = provider_id.to_string();

        if !entry.instances.contains_key(&cache_key) {
            let options = ProviderOptions {
                id: provider_id.to_string(),
                api_key,
                api_endpoint,
            };
            let instance = entry.factory.create(options)?;
            entry.instances.insert(cache_key.clone(), instance);
        }

        let provider_instance = &entry.instances[&cache_key];
        Ok(Some(provider_instance.model(model_id)))
    }

    /// Fall back to a directly-registered provider.
    fn model_via_direct(
        &mut self,
        provider_name: &str,
        model_id: &str,
    ) -> Result<LanguageModel, Error> {
        if !self.providers.contains_key(provider_name) {
            return Err(Error::ProviderNotFound(provider_name.to_string()));
        }

        let needs_init = self.providers[provider_name].instance.is_none();
        if needs_init {
            let options = self.build_direct_options(provider_name);
            let entry = self.providers.get_mut(provider_name).unwrap();
            let instance = entry.factory.create(options)?;
            entry.instance = Some(instance);
        }

        Ok(self.providers[provider_name]
            .instance
            .as_ref()
            .unwrap()
            .model(model_id))
    }

    /// Build [`ProviderOptions`] for a directly-registered provider.
    fn build_direct_options(&self, provider_name: &str) -> ProviderOptions {
        let provider_spec = self.spec.as_ref().and_then(|s| s.get(provider_name));

        let api_key = provider_spec
            .and_then(|ps| ps.env.iter().find_map(|var| std::env::var(var).ok()));

        let api_endpoint = provider_spec.and_then(|ps| ps.api.clone());

        ProviderOptions {
            id: provider_name.to_string(),
            api_key,
            api_endpoint,
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
