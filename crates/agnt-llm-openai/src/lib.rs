mod convert;
#[cfg(feature = "registry")]
mod register;
mod stream;
mod types;

#[cfg(feature = "registry")]
pub use register::{
    OpenAIProviderBehavior, register, register_oauth_provider,
    register_oauth_provider_with_behavior,
};

use agnt_llm::request::GenerateRequest;
use agnt_llm::response::Response;
use agnt_llm::{
    LanguageModel, LanguageModelBackend, LanguageModelProvider, LanguageModelProviderBackend,
    RequestBuilder,
};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the OpenAI provider.
pub struct OpenAIConfig {
    pub auth_token: String,
    pub base_url: String,
    /// Whether to send the Responses API `store` field.
    /// - `Some(false)` is required for Codex OAuth endpoints.
    /// - `None` omits the field.
    pub response_store: Option<bool>,
    /// Whether to request encrypted reasoning content in responses.
    pub include_reasoning_encrypted_content: bool,
    /// Additional headers to include in every request.
    pub extra_headers: HashMap<String, String>,
    /// Whether to derive and send `chatgpt-account-id` from the auth token.
    pub include_chatgpt_account_id_header: bool,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            auth_token: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            response_store: None,
            include_reasoning_encrypted_content: false,
            extra_headers: HashMap::new(),
            include_chatgpt_account_id_header: false,
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
        auth_token: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Extension trait for OpenAI-specific request options
// ---------------------------------------------------------------------------

/// Extension methods for [`RequestBuilder`] that set OpenAI-specific options.
///
/// ```ignore
/// use agnt_llm_openai::OpenAIRequestExt;
///
/// let mut req = agnt_llm::request();
/// req.system("You are helpful")
///    .user("Explain monads")
///    .reasoning_effort("high")
///    .temperature(0.7);
/// model.generate(req);
/// ```
/// Reasoning effort level for o-series / gpt-5 models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::None => "none",
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
        }
    }
}

/// Reasoning summary setting for reasoning models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningSummary {
    /// Automatically select the most detailed summary available for the model.
    Auto,
    /// Concise summaries (supported by some models, e.g. computer-use).
    Concise,
    /// Detailed summaries (supported by most reasoning models).
    Detailed,
}

impl ReasoningSummary {
    fn as_str(self) -> &'static str {
        match self {
            ReasoningSummary::Auto => "auto",
            ReasoningSummary::Concise => "concise",
            ReasoningSummary::Detailed => "detailed",
        }
    }
}

pub trait OpenAIRequestExt {
    /// Set reasoning effort for o-series / gpt-5 models.
    fn reasoning_effort(&mut self, effort: ReasoningEffort) -> &mut Self;
    /// Set reasoning summary mode for reasoning models.
    fn reasoning_summary(&mut self, summary: ReasoningSummary) -> &mut Self;
}

impl OpenAIRequestExt for RequestBuilder {
    fn reasoning_effort(&mut self, effort: ReasoningEffort) -> &mut Self {
        self.meta("reasoning_effort", effort.as_str())
    }

    fn reasoning_summary(&mut self, summary: ReasoningSummary) -> &mut Self {
        self.meta("reasoning_summary", summary.as_str())
    }
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
        let body = convert::to_openai_request(&self.model_id, &request, &self.state.config);
        let state = Arc::clone(&self.state);
        let event_stream = stream::open(state, body);
        Response::new(event_stream)
    }
}
