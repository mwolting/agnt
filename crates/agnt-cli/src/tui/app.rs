use agnt_core::{Agent, AgentEvent, AgentStream, ConversationState};
use agnt_llm::{AssistantPart, Message, UserPart};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::session::SharedSessionStore;
use crate::tui::typeahead::{ActiveTypeahead, TypeaheadActivation, TypeaheadState};
use crate::typeahead::{Command, Mention};

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
    pub session_store: SharedSessionStore,
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
    typeahead: TypeaheadState,
}

impl App {
    pub fn new(agent: Agent, session_store: SharedSessionStore) -> Self {
        Self {
            messages: display_messages_from_history(&agent.messages()),
            agent,
            session_store,
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            state: AppState::Idle,
            stream_chunks: Vec::new(),
            should_quit: false,
            cursor_blink_on: true,
            max_scroll: 0,
            typeahead: TypeaheadState::new_for_current_project(),
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
                if let Some(activation) = self
                    .typeahead
                    .activate_selected(&self.input, self.cursor_pos)
                {
                    self.apply_typeahead_activation(activation);
                    return true;
                }
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
                } else {
                    self.typeahead.dismiss(&self.input, self.cursor_pos);
                }
                true
            }

            // Text input
            KeyCode::Char(c) => {
                self.insert_char(c);
                self.typeahead.sync(&self.input, self.cursor_pos);
                true
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                    self.typeahead.sync(&self.input, self.cursor_pos);
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                    self.typeahead.sync(&self.input, self.cursor_pos);
                }
                true
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.typeahead.sync(&self.input, self.cursor_pos);
                }
                true
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                    self.typeahead.sync(&self.input, self.cursor_pos);
                }
                true
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                self.typeahead.sync(&self.input, self.cursor_pos);
                true
            }
            KeyCode::End => {
                self.cursor_pos = self.input.len();
                self.typeahead.sync(&self.input, self.cursor_pos);
                true
            }
            KeyCode::Up => {
                self.typeahead
                    .move_selection(-1, &self.input, self.cursor_pos);
                true
            }
            KeyCode::Down => {
                self.typeahead
                    .move_selection(1, &self.input, self.cursor_pos);
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
                self.typeahead.sync(&self.input, self.cursor_pos);
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
            AgentEvent::TurnComplete { usage } => {
                if let Err(err) = self
                    .session_store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .persist_turn_from_agent(&self.agent, &usage)
                {
                    self.stream_chunks
                        .push(StreamChunk::Tool(format!("[session save error: {err}]")));
                }
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

    pub fn typeahead_matches(&mut self) -> Option<ActiveTypeahead> {
        self.typeahead.visible_matches(&self.input, self.cursor_pos)
    }

    pub fn typeahead_selected_index(&self) -> usize {
        self.typeahead.selected_index()
    }

    pub fn should_poll_typeahead(&self) -> bool {
        self.typeahead
            .has_background_work(&self.input, self.cursor_pos)
    }

    fn apply_typeahead_activation(&mut self, activation: TypeaheadActivation) {
        match activation {
            TypeaheadActivation::Mention {
                mention,
                token_start,
                token_end,
            } => self.apply_mention(mention, token_start, token_end),
            TypeaheadActivation::Command { command, .. } => self.run_command(command),
        }
    }

    fn apply_mention(&mut self, mention: Mention, token_start: usize, token_end: usize) {
        let mention_text = match mention {
            Mention::File(path) => path.to_string_lossy().replace('\\', "/"),
        };
        let replacement = format!("{mention_text} ");
        self.input
            .replace_range(token_start..token_end, &replacement);
        self.cursor_pos = token_start + replacement.len();
        self.typeahead.sync(&self.input, self.cursor_pos);
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::NewSession => self.start_new_session(),
        }
    }

    fn start_new_session(&mut self) {
        if matches!(self.state, AppState::Generating { .. }) {
            self.finalize_response();
            self.state = AppState::Idle;
        }

        match self
            .session_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create_session(None)
        {
            Ok(_) => {
                self.agent.restore_conversation_state(ConversationState {
                    messages: Vec::new(),
                });
                self.messages.clear();
                self.stream_chunks.clear();
                self.input.clear();
                self.cursor_pos = 0;
                self.scroll_offset = 0;
                self.max_scroll = 0;
                self.typeahead.sync(&self.input, self.cursor_pos);
            }
            Err(err) => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[session error: {err}]")));
            }
        }
    }
}

pub fn display_messages_from_history(messages: &[Message]) -> Vec<DisplayMessage> {
    let mut out = Vec::new();

    for message in messages {
        match message {
            Message::User { parts } => {
                let mut chunks = Vec::new();
                for part in parts {
                    match part {
                        UserPart::Text(text) => {
                            if !text.text.is_empty() {
                                chunks.push(StreamChunk::Text(text.text.clone()));
                            }
                        }
                        UserPart::Image(image) => {
                            chunks.push(StreamChunk::Text(format!("[image: {}]", image.url)));
                        }
                    }
                }
                if !chunks.is_empty() {
                    out.push(DisplayMessage {
                        role: Role::User,
                        chunks,
                    });
                }
            }
            Message::Assistant { parts } => {
                let mut chunks = Vec::new();
                for part in parts {
                    match part {
                        AssistantPart::Text(text) => {
                            if !text.text.is_empty() {
                                chunks.push(StreamChunk::Text(text.text.clone()));
                            }
                        }
                        AssistantPart::Reasoning(reasoning) => {
                            if let Some(text) = &reasoning.text
                                && !text.is_empty()
                            {
                                chunks.push(StreamChunk::Reasoning(text.clone()));
                            }
                        }
                        AssistantPart::ToolCall(call) => {
                            if let Some(display) = &call.display {
                                chunks.push(StreamChunk::Tool(format!("[{}...]", display.title)));
                                if let Some(result) = &display.result {
                                    chunks.push(StreamChunk::Tool(format!("[{}]", result.title)));
                                }
                            } else {
                                chunks.push(StreamChunk::Tool(format!("[tool: {}...]", call.name)));
                            }
                        }
                    }
                }
                if !chunks.is_empty() {
                    out.push(DisplayMessage {
                        role: Role::Assistant,
                        chunks,
                    });
                }
            }
            Message::System { .. } | Message::Tool { .. } => {}
        }
    }

    out
}
