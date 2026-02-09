//! The core registry: maps provider names to factories and resolves models.

use std::collections::HashMap;
use std::sync::Arc;

use agnt_llm::{LanguageModel, LanguageModelProvider};

use crate::auth::{ApiKeyAuth, AuthMethod, AuthRequest, AuthResolver, ResolvedAuth};
use crate::error::Error;
use crate::factory::{ProviderFactory, ProviderOptions};
use crate::model_source::ModelSource;
use crate::provider::ProviderRegistration;
use crate::spec::{ModelSpec, ModelsDevSpec, ProviderSpec};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// A provider that is configured and compatible with at least one factory.
#[derive(Debug, Clone)]
pub struct AvailableProvider {
    /// Provider identifier (e.g. `"openai"`).
    pub id: String,
    /// Human-friendly display name (e.g. `"OpenAI"`).
    pub name: String,
}

/// A provider known to the registry.
#[derive(Debug, Clone)]
pub struct KnownProvider {
    /// Provider identifier (e.g. `"openai"`).
    pub id: String,
    /// Human-friendly display name.
    pub name: String,
    /// Auth method kind identifier.
    pub auth_method: String,
    /// True when credentials are currently resolvable.
    pub configured: bool,
    /// True when at least one registered factory (or direct provider) can serve it.
    pub compatible: bool,
}

/// A cached provider instance plus the auth signature used to build it.
struct CachedProvider {
    auth_signature: String,
    provider: LanguageModelProvider,
}

/// An entry keyed by npm package name.
struct NpmEntry {
    factory: Box<dyn ProviderFactory>,
    instances: HashMap<String, CachedProvider>,
}

/// A directly-registered provider (non-npm path).
struct ProviderEntry {
    factory: Box<dyn ProviderFactory>,
    instance: Option<CachedProvider>,
}

/// Central provider/model registry.
pub struct Registry {
    /// Factories keyed by npm package name (e.g. `"@ai-sdk/openai"`).
    factories: HashMap<String, NpmEntry>,
    /// Simple named providers registered directly (no npm routing).
    providers: HashMap<String, ProviderEntry>,
    /// Explicitly registered provider metadata (auth method, model source, etc).
    registrations: HashMap<String, ProviderRegistration>,
    spec: Option<ModelsDevSpec>,
    auth_resolver: Option<Arc<dyn AuthResolver>>,
}

