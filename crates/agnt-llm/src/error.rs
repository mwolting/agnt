use std::collections::HashMap;

/// Errors that can occur when interacting with a language model.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(Box<dyn std::error::Error + Send + Sync>),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("sse error: {0}")]
    Sse(String),

    #[error("api error ({code}): {message}")]
    Api {
        code: String,
        message: String,
        metadata: HashMap<String, serde_json::Value>,
    },

    #[error("{0}")]
    Other(String),
}
