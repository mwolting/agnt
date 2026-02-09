//! Registry integration for OpenAI Codex.

use std::collections::HashMap;

use agnt_llm_openai::{OpenAIProviderBehavior, register_oauth_provider_with_behavior};
use agnt_llm_registry::{Modalities, ModelLimit, ModelSpec, OAuthPkceAuth, Registry};

pub const PROVIDER_ID: &str = "openai-codex";
pub const PROVIDER_NAME: &str = "OpenAI Codex";
pub const DEFAULT_MODEL_ID: &str = "gpt-5.2-codex";

/// Register the OpenAI Codex OAuth provider.
pub fn register(registry: &mut Registry) {
    let mut oauth = OAuthPkceAuth {
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
        authorize_url: "https://auth.openai.com/oauth/authorize".to_string(),
        token_url: "https://auth.openai.com/oauth/token".to_string(),
        redirect_url: "http://localhost:1455/auth/callback".to_string(),
        scopes: vec![
            "openid".to_string(),
            "profile".to_string(),
            "email".to_string(),
            "offline_access".to_string(),
        ],
        ..Default::default()
    };
    oauth
        .authorize_params
        .insert("id_token_add_organizations".to_string(), "true".to_string());
    oauth
        .authorize_params
        .insert("codex_cli_simplified_flow".to_string(), "true".to_string());
    oauth
        .authorize_params
        .insert("originator".to_string(), "pi".to_string());

    register_oauth_provider_with_behavior(
        registry,
        PROVIDER_ID,
        PROVIDER_NAME,
        oauth,
        codex_models(),
        Some("https://chatgpt.com/backend-api/codex".to_string()),
        codex_behavior(),
    );
}

fn codex_models() -> Vec<ModelSpec> {
    vec![
        codex_model("gpt-5.1", "GPT-5.1"),
        codex_model("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
        codex_model("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini"),
        codex_model("gpt-5.2", "GPT-5.2"),
        codex_model("gpt-5.2-codex", "GPT-5.2 Codex"),
        codex_model("gpt-5.3-codex", "GPT-5.3 Codex"),
    ]
}

fn codex_model(id: &str, name: &str) -> ModelSpec {
    ModelSpec {
        id: id.to_string(),
        name: Some(name.to_string()),
        family: None,
        attachment: true,
        reasoning: true,
        tool_call: true,
        structured_output: true,
        temperature: true,
        knowledge: None,
        release_date: None,
        last_updated: None,
        modalities: Some(Modalities {
            input: vec!["text".to_string(), "image".to_string()],
            output: vec!["text".to_string()],
        }),
        open_weights: false,
        cost: None,
        limit: Some(ModelLimit {
            context: 272_000,
            output: 128_000,
        }),
        provider: None,
    }
}

fn codex_behavior() -> OpenAIProviderBehavior {
    let mut headers = HashMap::new();
    headers.insert(
        "OpenAI-Beta".to_string(),
        "responses=experimental".to_string(),
    );
    headers.insert("originator".to_string(), "pi".to_string());
    OpenAIProviderBehavior {
        // Codex endpoint requires explicit store=false (ZDR mode).
        response_store: Some(false),
        include_reasoning_encrypted_content: true,
        include_chatgpt_account_id_header: true,
        extra_headers: headers,
    }
}
