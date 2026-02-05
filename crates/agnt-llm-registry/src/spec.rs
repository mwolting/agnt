//! Types representing the [models.dev](https://models.dev) API specification.
//!
//! These structs map 1:1 to the JSON returned by `https://models.dev/api.json`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The full registry payload: a flat map of `provider_id => ProviderSpec`.
pub type ModelsDevSpec = HashMap<String, ProviderSpec>;

/// A provider entry from models.dev.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpec {
    /// Provider identifier, e.g. `"openai"`.
    pub id: String,

    /// Display name, e.g. `"OpenAI"`.
    pub name: String,

    /// Environment variable names for the API key.
    #[serde(default)]
    pub env: Vec<String>,

    /// Optional base API URL. When absent, the provider SDK uses its built-in
    /// default.
    #[serde(default)]
    pub api: Option<String>,

    /// NPM package name for the AI SDK adapter (informational).
    #[serde(default)]
    pub npm: Option<String>,

    /// Link to provider documentation.
    #[serde(default)]
    pub doc: Option<String>,

    /// Models offered by this provider. Key is the model ID.
    #[serde(default)]
    pub models: HashMap<String, ModelSpec>,
}

/// A model entry within a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Model identifier, e.g. `"gpt-4.1-nano"`.
    pub id: String,

    /// Human-friendly display name.
    #[serde(default)]
    pub name: Option<String>,

    /// Model family grouping, e.g. `"gpt-nano"`, `"claude-opus"`.
    #[serde(default)]
    pub family: Option<String>,

    /// Whether file/image attachments are supported.
    #[serde(default)]
    pub attachment: bool,

    /// Whether the model supports reasoning / chain-of-thought.
    #[serde(default)]
    pub reasoning: bool,

    /// Whether the model supports tool/function calling.
    #[serde(default)]
    pub tool_call: bool,

    /// Whether the model supports structured (JSON schema) output.
    #[serde(default)]
    pub structured_output: bool,

    /// Whether the temperature parameter is accepted.
    #[serde(default)]
    pub temperature: bool,

    /// Knowledge cutoff date (e.g. `"2024-04"`).
    #[serde(default)]
    pub knowledge: Option<String>,

    /// Model release date.
    #[serde(default)]
    pub release_date: Option<String>,

    /// Last update date.
    #[serde(default)]
    pub last_updated: Option<String>,

    /// Input/output modalities.
    #[serde(default)]
    pub modalities: Option<Modalities>,

    /// Whether the model weights are openly available.
    #[serde(default)]
    pub open_weights: bool,

    /// Pricing information (per million tokens).
    #[serde(default)]
    pub cost: Option<ModelCost>,

    /// Token limits.
    #[serde(default)]
    pub limit: Option<ModelLimit>,

    /// Per-model provider override. When present, the `npm` field indicates
    /// that this model should be routed through a different SDK than the
    /// parent provider's top-level `npm` value.
    #[serde(default)]
    pub provider: Option<ModelProviderOverride>,
}

/// Per-model provider override from the models.dev spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProviderOverride {
    /// NPM package override, e.g. `"@ai-sdk/openai"` when the parent
    /// provider defaults to `"@ai-sdk/openai-compatible"`.
    #[serde(default)]
    pub npm: Option<String>,
}

/// Input/output modality declarations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

/// Cost per million tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    /// Input cost ($/M tokens).
    #[serde(default)]
    pub input: f64,
    /// Output cost ($/M tokens).
    #[serde(default)]
    pub output: f64,
    /// Cached read cost ($/M tokens), if supported.
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Cache write cost ($/M tokens), if supported.
    #[serde(default)]
    pub cache_write: Option<f64>,
}

/// Token limits for the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    /// Maximum context window size in tokens.
    #[serde(default)]
    pub context: u64,
    /// Maximum output tokens.
    #[serde(default)]
    pub output: u64,
}