impl Registry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            providers: HashMap::new(),
            registrations: HashMap::new(),
            spec: None,
            auth_resolver: None,
        }
    }

    /// Set the credential resolver used for provider auth resolution.
    pub fn set_auth_resolver(&mut self, resolver: Arc<dyn AuthResolver>) {
        self.auth_resolver = Some(resolver);
    }

    /// Register provider metadata, including auth method and model source.
    pub fn add_registration(&mut self, registration: ProviderRegistration) {
        self.registrations
            .insert(registration.id.clone(), registration);
    }

    // -----------------------------------------------------------------------
    // Factory registration
    // -----------------------------------------------------------------------

    /// Register a factory for a given npm package name.
    pub fn add_factory(&mut self, npm: impl Into<String>, factory: impl ProviderFactory + 'static) {
        self.factories.insert(
            npm.into(),
            NpmEntry {
                factory: Box::new(factory),
                instances: HashMap::new(),
            },
        );
    }

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

    /// Check whether a provider is known and compatible with a registered factory.
    pub fn has_provider(&self, name: &str) -> bool {
        self.known_providers()
            .iter()
            .any(|provider| provider.id == name && provider.compatible)
    }

    // -----------------------------------------------------------------------
    // Model resolution
    // -----------------------------------------------------------------------

    /// Obtain a [`LanguageModel`] for the given provider and model ID.
    pub fn model(&mut self, provider: &str, model_id: &str) -> Result<LanguageModel, Error> {
        if let Some(result) = self.model_via_registered(provider, model_id)? {
            return Ok(result);
        }

        if let Some(result) = self.model_via_spec(provider, model_id)? {
            return Ok(result);
        }

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

    /// List all provider IDs from the loaded models.dev spec.
    pub fn spec_providers(&self) -> Vec<String> {
        match &self.spec {
            Some(spec) => spec.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// List models for a provider from the provider registration or models.dev.
    pub fn list_models(&self, provider: &str) -> Vec<ModelSpec> {
        if let Some(registration) = self.registrations.get(provider) {
            return self.list_registered_models(provider, registration);
        }
        self.list_spec_models(provider)
    }

    /// Get a specific model's metadata.
    pub fn model_spec(&self, provider: &str, model_id: &str) -> Option<ModelSpec> {
        if let Some(registration) = self.registrations.get(provider) {
            let models = self.models_from_registration(provider, registration).ok()?;
            return models.into_iter().find(|m| m.id == model_id);
        }

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

    /// Return all providers known to this registry.
    pub fn known_providers(&self) -> Vec<KnownProvider> {
        let mut ids: HashMap<String, ()> = HashMap::new();

        for id in self.providers.keys() {
            ids.insert(id.clone(), ());
        }
        for id in self.registrations.keys() {
            ids.insert(id.clone(), ());
        }
        if let Some(spec) = &self.spec {
            for id in spec.keys() {
                ids.insert(id.clone(), ());
            }
        }

        let mut providers: Vec<KnownProvider> = ids
            .into_keys()
            .map(|id| self.build_known_provider(&id))
            .collect();
        providers.sort_by(|a, b| a.id.cmp(&b.id));
        providers
    }

    /// Return providers that are configured and compatible.
    pub fn available_providers(&self) -> Vec<AvailableProvider> {
        self.known_providers()
            .into_iter()
            .filter(|p| p.configured && p.compatible)
            .map(|p| AvailableProvider {
                id: p.id,
                name: p.name,
            })
            .collect()
    }

    /// Build an auth request for a provider, if known to the registry.
    pub fn auth_request(&self, provider_id: &str) -> Option<AuthRequest> {
        self.build_auth_request(provider_id)
    }

    // -----------------------------------------------------------------------
    // Internal: provider/model metadata
    // -----------------------------------------------------------------------

    fn list_registered_models(
        &self,
        provider_id: &str,
        registration: &ProviderRegistration,
    ) -> Vec<ModelSpec> {
        let models = match self.models_from_registration(provider_id, registration) {
            Ok(models) => models,
            Err(_) => return Vec::new(),
        };

        models
            .into_iter()
            .filter(|model| {
                if self.providers.contains_key(provider_id) {
                    return true;
                }

                let effective_npm = model
                    .provider
                    .as_ref()
                    .and_then(|p| p.npm.as_ref())
                    .cloned()
                    .or_else(|| registration.npm_packages.first().cloned());
                effective_npm
                    .as_ref()
                    .map(|npm| self.factories.contains_key(npm.as_str()))
                    .unwrap_or(false)
            })
            .collect()
    }

    fn list_spec_models(&self, provider: &str) -> Vec<ModelSpec> {
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
                if self.providers.contains_key(provider) {
                    return true;
                }

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

    fn models_from_registration(
        &self,
        provider_id: &str,
        registration: &ProviderRegistration,
    ) -> Result<Vec<ModelSpec>, Error> {
        match &registration.model_source {
            ModelSource::ModelsDev => Ok(self
                .spec
                .as_ref()
                .and_then(|s| s.get(provider_id))
                .map(|ps| ps.models.values().cloned().collect())
                .unwrap_or_default()),
            ModelSource::Static(models) => Ok(models.clone()),
            ModelSource::Dynamic(loader) => loader.load_models(provider_id),
        }
    }

    fn build_known_provider(&self, provider_id: &str) -> KnownProvider {
        let auth_request = self
            .build_auth_request(provider_id)
            .unwrap_or_else(|| AuthRequest {
                provider_id: provider_id.to_string(),
                provider_name: provider_id.to_string(),
                auth_method: AuthMethod::ApiKey(ApiKeyAuth::default()),
                env_candidates: Vec::new(),
            });

        let compatible = self.provider_is_compatible(provider_id);
        let configured = self
            .resolve_auth_optional(
                provider_id,
                &auth_request.provider_name,
                &auth_request.auth_method,
                auth_request.env_candidates.clone(),
            )
            .is_some();

        KnownProvider {
            id: provider_id.to_string(),
            name: auth_request.provider_name,
            auth_method: auth_request.auth_method.kind().to_string(),
            configured,
            compatible,
        }
    }

    fn provider_is_compatible(&self, provider_id: &str) -> bool {
        if self.providers.contains_key(provider_id) {
            return true;
        }

        if let Some(registration) = self.registrations.get(provider_id)
            && registration
                .npm_packages
                .iter()
                .any(|npm| self.factories.contains_key(npm.as_str()))
        {
            return true;
        }

        if let Some(provider_spec) = self.spec.as_ref().and_then(|s| s.get(provider_id)) {
            if let Some(npm) = &provider_spec.npm
                && self.factories.contains_key(npm.as_str())
            {
                return true;
            }

            for model in provider_spec.models.values() {
                if let Some(ref p) = model.provider
                    && let Some(ref npm) = p.npm
                    && self.factories.contains_key(npm.as_str())
                {
                    return true;
                }
            }
        }

        false
    }

    // -----------------------------------------------------------------------
    // Internal: model resolution
    // -----------------------------------------------------------------------

    fn model_via_registered(
        &mut self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<Option<LanguageModel>, Error> {
        let registration = match self.registrations.get(provider_id).cloned() {
            Some(reg) => reg,
            None => return Ok(None),
        };

        let provider_spec = self.spec.as_ref().and_then(|s| s.get(provider_id)).cloned();
        let models = self.models_from_registration(provider_id, &registration)?;
        let model = models.into_iter().find(|m| m.id == model_id);

        if let Some(model) = model {
            let effective_npm = model
                .provider
                .as_ref()
                .and_then(|p| p.npm.as_ref())
                .cloned()
                .or_else(|| {
                    provider_spec
                        .as_ref()
                        .and_then(|ps| ps.npm.as_ref())
                        .cloned()
                        .or_else(|| registration.npm_packages.first().cloned())
                });

            let auth = self.resolve_auth_required(
                provider_id,
                &registration.name,
                &registration.auth_method,
                env_candidates_for_auth(&registration.auth_method),
            )?;
            let options = ProviderOptions {
                id: provider_id.to_string(),
                api_endpoint: registration
                    .api_endpoint
                    .clone()
                    .or_else(|| provider_spec.as_ref().and_then(|ps| ps.api.clone())),
                factory_options: registration.factory_options.clone(),
                auth,
            };

            if let Some(npm) = effective_npm
                && let Some(model) =
                    self.model_from_npm(&npm, provider_id, model_id, options.clone())?
            {
                return Ok(Some(model));
            }

            if self.providers.contains_key(provider_id) {
                return self.model_via_direct_with_options(provider_id, model_id, options);
            }

            return Err(Error::ProviderNotFound(format!(
                "{provider_id} (no compatible provider factory)"
            )));
        }

        // If registration has explicit models and none matched, return not found.
        if !self
            .models_from_registration(provider_id, &registration)?
            .is_empty()
        {
            return Err(Error::ModelNotFound {
                provider: provider_id.to_string(),
                model: model_id.to_string(),
            });
        }

        Ok(None)
    }

    fn model_via_spec(
        &mut self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<Option<LanguageModel>, Error> {
        let spec = match &self.spec {
            Some(s) => s,
            None => return Ok(None),
        };
        let provider_spec = match spec.get(provider_id) {
            Some(ps) => ps.clone(),
            None => return Ok(None),
        };

        let model_spec = match provider_spec.models.get(model_id) {
            Some(model) => model,
            None => return Ok(None),
        };

        let effective_npm = model_spec
            .provider
            .as_ref()
            .and_then(|p| p.npm.as_ref())
            .or(provider_spec.npm.as_ref())
            .cloned();

        let npm = match effective_npm {
            Some(n) => n,
            None => return Ok(None),
        };

        let auth_method = AuthMethod::ApiKey(ApiKeyAuth {
            env: provider_spec.env.clone(),
        });
        let auth = self.resolve_auth_required(
            provider_id,
            &provider_spec.name,
            &auth_method,
            provider_spec.env.clone(),
        )?;
        let options = ProviderOptions {
            id: provider_id.to_string(),
            api_endpoint: provider_spec.api.clone(),
            factory_options: None,
            auth,
        };

        self.model_from_npm(&npm, provider_id, model_id, options)
    }

    fn model_from_npm(
        &mut self,
        npm: &str,
        provider_id: &str,
        model_id: &str,
        options: ProviderOptions,
    ) -> Result<Option<LanguageModel>, Error> {
        let signature = options_signature(&options);
        let entry = match self.factories.get_mut(npm) {
            Some(e) => e,
            None => return Ok(None),
        };

        let needs_rebuild = entry
            .instances
            .get(provider_id)
            .map(|cached| cached.auth_signature != signature)
            .unwrap_or(true);
        if needs_rebuild {
            let instance = entry.factory.create(options)?;
            entry.instances.insert(
                provider_id.to_string(),
                CachedProvider {
                    auth_signature: signature,
                    provider: instance,
                },
            );
        }

        let provider_instance = &entry.instances[provider_id].provider;
        Ok(Some(provider_instance.model(model_id)))
    }

    fn model_via_direct(
        &mut self,
        provider_name: &str,
        model_id: &str,
    ) -> Result<LanguageModel, Error> {
        if !self.providers.contains_key(provider_name) {
            return Err(Error::ProviderNotFound(provider_name.to_string()));
        }

        let options = self.build_direct_options(provider_name)?;
        self.model_via_direct_with_options(provider_name, model_id, options)?
            .ok_or_else(|| Error::ProviderNotFound(provider_name.to_string()))
    }

    fn model_via_direct_with_options(
        &mut self,
        provider_name: &str,
        model_id: &str,
        options: ProviderOptions,
    ) -> Result<Option<LanguageModel>, Error> {
        let signature = options_signature(&options);
        let entry = match self.providers.get_mut(provider_name) {
            Some(entry) => entry,
            None => return Ok(None),
        };

        let needs_init = entry
            .instance
            .as_ref()
            .map(|cached| cached.auth_signature != signature)
            .unwrap_or(true);
        if needs_init {
            let instance = entry.factory.create(options)?;
            entry.instance = Some(CachedProvider {
                auth_signature: signature,
                provider: instance,
            });
        }

        Ok(Some(
            entry
                .instance
                .as_ref()
                .expect("direct provider instance should exist")
                .provider
                .model(model_id),
        ))
    }

    /// Build [`ProviderOptions`] for a directly-registered provider.
    fn build_direct_options(&self, provider_name: &str) -> Result<ProviderOptions, Error> {
        let registration = self.registrations.get(provider_name).cloned();
        let provider_spec = self
            .spec
            .as_ref()
            .and_then(|s| s.get(provider_name))
            .cloned();

        let auth_method = registration
            .as_ref()
            .map(|r| r.auth_method.clone())
            .unwrap_or_else(|| {
                AuthMethod::ApiKey(ApiKeyAuth {
                    env: provider_spec
                        .as_ref()
                        .map(|ps| ps.env.clone())
                        .unwrap_or_default(),
                })
            });
        let env_candidates = env_candidates_for_auth(&auth_method);
        let provider_name_human = registration
            .as_ref()
            .map(|r| r.name.clone())
            .or_else(|| provider_spec.as_ref().map(|ps| ps.name.clone()))
            .unwrap_or_else(|| provider_name.to_string());
        let auth = self.resolve_auth_required(
            provider_name,
            &provider_name_human,
            &auth_method,
            env_candidates,
        )?;

        let api_endpoint = registration
            .as_ref()
            .and_then(|r| r.api_endpoint.clone())
            .or_else(|| provider_spec.as_ref().and_then(|ps| ps.api.clone()));

        Ok(ProviderOptions {
            id: provider_name.to_string(),
            api_endpoint,
            factory_options: registration
                .as_ref()
                .and_then(|r| r.factory_options.clone()),
            auth,
        })
    }

    // -----------------------------------------------------------------------
    // Internal: auth resolution
    // -----------------------------------------------------------------------

    fn resolve_auth_optional(
        &self,
        provider_id: &str,
        provider_name: &str,
        auth_method: &AuthMethod,
        env_candidates: Vec<String>,
    ) -> Option<ResolvedAuth> {
        if let Some(resolver) = &self.auth_resolver {
            let req = AuthRequest {
                provider_id: provider_id.to_string(),
                provider_name: provider_name.to_string(),
                auth_method: auth_method.clone(),
                env_candidates: env_candidates.clone(),
            };
            if let Ok(Some(auth)) = resolver.resolve(&req) {
                return Some(auth);
            }
        }

        match auth_method {
            AuthMethod::ApiKey(_) => {
                if env_candidates.is_empty() {
                    return Some(ResolvedAuth {
                        method: "api_key".to_string(),
                        values: HashMap::new(),
                    });
                }

                env_candidates
                    .iter()
                    .find_map(|var| std::env::var(var).ok())
                    .map(ResolvedAuth::api_key)
            }
            _ => None,
        }
    }

    fn resolve_auth_required(
        &self,
        provider_id: &str,
        provider_name: &str,
        auth_method: &AuthMethod,
        env_candidates: Vec<String>,
    ) -> Result<ResolvedAuth, Error> {
        self.resolve_auth_optional(provider_id, provider_name, auth_method, env_candidates)
            .ok_or_else(|| Error::MissingCredentials {
                provider: provider_id.to_string(),
                method: auth_method.kind().to_string(),
            })
    }

    fn build_auth_request(&self, provider_id: &str) -> Option<AuthRequest> {
        let registration = self.registrations.get(provider_id);
        let provider_spec = self.spec.as_ref().and_then(|s| s.get(provider_id));

        if registration.is_none()
            && provider_spec.is_none()
            && !self.providers.contains_key(provider_id)
        {
            return None;
        }

        let provider_name = registration
            .map(|r| r.name.clone())
            .or_else(|| provider_spec.map(|s| s.name.clone()))
            .unwrap_or_else(|| provider_id.to_string());
        let auth_method = registration
            .map(|r| r.auth_method.clone())
            .unwrap_or_else(|| {
                AuthMethod::ApiKey(ApiKeyAuth {
                    env: provider_spec.map(|s| s.env.clone()).unwrap_or_default(),
                })
            });

        Some(AuthRequest {
            provider_id: provider_id.to_string(),
            provider_name,
            env_candidates: env_candidates_for_auth(&auth_method),
            auth_method,
        })
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

fn env_candidates_for_auth(auth_method: &AuthMethod) -> Vec<String> {
    match auth_method {
        AuthMethod::ApiKey(cfg) => cfg.env.clone(),
        _ => Vec::new(),
    }
}

fn auth_signature(auth: &ResolvedAuth) -> String {
    let mut kv: Vec<(&str, &str)> = auth
        .values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    kv.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::new();
    out.push_str(auth.method.as_str());
    for (k, v) in kv {
        out.push('|');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

fn options_signature(options: &ProviderOptions) -> String {
    let mut out = String::new();
    out.push_str(options.id.as_str());
    out.push('|');
    if let Some(endpoint) = &options.api_endpoint {
        out.push_str(endpoint);
    }
    out.push('|');
    out.push_str(auth_signature(&options.auth).as_str());
    out.push('|');
    if let Some(factory_options) = &options.factory_options {
        out.push_str(factory_options.to_string().as_str());
    }

    out
}
