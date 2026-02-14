mod gui;
mod session;
mod tui;
mod typeahead;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agnt_auth::AuthManager;
use agnt_db::Session;
use agnt_llm_registry::{AuthMethod, OAuthPkceAuth, Registry};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use clap::{Parser, Subcommand};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::session::{SessionStore, SharedSessionStore};
use crate::tui::app::App;

const DEFAULT_PROVIDER_ID: &str = agnt_llm_codex::PROVIDER_ID;
const DEFAULT_MODEL_ID: &str = agnt_llm_codex::DEFAULT_MODEL_ID;
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const OAUTH_SUCCESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\" /><title>Authentication successful</title></head><body><p>Authentication successful. Return to your terminal.</p></body></html>";

#[derive(Parser)]
#[command(name = "agnt")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    // Backward-compatible alias for `agnt providers`.
    #[arg(long, hide = true)]
    providers: bool,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// Start the terminal UI (default).
    Tui {
        /// Run the TUI from this working directory.
        cwd: Option<PathBuf>,
    },
    /// Start the desktop GUI.
    Gui,
    /// List known providers and their models.
    Providers,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Tui,
    Gui,
    Providers,
}

impl Cli {
    fn mode(&self) -> Mode {
        if self.providers {
            return Mode::Providers;
        }

        match &self.command {
            Some(Command::Tui { .. }) | None => Mode::Tui,
            Some(Command::Gui) => Mode::Gui,
            Some(Command::Providers) => Mode::Providers,
        }
    }

    fn tui_cwd(&self) -> Option<&PathBuf> {
        match &self.command {
            Some(Command::Tui { cwd }) => cwd.as_ref(),
            _ => None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mode = cli.mode();

    let _ = dotenvy::dotenv();

    // Install a panic hook that restores the terminal before printing the
    // panic message, so the user isn't left with a broken terminal.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tui::restore_terminal();
        default_hook(info);
    }));

    // Set up auth + registry.
    let auth_manager = Arc::new(AuthManager::new("agnt"));
    let mut registry = Registry::new();
    registry.set_auth_resolver(auth_manager.resolver());
    agnt_llm_openai::register(&mut registry);
    agnt_llm_codex::register(&mut registry);
    registry.fetch_spec().await?;

    if mode == Mode::Providers {
        print_providers(&registry);
        return Ok(());
    }

    if mode == Mode::Tui
        && let Some(cwd) = cli.tui_cwd()
    {
        std::env::set_current_dir(cwd)?;
    }

    let cwd = std::env::current_dir()?;
    let mut session_store = SessionStore::open_for_project_root(&cwd)?;
    let existing_sessions = session_store.list_sessions(100)?;
    let selected_session = select_startup_session(mode, &existing_sessions)?;
    let restored_state = activate_selected_session(&mut session_store, selected_session)?;
    let session_store: SharedSessionStore = Arc::new(Mutex::new(session_store));

    if mode == Mode::Gui {
        ensure_provider_credentials(&registry, &auth_manager, DEFAULT_PROVIDER_ID).await?;
        let agent = build_default_agent(&mut registry, restored_state)?;
        gui::launch(agent, session_store);
        return Ok(());
    }

    ensure_provider_credentials(&registry, &auth_manager, DEFAULT_PROVIDER_ID).await?;
    let agent = build_default_agent(&mut registry, restored_state)?;
    let mut app = App::new(agent, session_store);
    tui::launch(&mut app).await
}

fn print_providers(registry: &Registry) {
    for provider in registry
        .known_providers()
        .into_iter()
        .filter(|provider| provider.configured)
    {
        let compat = if provider.compatible {
            "compatible"
        } else {
            "no-factory"
        };
        println!(
            "{} ({}) [{} | configured | {}]",
            provider.id, provider.name, provider.auth_method, compat
        );

        let mut models = registry.list_models(&provider.id);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        for model in &models {
            let name = model.name.as_deref().unwrap_or("");
            println!("  {:<30} {}", model.id, name);
        }
    }
}

enum StartupSessionChoice {
    Existing { session_id: String },
    New,
}

fn select_startup_session(
    mode: Mode,
    existing_sessions: &[Session],
) -> Result<StartupSessionChoice, Box<dyn std::error::Error>> {
    if existing_sessions.is_empty() {
        return Ok(StartupSessionChoice::New);
    }

    println!(
        "Found {} existing session(s). Select one to continue or create a new session:",
        existing_sessions.len()
    );
    for (idx, session) in existing_sessions.iter().enumerate() {
        let label = session_label(session);
        println!("  {}. {}", idx + 1, label);
    }
    println!("  n. Create new session");
    if mode == Mode::Gui {
        println!("  q. Quit");
    }

    loop {
        let input = prompt_line("Selection: ")?;
        let normalized = input.trim().to_lowercase();

        if normalized == "n" {
            return Ok(StartupSessionChoice::New);
        }
        if mode == Mode::Gui && normalized == "q" {
            std::process::exit(0);
        }

        if let Ok(n) = normalized.parse::<usize>()
            && n >= 1
            && n <= existing_sessions.len()
        {
            let session_id = existing_sessions[n - 1].id.clone();
            return Ok(StartupSessionChoice::Existing { session_id });
        }

        println!("Invalid selection.");
    }
}

