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
    pub chunks: Vec<StreamChunk>,
}

/// A typed chunk in the streaming assistant response, preserving
/// the natural ordering of reasoning, text, and tool calls.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Model reasoning/thinking text (rendered dimmed/italic).
    Reasoning(String),
    /// Regular assistant text.
    Text(String),
    /// Tool call status line (e.g. "[Read src/main.rs...]" or "[Read src/main.rs]").
    Tool(String),
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
    /// Streaming assistant response as an ordered list of typed chunks.
    pub stream_chunks: Vec<StreamChunk>,
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
            stream_chunks: Vec::new(),
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
                    chunks: vec![StreamChunk::Text(content)],
                });
            }
            AgentEvent::TextDelta { delta } => {
                // Append to the last Text chunk, or start a new one.
                if let Some(StreamChunk::Text(s)) = self.stream_chunks.last_mut() {
                    s.push_str(&delta);
                } else {
                    self.stream_chunks.push(StreamChunk::Text(delta));
                }
                self.cursor_blink_on = true;
            }
            AgentEvent::ReasoningDelta { delta } => {
                // Append to the last Reasoning chunk, or start a new one.
                if let Some(StreamChunk::Reasoning(s)) = self.stream_chunks.last_mut() {
                    s.push_str(&delta);
                } else {
                    self.stream_chunks.push(StreamChunk::Reasoning(delta));
                }
                self.cursor_blink_on = true;
            }
            AgentEvent::ToolCallStart { display, .. } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}...]", display.title)));
            }
            AgentEvent::ToolCallDone { display, .. } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}]", display.title)));
            }
            AgentEvent::TurnComplete { .. } => {
                self.finalize_response();
                self.state = AppState::Idle;
            }
            AgentEvent::Error { error } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[error: {error}]")));
                self.finalize_response();
                self.state = AppState::Idle;
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        self.stream_chunks.clear();
        // Input stays visible until UserMessage event confirms it's in history
        let stream = self.agent.submit(&text);
        self.state = AppState::Generating { stream };
    }

    fn finalize_response(&mut self) {
        let chunks = std::mem::take(&mut self.stream_chunks);
        if !chunks.is_empty() {
            self.messages.push(DisplayMessage {
                role: Role::Assistant,
                chunks,
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
