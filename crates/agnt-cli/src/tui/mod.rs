pub mod app;
pub mod typeahead;
pub mod ui;

use std::io;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio_stream::StreamExt;

use crate::tui::app::{App, AppState};

/// Restore the terminal to its original state. Called on normal exit and
/// from the panic hook.
pub fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

pub async fn launch(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Event stream from crossterm
    let mut events = EventStream::new();

    // Main loop
    let result = run_loop(&mut terminal, app, &mut events).await;

    // Restore terminal
    restore_terminal();

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut blink_interval = tokio::time::interval(std::time::Duration::from_millis(530));
    blink_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut typeahead_poll_interval = tokio::time::interval(std::time::Duration::from_millis(50));
    typeahead_poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            Some(Ok(event)) = events.next() => {
                match event {
                    Event::Key(key) => {
                        app.handle_key(key);
                    }
                    Event::Mouse(mouse) => {
                        app.handle_mouse(mouse);
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }

            Some(agent_event) = async {
                match &mut app.state {
                    AppState::Generating { stream } => stream.next().await,
                    AppState::Idle => std::future::pending().await,
                }
            } => {
                app.handle_agent_event(agent_event);
            }

            _ = blink_interval.tick() => {
                if matches!(app.state, AppState::Generating { .. }) {
                    app.toggle_cursor_blink();
                }
            }

            _ = typeahead_poll_interval.tick(), if app.should_poll_typeahead() => {}
        }
    }
    Ok(())
}
