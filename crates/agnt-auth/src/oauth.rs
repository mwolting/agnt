use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use agnt_llm_registry::OAuthPkceAuth;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::random;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::Error;

#[derive(Debug, Clone)]
pub struct OAuthStart {
    pub authorize_url: String,
    pub verifier: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct OAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

pub fn begin_pkce(config: &OAuthPkceAuth) -> Result<OAuthStart, Error> {
    let verifier = create_pkce_verifier();
    let challenge = create_pkce_challenge(&verifier);
    let state = create_state();
    let authorize_url = build_authorize_url(config, &challenge, &state)?;
    Ok(OAuthStart {
        authorize_url,
        verifier,
        state,
    })
}

pub fn extract_code_from_input(input: &str, expected_state: &str) -> Result<String, Error> {
    let value = input.trim();
    if value.is_empty() {
        return Err(Error::MissingOAuthCode);
    }

    let (code, state) = match Url::parse(value) {
        Ok(url) => (
            url.query_pairs().find_map(|(k, v)| {
                if k == "code" {
                    Some(v.to_string())
                } else {
                    None
                }
            }),
            url.query_pairs().find_map(|(k, v)| {
                if k == "state" {
                    Some(v.to_string())
                } else {
                    None
                }
            }),
        ),
        Err(_) => {
            if value.contains("code=") {
                let params = url::form_urlencoded::parse(value.as_bytes())
                    .into_owned()
                    .collect::<HashMap<_, _>>();
                (params.get("code").cloned(), params.get("state").cloned())
            } else if value.contains('#') {
                let mut parts = value.splitn(2, '#');
                (
                    parts.next().map(str::to_string),
                    parts.next().map(str::to_string),
                )
            } else {
                (Some(value.to_string()), None)
            }
        }
    };

    if let Some(state) = state
        && state != expected_state
    {
        return Err(Error::OAuthStateMismatch);
    }

    code.ok_or(Error::MissingOAuthCode)
}

pub async fn exchange_authorization_code(
    config: &OAuthPkceAuth,
    code: &str,
    verifier: &str,
) -> Result<OAuthCredential, Error> {
    let mut form: HashMap<String, String> = HashMap::new();
    form.insert("grant_type".to_string(), "authorization_code".to_string());
    form.insert("client_id".to_string(), config.client_id.clone());
    form.insert("code".to_string(), code.to_string());
    form.insert("code_verifier".to_string(), verifier.to_string());
    form.insert("redirect_uri".to_string(), config.redirect_url.clone());
    for (k, v) in &config.token_params {
        form.insert(k.clone(), v.clone());
    }

    let client = reqwest::Client::new();
    let res = client.post(&config.token_url).form(&form).send().await?;
    let body: OAuthTokenResponse = res.error_for_status()?.json().await?;

    let access_token = body.access_token.ok_or(Error::InvalidOAuthTokenResponse)?;
    let refresh_token = body.refresh_token.ok_or(Error::InvalidOAuthTokenResponse)?;
    let expires_in = body.expires_in.ok_or(Error::InvalidOAuthTokenResponse)?;

    Ok(OAuthCredential {
        access_token,
        refresh_token,
        expires_at_ms: now_ms().saturating_add(expires_in.saturating_mul(1000)),
    })
}

pub async fn refresh_pkce_token(
    config: &OAuthPkceAuth,
    refresh_token: &str,
) -> Result<OAuthCredential, Error> {
    let mut form: HashMap<String, String> = HashMap::new();
    form.insert("grant_type".to_string(), "refresh_token".to_string());
    form.insert("client_id".to_string(), config.client_id.clone());
    form.insert("refresh_token".to_string(), refresh_token.to_string());
    for (k, v) in &config.token_params {
        form.insert(k.clone(), v.clone());
    }

    let client = reqwest::Client::new();
    let res = client.post(&config.token_url).form(&form).send().await?;
    let body: OAuthTokenResponse = res.error_for_status()?.json().await?;

    let access_token = body.access_token.ok_or(Error::InvalidOAuthTokenResponse)?;
    let refresh_token = body
        .refresh_token
        .unwrap_or_else(|| refresh_token.to_string());
    let expires_in = body.expires_in.ok_or(Error::InvalidOAuthTokenResponse)?;

    Ok(OAuthCredential {
        access_token,
        refresh_token,
        expires_at_ms: now_ms().saturating_add(expires_in.saturating_mul(1000)),
    })
}

fn create_state() -> String {
    let bytes: [u8; 16] = random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn create_pkce_verifier() -> String {
    let bytes: [u8; 32] = random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn create_pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn build_authorize_url(
    config: &OAuthPkceAuth,
    challenge: &str,
    state: &str,
) -> Result<String, Error> {
    let mut url = Url::parse(&config.authorize_url)
        .map_err(|_| Error::InvalidRedirectUrl(config.authorize_url.clone()))?;

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.redirect_url)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);

    if !config.scopes.is_empty() {
        url.query_pairs_mut()
            .append_pair("scope", &config.scopes.join(" "));
    }
    for (k, v) in &config.authorize_params {
        url.query_pairs_mut().append_pair(k, v);
    }

    Ok(url.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
