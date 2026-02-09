mod app;
mod gui;
mod ui;

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use agnt_auth::AuthManager;
use agnt_llm_registry::{AuthMethod, OAuthPkceAuth, Registry};
use app::{App, AppState};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use clap::{Parser, Subcommand};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use url::Url;

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

#[derive(Clone, Copy, Subcommand)]
enum Command {
    /// Start the terminal UI (default).
    Tui,
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

        match self.command.unwrap_or(Command::Tui) {
            Command::Tui => Mode::Tui,
            Command::Gui => Mode::Gui,
            Command::Providers => Mode::Providers,
        }
    }
}

/// Restore the terminal to its original state. Called on normal exit and
/// from the panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
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
        restore_terminal();
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

    if mode == Mode::Gui {
        ensure_provider_credentials(&registry, &auth_manager, DEFAULT_PROVIDER_ID).await?;
        let agent = build_default_agent(&mut registry)?;
        tokio::task::block_in_place(|| {
            gui::run(agent);
        });
        return Ok(());
    }

    run_tui(&mut registry, &auth_manager).await
}

fn print_providers(registry: &Registry) {
    for provider in registry.known_providers() {
        let status = if provider.configured {
            "configured"
        } else {
            "needs login"
        };
        let compat = if provider.compatible {
            "compatible"
        } else {
            "no-factory"
        };
        println!(
            "{} ({}) [{} | {} | {}]",
            provider.id, provider.name, provider.auth_method, status, compat
        );

        let mut models = registry.list_models(&provider.id);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        for model in &models {
            let name = model.name.as_deref().unwrap_or("");
            println!("  {:<30} {}", model.id, name);
        }
    }
}

async fn run_tui(
    registry: &mut Registry,
    auth_manager: &Arc<AuthManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_provider_credentials(registry, auth_manager, DEFAULT_PROVIDER_ID).await?;
    let agent = build_default_agent(registry)?;

    let mut app = App::new(agent);

    // Terminal setup
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Event stream from crossterm
    let mut events = EventStream::new();

    // Main loop
    let result = run(&mut terminal, &mut app, &mut events).await;

    // Restore terminal
    restore_terminal();

    result
}

fn build_default_agent(
    registry: &mut Registry,
) -> Result<agnt_core::Agent, Box<dyn std::error::Error>> {
    let model = registry.model(DEFAULT_PROVIDER_ID, DEFAULT_MODEL_ID)?;
    let cwd = std::env::current_dir()?;
    let mut agent = agnt_core::Agent::with_defaults(model, cwd);

    use agnt_llm_openai::{OpenAIRequestExt, ReasoningSummary};
    agent.configure_request(|req| {
        req.reasoning_summary(ReasoningSummary::Detailed);
    });

    Ok(agent)
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut blink_interval = tokio::time::interval(std::time::Duration::from_millis(530));
    blink_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Draw
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        // Wait for input, agent events, or blink tick
        tokio::select! {
            // Terminal events (keyboard, resize)
            Some(Ok(event)) = events.next() => {
                match event {
                    Event::Key(key) => {
                        app.handle_key(key);
                    }
                    Event::Mouse(mouse) => {
                        app.handle_mouse(mouse);
                    }
                    Event::Resize(_, _) => {
                        // Redraw handled by next loop iteration
                    }
                    _ => {}
                }
            }

            // Agent stream events (only when generating)
            Some(agent_event) = async {
                match &mut app.state {
                    AppState::Generating { stream } => stream.next().await,
                    AppState::Idle => std::future::pending().await,
                }
            } => {
                app.handle_agent_event(agent_event);
            }

            // Cursor blink tick (only matters during generation)
            _ = blink_interval.tick() => {
                if matches!(app.state, AppState::Generating { .. }) {
                    app.toggle_cursor_blink();
                }
            }
        }
    }
    Ok(())
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
