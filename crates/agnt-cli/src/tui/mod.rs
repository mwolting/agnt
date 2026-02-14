pub mod app;
pub mod session_dialog;
pub mod ui;

use std::io;
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind,
    KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
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
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
}

pub async fn launch(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
        )
    )?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Event stream from crossterm
    let mut events = EventStream::new();

    // Main loop
    let result = run_loop(&mut terminal, app, &mut events).await;
    app.shutdown_background_workers().await;

    // Restore terminal
    restore_terminal();

    result
}
fn flush_pending_scroll(app: &mut App, pending_scroll_delta: &mut i32) {
    if *pending_scroll_delta != 0 {
        app.scroll_by(*pending_scroll_delta);
        *pending_scroll_delta = 0;
    }
}

fn handle_terminal_event(app: &mut App, event: Event, pending_scroll_delta: &mut i32) {
    match event {
        Event::Key(key) => {
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                flush_pending_scroll(app, pending_scroll_delta);
                app.handle_key(key);
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                *pending_scroll_delta = pending_scroll_delta.saturating_add(3);
            }
            MouseEventKind::ScrollDown => {
                *pending_scroll_delta = pending_scroll_delta.saturating_sub(3);
            }
            _ => {
                flush_pending_scroll(app, pending_scroll_delta);
                app.handle_mouse(mouse);
            }
        },
        Event::Resize(_, _) => {
            flush_pending_scroll(app, pending_scroll_delta);
        }
        _ => {
            flush_pending_scroll(app, pending_scroll_delta);
        }
    }
}

async fn handle_terminal_event_and_drain(
    app: &mut App,
    first_event: Event,
    events: &mut EventStream,
) {
    // Trackpads can emit long momentum bursts (especially on macOS).
    // Drain already-buffered events in one pass so stale scroll events don't
    // make the UI feel "stuck" at bounds.
    const MAX_DRAINED_EVENTS_PER_TICK: usize = 256;

    let mut pending_scroll_delta = 0;
    handle_terminal_event(app, first_event, &mut pending_scroll_delta);

    for _ in 0..MAX_DRAINED_EVENTS_PER_TICK {
        let Ok(next) = tokio::time::timeout(Duration::from_millis(0), events.next()).await else {
            break;
        };

        let Some(Ok(event)) = next else {
            break;
        };

        handle_terminal_event(app, event, &mut pending_scroll_delta);
    }

    flush_pending_scroll(app, &mut pending_scroll_delta);
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut blink_interval = tokio::time::interval(std::time::Duration::from_millis(530));
    blink_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let [mut command_typeahead_updates, mut mention_typeahead_updates] = app.typeahead_updates();
    let mut command_updates_open = true;
    let mut mention_updates_open = true;

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            Some(Ok(event)) = events.next() => {
                handle_terminal_event_and_drain(app, event, events).await;
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

            result = command_typeahead_updates.changed(), if command_updates_open => {
                if result.is_err() {
                    command_updates_open = false;
                }
            }

            result = mention_typeahead_updates.changed(), if mention_updates_open => {
                if result.is_err() {
                    mention_updates_open = false;
                }
            }
        }
    }
    Ok(())
}
