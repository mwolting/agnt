use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agnt_llm_registry::{
    ApiKeyAuth, AuthMethod, AuthRequest, AuthResolver, OAuthPkceAuth, ResolvedAuth,
};

use crate::error::Error;
use crate::oauth::{
    OAuthCredential, OAuthStart, begin_pkce, exchange_authorization_code, extract_code_from_input,
    refresh_pkce_token,
};
use crate::store::{KeyringStore, StoredCredential};

/// Headless auth manager (keyring + oauth/api-key resolution/persistence).
pub struct AuthManager {
    store: KeyringStore,
    cache: Mutex<HashMap<String, StoredCredential>>,
}

impl AuthManager {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            store: KeyringStore::new(service_name),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn resolver(self: &Arc<Self>) -> Arc<dyn AuthResolver> {
        Arc::new(AuthManagerResolver {
            manager: Arc::clone(self),
        })
    }

    pub fn resolve_cached(&self, request: &AuthRequest) -> Result<Option<ResolvedAuth>, Error> {
        match &request.auth_method {
            AuthMethod::ApiKey(ApiKeyAuth { env }) => {
                for var in env {
                    if let Ok(value) = std::env::var(var)
                        && !value.trim().is_empty()
                    {
                        return Ok(Some(ResolvedAuth::api_key(value)));
                    }
                }
                match self.load_credential(&request.provider_id)? {
                    Some(StoredCredential::ApiKey { api_key }) => {
                        Ok(Some(ResolvedAuth::api_key(api_key)))
                    }
                    _ => Ok(None),
                }
            }
            AuthMethod::OAuthPkce(_) => match self.load_credential(&request.provider_id)? {
                Some(StoredCredential::OAuthPkce { access_token, .. }) => {
                    Ok(Some(ResolvedAuth::bearer(access_token)))
                }
                _ => Ok(None),
            },
        }
    }

    pub fn store_api_key(
        &self,
        provider_id: &str,
        api_key: impl Into<String>,
    ) -> Result<ResolvedAuth, Error> {
        let api_key = api_key.into();
        let credential = StoredCredential::ApiKey {
            api_key: api_key.clone(),
        };
        self.store.save(provider_id, &credential)?;
        self.cache_set(provider_id, credential);
        Ok(ResolvedAuth::api_key(api_key))
    }

    pub fn begin_oauth(
        &self,
        _provider_id: &str,
        config: &OAuthPkceAuth,
    ) -> Result<OAuthStart, Error> {
        begin_pkce(config)
    }

    pub async fn complete_oauth(
        &self,
        provider_id: &str,
        config: &OAuthPkceAuth,
        pending: &OAuthStart,
        authorization_input: &str,
    ) -> Result<ResolvedAuth, Error> {
        let code = extract_code_from_input(authorization_input, &pending.state)?;
        let credential = exchange_authorization_code(config, &code, &pending.verifier).await?;
        let access_token = credential.access_token.clone();
        self.save_oauth_credential(provider_id, credential)?;
        Ok(ResolvedAuth::bearer(access_token))
    }

    pub async fn refresh_oauth_if_needed(
        &self,
        provider_id: &str,
        config: &OAuthPkceAuth,
    ) -> Result<Option<ResolvedAuth>, Error> {
        let Some(stored) = self.load_credential(provider_id)? else {
            return Ok(None);
        };
        let StoredCredential::OAuthPkce {
            access_token,
            refresh_token,
            expires_at_ms,
            metadata,
        } = stored
        else {
            return Ok(None);
        };

        if expires_at_ms > now_ms() {
            return Ok(Some(ResolvedAuth::bearer(access_token)));
        }

        let refreshed = refresh_pkce_token(config, &refresh_token).await?;
        let credential = StoredCredential::OAuthPkce {
            access_token: refreshed.access_token.clone(),
            refresh_token: refreshed.refresh_token,
            expires_at_ms: refreshed.expires_at_ms,
            metadata,
        };
        self.store.save(provider_id, &credential)?;
        self.cache_set(provider_id, credential);
        Ok(Some(ResolvedAuth::bearer(refreshed.access_token)))
    }

    fn save_oauth_credential(
        &self,
        provider_id: &str,
        credential: OAuthCredential,
    ) -> Result<(), Error> {
        let stored = StoredCredential::OAuthPkce {
            access_token: credential.access_token,
            refresh_token: credential.refresh_token,
            expires_at_ms: credential.expires_at_ms,
            metadata: Default::default(),
        };
        self.store.save(provider_id, &stored)?;
        self.cache_set(provider_id, stored);
        Ok(())
    }

    fn load_credential(&self, provider_id: &str) -> Result<Option<StoredCredential>, Error> {
        if let Some(value) = self.cache_get(provider_id) {
            return Ok(Some(value));
        }

        let loaded = self.store.load(provider_id)?;
        if let Some(ref value) = loaded {
            self.cache_set(provider_id, value.clone());
        }
        Ok(loaded)
    }

    fn cache_get(&self, provider_id: &str) -> Option<StoredCredential> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(provider_id)
            .cloned()
    }

    fn cache_set(&self, provider_id: &str, credential: StoredCredential) {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(provider_id.to_string(), credential);
    }
}

struct AuthManagerResolver {
    manager: Arc<AuthManager>,
}

impl AuthResolver for AuthManagerResolver {
    fn resolve(
        &self,
        request: &AuthRequest,
    ) -> Result<Option<ResolvedAuth>, agnt_llm_registry::Error> {
        self.manager
            .resolve_cached(request)
            .map_err(|e| agnt_llm_registry::Error::Factory(Box::new(e)))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
