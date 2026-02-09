#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("credential json parse error: {0}")]
    CredentialParse(#[from] serde_json::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported auth method: {0}")]
    UnsupportedAuthMethod(String),

    #[error("missing credentials for provider '{provider}' (auth method: {method})")]
    MissingCredentials { provider: String, method: String },

    #[error("oauth callback state mismatch")]
    OAuthStateMismatch,

    #[error("missing oauth authorization code")]
    MissingOAuthCode,

    #[error("oauth token response missing required fields")]
    InvalidOAuthTokenResponse,

    #[error("failed to parse redirect url: {0}")]
    InvalidRedirectUrl(String),

    #[error("{0}")]
    Other(String),
}
