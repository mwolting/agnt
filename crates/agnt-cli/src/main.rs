mod app;
mod ui;

use app::{App, AppState};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use tokio_stream::StreamExt;

/// Restore the terminal to its original state. Called on normal exit and
/// from the panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    // Install a panic hook that restores the terminal before printing the
    // panic message, so the user isn't left with a broken terminal.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // Set up the model and agent
    let provider = agnt_llm_openai::from_env();
    let model = provider.model("gpt-5-nano");

    let cwd = std::env::current_dir()?;
    let agent = agnt_core::Agent::with_defaults(model, cwd);

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
