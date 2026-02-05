use agnt_core::{Agent, AgentEvent, AgentStream};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

// ---------------------------------------------------------------------------
// Display messages (what the UI renders)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: Role,
    pub content: String,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub enum AppState {
    Idle,
    Generating { stream: AgentStream },
}

pub struct App {
    pub agent: Agent,
    pub messages: Vec<DisplayMessage>,
    pub input: String,
    pub cursor_pos: usize,
    pub scroll_offset: u16,
    pub state: AppState,
    /// Accumulates streaming text for the current assistant response.
    pub current_response: String,
    pub should_quit: bool,
    /// Toggled by a timer to blink the streaming cursor.
    pub cursor_blink_on: bool,
    /// Maximum scroll offset (set by the renderer each frame).
    pub max_scroll: u16,
}

impl App {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent,
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            state: AppState::Idle,
            current_response: String::new(),
            should_quit: false,
            cursor_blink_on: true,
            max_scroll: 0,
        }
    }

    /// Handle a keyboard event. Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // Quit
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(self.state, AppState::Generating { .. }) {
                    // Cancel generation by dropping the stream
                    self.finalize_response();
                    self.state = AppState::Idle;
                } else {
                    self.should_quit = true;
                }
                true
            }

            // Submit
            KeyCode::Enter
                if !key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                if matches!(self.state, AppState::Idle) && !self.input.trim().is_empty() {
                    self.submit();
                }
                true
            }

            // Newline in input
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.insert_char('\n');
                true
            }

            // Escape → cancel if generating
            KeyCode::Esc => {
                if matches!(self.state, AppState::Generating { .. }) {
                    self.finalize_response();
                    self.state = AppState::Idle;
                }
                true
            }

            // Text input
            KeyCode::Char(c) => {
                self.insert_char(c);
                true
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                }
                true
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                true
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                }
                true
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                true
            }
            KeyCode::End => {
                self.cursor_pos = self.input.len();
                true
            }

            // Scroll history
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10).min(self.max_scroll);
                true
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                true
            }

            _ => false,
        }
    }

    /// Handle a mouse event.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(3).min(self.max_scroll);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
            }
            _ => {}
        }
    }

    /// Handle an agent event.
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::UserMessage { content } => {
                // Clear input now that the message is recorded in history
                self.input.clear();
                self.cursor_pos = 0;
                self.messages.push(DisplayMessage {
                    role: Role::User,
                    content,
                });
            }
            AgentEvent::TextDelta { delta } => {
                self.current_response.push_str(&delta);
                self.cursor_blink_on = true; // reset blink on new content
            }
            AgentEvent::ToolCallStart { display, .. } => {
                self.current_response
                    .push_str(&format!("\n[{}...]\n", display.title));
            }
            AgentEvent::ToolCallDone { display, .. } => {
                self.current_response
                    .push_str(&format!("[{}]\n", display.title));
            }
            AgentEvent::TurnComplete { .. } => {
                self.finalize_response();
                self.state = AppState::Idle;
            }
            AgentEvent::Error { error } => {
                self.current_response
                    .push_str(&format!("\n[error: {error}]"));
                self.finalize_response();
                self.state = AppState::Idle;
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        self.current_response.clear();
        // Input stays visible until UserMessage event confirms it's in history
        let stream = self.agent.submit(&text);
        self.state = AppState::Generating { stream };
    }

    fn finalize_response(&mut self) {
        if !self.current_response.is_empty() {
            self.messages.push(DisplayMessage {
                role: Role::Assistant,
                content: std::mem::take(&mut self.current_response),
            });
        }
    }

    pub fn toggle_cursor_blink(&mut self) {
        self.cursor_blink_on = !self.cursor_blink_on;
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }
}
