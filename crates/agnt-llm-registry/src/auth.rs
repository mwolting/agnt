//! Provider auth method declarations and resolved auth payloads.

use std::collections::HashMap;

use crate::error::Error;

/// Declarative auth method for a provider.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// API key credentials (default).
    ApiKey(ApiKeyAuth),
    /// OAuth authorization code flow with PKCE.
    OAuthPkce(OAuthPkceAuth),
}

impl AuthMethod {
    /// Stable method identifier (e.g. `"api_key"` or `"oauth_pkce"`).
    pub fn kind(&self) -> &str {
        match self {
            AuthMethod::ApiKey(_) => "api_key",
            AuthMethod::OAuthPkce(_) => "oauth_pkce",
        }
    }
}

impl Default for AuthMethod {
    fn default() -> Self {
        Self::ApiKey(ApiKeyAuth::default())
    }
}

/// API key auth configuration.
#[derive(Debug, Clone, Default)]
pub struct ApiKeyAuth {
    /// Candidate environment variable names for this API key.
    pub env: Vec<String>,
}

/// OAuth PKCE configuration.
#[derive(Debug, Clone, Default)]
pub struct OAuthPkceAuth {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
    /// Extra query parameters for the authorization URL.
    pub authorize_params: HashMap<String, String>,
    /// Extra body parameters for token exchange/refresh.
    pub token_params: HashMap<String, String>,
}

/// Resolved provider auth payload returned by an auth resolver.
#[derive(Debug, Clone, Default)]
pub struct ResolvedAuth {
    /// Auth method identifier (e.g. `"api_key"`, `"oauth_pkce"`).
    pub method: String,
    /// Key/value payload (e.g. `api_key`, `access_token`, `account_id`).
    pub values: HashMap<String, String>,
}

impl ResolvedAuth {
    pub fn api_key(api_key: impl Into<String>) -> Self {
        let mut values = HashMap::new();
        values.insert("api_key".to_string(), api_key.into());
        Self {
            method: "api_key".to_string(),
            values,
        }
    }

    pub fn bearer(token: impl Into<String>) -> Self {
        let mut values = HashMap::new();
        values.insert("access_token".to_string(), token.into());
        Self {
            method: "oauth_pkce".to_string(),
            values,
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

/// Request passed to an auth resolver.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub provider_id: String,
    pub provider_name: String,
    pub auth_method: AuthMethod,
    pub env_candidates: Vec<String>,
}

/// External hook used to resolve credentials (keyring, OAuth refresh, etc).
pub trait AuthResolver: Send + Sync {
    fn resolve(&self, request: &AuthRequest) -> Result<Option<ResolvedAuth>, Error>;
}
