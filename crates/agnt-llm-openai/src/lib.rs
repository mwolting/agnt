mod convert;
mod stream;
mod types;

use agnt_llm::request::GenerateRequest;
use agnt_llm::response::Response;
use agnt_llm::{
    LanguageModel, LanguageModelBackend, LanguageModelProvider, LanguageModelProviderBackend,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the OpenAI provider.
pub struct OpenAIConfig {
    pub api_key: String,
    pub base_url: String,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".into(),
        }
    }
}

/// Create an OpenAI provider with the given config.
pub fn provider(config: OpenAIConfig) -> LanguageModelProvider {
    LanguageModelProvider::new(OpenAIProvider {
        state: Arc::new(ProviderState {
            client: reqwest::Client::new(),
            config,
        }),
    })
}

/// Create an OpenAI provider reading `OPENAI_API_KEY` from the environment.
pub fn from_env() -> LanguageModelProvider {
    provider(OpenAIConfig {
        api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

struct ProviderState {
    client: reqwest::Client,
    config: OpenAIConfig,
}

struct OpenAIProvider {
    state: Arc<ProviderState>,
}

impl LanguageModelProviderBackend for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn model(&self, model_id: &str) -> LanguageModel {
        LanguageModel::new(OpenAIModel {
            model_id: model_id.to_string(),
            state: Arc::clone(&self.state),
        })
    }
}

struct OpenAIModel {
    model_id: String,
    state: Arc<ProviderState>,
}

impl LanguageModelBackend for OpenAIModel {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn provider(&self) -> &str {
        "openai"
    }

    fn generate(&self, request: GenerateRequest) -> Response {
        let body = convert::to_openai_request(&self.model_id, &request);
        let state = Arc::clone(&self.state);
        let event_stream = stream::open(state, body);
        Response::new(event_stream)
    }
}