fn activate_selected_session(
    store: &mut SessionStore,
    choice: StartupSessionChoice,
) -> Result<Option<agnt_core::ConversationState>, Box<dyn std::error::Error>> {
    match choice {
        StartupSessionChoice::Existing { session_id } => store.activate_session(&session_id),
        StartupSessionChoice::New => {
            store.create_session(None)?;
            Ok(None)
        }
    }
}

fn session_label(session: &Session) -> String {
    if let Some(title) = &session.title {
        return format!("{title} ({})", session.id);
    }
    format!("Session {}", session.id)
}

fn build_default_agent(
    registry: &mut Registry,
    restored_state: Option<agnt_core::ConversationState>,
) -> Result<agnt_core::Agent, Box<dyn std::error::Error>> {
    let model = registry.model(DEFAULT_PROVIDER_ID, DEFAULT_MODEL_ID)?;
    let cwd = std::env::current_dir()?;
    let mut agent = agnt_core::Agent::with_defaults(model, cwd);

    use agnt_llm_openai::{OpenAIRequestExt, ReasoningEffort, ReasoningSummary};
    agent.configure_request(|req| {
        req.reasoning_effort(ReasoningEffort::High);
        req.reasoning_summary(ReasoningSummary::Detailed);
    });

    if let Some(state) = restored_state {
        agent.restore_conversation_state(state);
    }

    Ok(agent)
}

async fn ensure_provider_credentials(
    registry: &Registry,
    auth: &Arc<AuthManager>,
    provider_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(request) = registry.auth_request(provider_id) else {
        return Ok(());
    };

    match request.auth_method {
        AuthMethod::ApiKey(_) => {
            if auth.resolve_cached(&request)?.is_some() {
                return Ok(());
            }

            let prompt = format!("Enter API key for {}: ", request.provider_name);
            let value = rpassword::prompt_password(prompt)?;
            if value.trim().is_empty() {
                return Err(format!("no API key provided for {}", request.provider_name).into());
            }
            auth.store_api_key(provider_id, value)?;
        }
        AuthMethod::OAuthPkce(ref config) => {
            match auth.refresh_oauth_if_needed(provider_id, config).await {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {}
                Err(err) => {
                    eprintln!(
                        "stored OAuth session for {} is not usable ({}); starting sign-in flow",
                        request.provider_name, err
                    );
                }
            }

            let pending = auth.begin_oauth(provider_id, config)?;
            println!(
                "Sign in for {}:\n{}",
                request.provider_name, pending.authorize_url
            );
            if let Err(err) = webbrowser::open(&pending.authorize_url) {
                eprintln!("failed to open browser: {err}");
            }

            let authorization_input = match wait_for_oauth_callback(config, &pending.state).await? {
                Some(code) => code,
                None => prompt_line("Paste authorization code (or redirect URL): ")?,
            };
            auth.complete_oauth(provider_id, config, &pending, &authorization_input)
                .await?;
        }
    }

    Ok(())
}

#[derive(Clone)]
struct CallbackState {
    expected_path: String,
    expected_state: String,
    tx: mpsc::UnboundedSender<Result<String, String>>,
}

async fn wait_for_oauth_callback(
    config: &OAuthPkceAuth,
    expected_state: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let redirect = Url::parse(&config.redirect_url)?;
    if redirect.scheme() != "http" {
        return Ok(None);
    }

    let host = redirect.host_str().unwrap_or("127.0.0.1");
    if host != "127.0.0.1" && host != "localhost" {
        return Ok(None);
    }
    let port = redirect.port_or_known_default().unwrap_or(80);
    let bind_host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let expected_path = redirect.path().to_string();

    let listener = match tokio::net::TcpListener::bind((bind_host, port)).await {
        Ok(listener) => listener,
        Err(_) => return Ok(None),
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<Result<String, String>>();
    let state = CallbackState {
        expected_path,
        expected_state: expected_state.to_string(),
        tx,
    };

    let app = Router::new()
        .route("/", get(oauth_callback))
        .route("/{*path}", get(oauth_callback))
        .with_state(state);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
    });
    let server_task = tokio::spawn(async move {
        let _ = server.await;
    });

    let result = tokio::time::timeout(OAUTH_CALLBACK_TIMEOUT, rx.recv()).await;
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(2), server_task).await;

    match result {
        Ok(Some(Ok(code))) => Ok(Some(code)),
        Ok(Some(Err(message))) => Err(message.into()),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

async fn oauth_callback(
    State(state): State<CallbackState>,
    Query(query): Query<HashMap<String, String>>,
    uri: Uri,
) -> impl IntoResponse {
    if uri.path() != state.expected_path {
        return (StatusCode::NOT_FOUND, Html("Not found".to_string())).into_response();
    }

    if query.get("state").map(String::as_str) != Some(state.expected_state.as_str()) {
        let _ = state
            .tx
            .send(Err("oauth callback state mismatch".to_string()));
        return (StatusCode::BAD_REQUEST, Html("State mismatch".to_string())).into_response();
    }

    let Some(code) = query.get("code").cloned() else {
        let _ = state
            .tx
            .send(Err("missing authorization code in callback".to_string()));
        return (
            StatusCode::BAD_REQUEST,
            Html("Missing authorization code".to_string()),
        )
            .into_response();
    };

    let _ = state.tx.send(Ok(code));
    (StatusCode::OK, Html(OAUTH_SUCCESS_HTML.to_string())).into_response()
}

fn prompt_line(prompt: &str) -> Result<String, io::Error> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
