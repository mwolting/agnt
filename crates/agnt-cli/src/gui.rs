use std::{collections::HashMap, time::Duration};

use agnt_core::{Agent, AgentEvent, ConversationState, DisplayBody};
use gpui::{
    AnyElement, App as GpuiApp, AppContext, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, KeyBinding, ListAlignment, ListState, ParentElement, Pixels, Render,
    ScrollWheelEvent, Styled, Subscription, Task, Window, WindowOptions, div, list, point, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Root, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{
        Enter as InputEnter, Escape as InputEscape, Input, InputEvent, InputState,
        MoveDown as InputMoveDown, MoveUp as InputMoveUp, Position,
    },
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState},
    v_flex,
};

use crate::session::SharedSessionStore;
use crate::tui::app::{DisplayMessage, Role, StreamChunk, display_messages_from_history};
use crate::typeahead::{Command, Mention, TypeaheadActivation};

mod session_dialog;
mod typeahead;
use session_dialog::ResumeDialogState;
use session_dialog::{build_dialog_entries, move_selection, selected_session_id};
use typeahead::GuiTypeahead;

#[derive(Clone, Copy)]
enum ThreadBlockKind {
    UserLabel,
    AssistantLabel,
    Markdown,
    ReasoningMarkdown,
    StreamingMarkdown,
    StreamingReasoning,
    Tool,
    Cursor,
    Hint,
    Spacer,
}

#[derive(Clone)]
struct ThreadBlock {
    kind: ThreadBlockKind,
    text: String,
    markdown_state: Option<Entity<TextViewState>>,
    markdown_id: Option<String>,
    min_height: Option<Pixels>,
}

struct AgntGui {
    agent: Agent,
    session_store: SharedSessionStore,
    input: Entity<InputState>,
    typeahead: GuiTypeahead,
    thread_list: ListState,
    messages: Vec<DisplayMessage>,
    message_markdown_states: Vec<Vec<Option<Entity<TextViewState>>>>,
    stream_chunks: Vec<StreamChunk>,
    stream_markdown_states: Vec<Option<Entity<TextViewState>>>,
    stream_block_height_floors: HashMap<String, Pixels>,
    generating: bool,
    cursor_blink_on: bool,
    stick_to_bottom: bool,
    resume_dialog: Option<ResumeDialogState>,
    stream_task: Task<()>,
    _blink_task: Task<()>,
    _typeahead_updates_task: Task<()>,
    _input_subscription: Subscription,
    markdown_remeasure_scheduled: bool,
    _markdown_remeasure_task: Task<()>,
    _markdown_state_subscriptions: Vec<Subscription>,
}

impl AgntGui {
    fn new(
        agent: Agent,
        session_store: SharedSessionStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let messages = display_messages_from_history(&agent.messages());
        let message_markdown_states = Self::build_markdown_states(&messages, cx);
        let typeahead = GuiTypeahead::new_for_current_project();
        let [mut command_typeahead_updates, mut mention_typeahead_updates] = typeahead.updates();

        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("Type a message...")
        });
        let input_subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        let blink_task = cx.spawn_in(window, async move |this, window| {
            loop {
                window
                    .background_executor()
                    .timer(Duration::from_millis(530))
                    .await;

                if this
                    .update_in(window, |this, _, cx| {
                        if this.generating {
                            this.cursor_blink_on = !this.cursor_blink_on;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let typeahead_updates_task = cx.spawn_in(window, async move |this, window| {
            let mut command_updates_open = true;
            let mut mention_updates_open = true;
            loop {
                tokio::select! {
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
                    else => break,
                }

                if this
                    .update_in(window, |_, _, cx| {
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        input.update(cx, |input, cx| input.focus(window, cx));

        let mut this = Self {
            agent,
            session_store,
            input,
            typeahead,
            thread_list: ListState::new(0, ListAlignment::Top, px(512.)).measure_all(),
            messages,
            message_markdown_states,
            stream_chunks: Vec::new(),
            stream_markdown_states: Vec::new(),
            stream_block_height_floors: HashMap::new(),
            generating: false,
            cursor_blink_on: true,
            stick_to_bottom: true,
            resume_dialog: None,
            stream_task: Task::ready(()),
            _blink_task: blink_task,
            _typeahead_updates_task: typeahead_updates_task,
            _input_subscription: input_subscription,
            markdown_remeasure_scheduled: false,
            _markdown_remeasure_task: Task::ready(()),
            _markdown_state_subscriptions: Vec::new(),
        };

        this.thread_list.reset(this.build_thread_blocks().len());
        this.rebuild_markdown_state_subscriptions(cx);
        this
    }

    fn build_markdown_states(
        messages: &[DisplayMessage],
        cx: &mut Context<Self>,
    ) -> Vec<Vec<Option<Entity<TextViewState>>>> {
        let mut all_states = Vec::with_capacity(messages.len());
        for message in messages {
            all_states.push(Self::build_markdown_states_for_chunks(&message.chunks, cx));
        }
        all_states
    }

    fn build_markdown_states_for_chunks(
        chunks: &[StreamChunk],
        cx: &mut Context<Self>,
    ) -> Vec<Option<Entity<TextViewState>>> {
        let mut states = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let state = match chunk {
                StreamChunk::Text(text) | StreamChunk::Reasoning(text) => {
                    let text = text.clone();
                    Some(cx.new(move |cx| TextViewState::markdown(&text, cx)))
                }
                StreamChunk::Tool(_) => None,
            };
            states.push(state);
        }
        states
    }

    fn request_thread_remeasure(&mut self, cx: &mut Context<Self>) {
        if self.markdown_remeasure_scheduled {
            return;
        }

        self.markdown_remeasure_scheduled = true;
        let delay = if self.generating {
            Duration::from_millis(120)
        } else {
            Duration::from_millis(40)
        };

        self._markdown_remeasure_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;

            let _ = this.update(cx, |this, cx| {
                this.markdown_remeasure_scheduled = false;
                this.thread_list.remeasure();
                this.maybe_auto_scroll_to_bottom();
                cx.notify();
            });
        });
    }
    fn rebuild_markdown_state_subscriptions(&mut self, cx: &mut Context<Self>) {
        let mut subscriptions = Vec::new();

        for state in self
            .message_markdown_states
            .iter()
            .flat_map(|states| states.iter())
            .chain(self.stream_markdown_states.iter())
            .filter_map(|state| state.as_ref())
        {
            subscriptions.push(cx.observe(state, |this, _, cx| {
                this.request_thread_remeasure(cx);
            }));
        }

        self._markdown_state_subscriptions = subscriptions;
    }

    fn on_input_event(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // In multiline mode Enter inserts a newline first; we submit right after and trim.
            InputEvent::PressEnter { secondary: false } => {
                self.submit_from_input(state, window, cx)
            }
            // Secondary enter (Shift+Enter on supported platforms) keeps newline for multiline input.
            InputEvent::PressEnter { secondary: true } => {}
            _ => {}
        }
    }

    fn on_send_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.input.clone();
        self.submit_from_input(&state, window, cx);
    }

    fn on_typeahead_enter_capture(
        &mut self,
        action: &InputEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.secondary {
            return;
        }

        if self.resume_dialog.is_some() {
            self.confirm_resume_selection(window, cx);
            cx.stop_propagation();
            return;
        }

        let (input, cursor_pos) = self.input_snapshot(cx);
        if let Some(activation) = self.typeahead.activate_selected(&input, cursor_pos) {
            self.apply_typeahead_activation(activation, window, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_typeahead_escape_capture(
        &mut self,
        _: &InputEscape,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resume_dialog.is_some() {
            self.resume_dialog = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let (input, cursor_pos) = self.input_snapshot(cx);
        if self.typeahead.dismiss_if_visible(&input, cursor_pos) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_typeahead_up_capture(
        &mut self,
        _: &InputMoveUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.resume_dialog.as_mut() {
            move_selection(dialog, -1);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let (input, cursor_pos) = self.input_snapshot(cx);
        if self.typeahead.move_if_visible(-1, &input, cursor_pos) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_typeahead_down_capture(
        &mut self,
        _: &InputMoveDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.resume_dialog.as_mut() {
            move_selection(dialog, 1);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let (input, cursor_pos) = self.input_snapshot(cx);
        if self.typeahead.move_if_visible(1, &input, cursor_pos) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_thread_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let delta_y = event.delta.pixel_delta(window.line_height()).y;
        if delta_y > px(0.) {
            self.stick_to_bottom = false;
        } else if delta_y < px(0.) && self.distance_from_bottom() <= px(24.) {
            self.stick_to_bottom = true;
        }
    }

    fn distance_from_bottom(&self) -> Pixels {
        let offset = self.thread_list.scroll_px_offset_for_scrollbar().y;
        let max = self.thread_list.max_offset_for_scrollbar().height;
        (max + offset).abs()
    }

    fn maybe_auto_scroll_to_bottom(&mut self) {
        if !self.stick_to_bottom && self.distance_from_bottom() <= px(24.) {
            self.stick_to_bottom = true;
        }

        if self.stick_to_bottom {
            let max = self.thread_list.max_offset_for_scrollbar().height;
            self.thread_list
                .set_offset_from_scrollbar(point(px(0.), -max));
        }
    }

    fn submit_from_input(
        &mut self,
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generating || self.resume_dialog.is_some() {
            return;
        }

        let text = state.read(cx).value();
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }

        let ensure_session_result = self.session_store.lock().ensure_active_session();
        if let Err(err) = ensure_session_result {
            self.stream_chunks
                .push(StreamChunk::Tool(format!("[session error: {err}]")));
            self.stream_markdown_states.push(None);
            self.maybe_auto_scroll_to_bottom();
            cx.notify();
            return;
        }

        state.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });

        self.start_stream(text, window, cx);
    }

    fn start_stream(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.stream_chunks.clear();
        self.stream_markdown_states.clear();
        self.stream_block_height_floors.clear();
        self.generating = true;
        self.cursor_blink_on = true;
        cx.notify();

        let mut stream = self.agent.submit(&text);
        self.stream_task = cx.spawn_in(window, async move |this, window| {
            while let Some(event) = stream.next().await {
                let finished = this
                    .update_in(window, |this, window, cx| {
                        this.handle_agent_event(event, window, cx);
                        !this.generating
                    })
                    .unwrap_or(true);

                if finished {
                    return;
                }
            }

            _ = this.update_in(window, |this, _, cx| {
                if this.generating {
                    this.finalize_response(cx);
                    this.generating = false;
                    cx.notify();
                }
            });
        });
    }

    fn handle_agent_event(
        &mut self,
        event: AgentEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut markdown_states_changed = false;
        match event {
            AgentEvent::UserMessage { content } => {
                self.messages.push(DisplayMessage {
                    role: Role::User,
                    chunks: vec![StreamChunk::Text(content)],
                });
                let state = cx.new(|cx| TextViewState::markdown("", cx));
                state.update(cx, |state, cx| {
                    if let Some(StreamChunk::Text(content)) =
                        self.messages.last().and_then(|m| m.chunks.first())
                    {
                        state.set_text(content, cx);
                    }
                });
                self.message_markdown_states.push(vec![Some(state)]);
                markdown_states_changed = true;
            }
            AgentEvent::TextDelta { delta } => {
                if let Some(StreamChunk::Text(s)) = self.stream_chunks.last_mut() {
                    s.push_str(&delta);
                    if let Some(Some(state)) = self.stream_markdown_states.last() {
                        state.update(cx, |state, cx| state.push_str(&delta, cx));
                    }
                } else {
                    self.stream_chunks.push(StreamChunk::Text(delta.clone()));
                    let state = cx.new(|cx| TextViewState::markdown(&delta, cx));
                    self.stream_markdown_states.push(Some(state));
                    markdown_states_changed = true;
                }
            }
            AgentEvent::ReasoningDelta { delta } => {
                if let Some(StreamChunk::Reasoning(s)) = self.stream_chunks.last_mut() {
                    s.push_str(&delta);
                    if let Some(Some(state)) = self.stream_markdown_states.last() {
                        state.update(cx, |state, cx| state.push_str(&delta, cx));
                    }
                } else {
                    self.stream_chunks
                        .push(StreamChunk::Reasoning(delta.clone()));
                    let state = cx.new(|cx| TextViewState::markdown(&delta, cx));
                    self.stream_markdown_states.push(Some(state));
                    markdown_states_changed = true;
                }
            }
            AgentEvent::ToolCallStart { display, .. } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}...]", display.title)));
                self.stream_markdown_states.push(None);
            }
            AgentEvent::ToolCallDone { display, .. } => {
                let diff = diff_from_display_body(display.body.as_ref());
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}]", display.title)));
                self.stream_markdown_states.push(None);
                if let Some(diff) = diff {
                    push_tool_diff_chunks(
                        &mut self.stream_chunks,
                        &mut self.stream_markdown_states,
                        diff,
                    );
                }
            }
            AgentEvent::TurnComplete { usage } => {
                if let Err(err) = self
                    .session_store
                    .lock()
                    .persist_turn_from_agent(&self.agent, &usage)
                {
                    self.stream_chunks
                        .push(StreamChunk::Tool(format!("[session save error: {err}]")));
                    self.stream_markdown_states.push(None);
                }
                self.finalize_response(cx);
                self.generating = false;
            }
            AgentEvent::Error { error } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[error: {error}]")));
                self.stream_markdown_states.push(None);
                self.finalize_response(cx);
                self.generating = false;
            }
        }

        if markdown_states_changed {
            self.rebuild_markdown_state_subscriptions(cx);
        }
        self.maybe_auto_scroll_to_bottom();
        cx.notify();
    }

    fn finalize_response(&mut self, cx: &mut Context<Self>) {
        let chunks = std::mem::take(&mut self.stream_chunks);
        let states = std::mem::take(&mut self.stream_markdown_states);
        self.stream_block_height_floors.clear();
        if !chunks.is_empty() {
            self.messages.push(DisplayMessage {
                role: Role::Assistant,
                chunks,
            });
            self.message_markdown_states.push(states);
            self.rebuild_markdown_state_subscriptions(cx);
        }
    }

    fn apply_typeahead_activation(
        &mut self,
        activation: TypeaheadActivation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match activation {
            TypeaheadActivation::Mention {
                mention,
                token_start,
                token_end,
            } => self.apply_mention(mention, token_start, token_end, window, cx),
            TypeaheadActivation::Command { command, .. } => self.run_command(command, window, cx),
        }
    }

    fn apply_mention(
        &mut self,
        mention: Mention,
        token_start: usize,
        token_end: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mention_text = match mention {
            Mention::File(path) => path.to_string_lossy().replace('\\', "/"),
        };
        let replacement = format!("{mention_text} ");
        let (mut input, _) = self.input_snapshot(cx);
        if token_start > token_end || token_end > input.len() {
            return;
        }
        input.replace_range(token_start..token_end, &replacement);
        let cursor_pos = token_start + replacement.len();
        self.set_input_text_and_cursor(input, cursor_pos, window, cx);
    }

    fn run_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::NewSession => self.start_new_session(window, cx),
            Command::ResumeSession => self.open_resume_dialog(cx),
        }
    }

    fn start_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generating {
            self.finalize_response(cx);
            self.generating = false;
        }

        self.session_store.lock().clear_active_session();
        self.restore_active_session_state(None, window, cx);

        self.maybe_auto_scroll_to_bottom();
        cx.notify();
    }

    fn open_resume_dialog(&mut self, cx: &mut Context<Self>) {
        if self.generating {
            self.finalize_response(cx);
            self.generating = false;
        }

        let (active_session_id, sessions_result) = {
            let store = self.session_store.lock();
            (
                store.active_session_id().map(str::to_owned),
                store.list_sessions(100),
            )
        };

        match sessions_result {
            Ok(mut sessions) => {
                if let Some(active_session_id) = active_session_id {
                    sessions.retain(|session| session.id != active_session_id);
                }

                if sessions.is_empty() {
                    self.stream_chunks.push(StreamChunk::Tool(
                        "[no previous sessions to resume]".to_string(),
                    ));
                    self.stream_markdown_states.push(None);
                } else {
                    self.resume_dialog =
                        Some(ResumeDialogState::new(build_dialog_entries(sessions)));
                }
            }
            Err(err) => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[session error: {err}]")));
                self.stream_markdown_states.push(None);
            }
        }

        self.maybe_auto_scroll_to_bottom();
        cx.notify();
    }

    fn confirm_resume_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.resume_dialog.take() else {
            return;
        };
        let Some(session_id) = selected_session_id(&dialog).map(str::to_owned) else {
            return;
        };

        let activate_result = self.session_store.lock().activate_session(&session_id);
        match activate_result {
            Ok(restored_state) => self.restore_active_session_state(restored_state, window, cx),
            Err(err) => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[session error: {err}]")));
                self.stream_markdown_states.push(None);
            }
        }

        self.maybe_auto_scroll_to_bottom();
        cx.notify();
    }

    fn restore_active_session_state(
        &mut self,
        restored_state: Option<ConversationState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent
            .restore_conversation_state(restored_state.unwrap_or_else(|| ConversationState {
                messages: Vec::new(),
            }));
        self.messages = display_messages_from_history(&self.agent.messages());
        self.message_markdown_states = Self::build_markdown_states(&self.messages, cx);
        self.stream_chunks.clear();
        self.stream_markdown_states.clear();
        self.stream_block_height_floors.clear();
        self.cursor_blink_on = true;
        self.stick_to_bottom = true;
        self.resume_dialog = None;
        self.thread_list.reset(self.build_thread_blocks().len());
        self.rebuild_markdown_state_subscriptions(cx);
        self.set_input_text_and_cursor(String::new(), 0, window, cx);
    }

    fn input_snapshot(&self, cx: &Context<Self>) -> (String, usize) {
        let input = self.input.read(cx);
        (input.value().to_string(), input.cursor())
    }

    fn set_input_text_and_cursor(
        &mut self,
        text: String,
        cursor_pos: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cursor_pos = cursor_pos.min(text.len());
        let position = byte_offset_to_position(&text, cursor_pos);
        self.input.update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            input.set_cursor_position(position, window, cx);
            input.focus(window, cx);
        });
    }

    fn render_typeahead_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.resume_dialog.is_some() {
            return None;
        }

        let (input, cursor_pos) = self.input_snapshot(cx);
        self.typeahead.render_panel(&input, cursor_pos, cx)
    }

    fn render_resume_dialog_panel(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.resume_dialog.as_ref()?;
        let max_items = 8usize;
        let start = if dialog.selected_index >= max_items {
            dialog.selected_index + 1 - max_items
        } else {
            0
        };
        let end = (start + max_items).min(dialog.entries.len());

        let mut panel = v_flex()
            .w_full()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .bg(cx.theme().muted)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Resume session"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Enter to resume, Esc to cancel"),
            );

        for idx in start..end {
            let entry = &dialog.entries[idx];
            let marker = if idx == dialog.selected_index {
                "› "
            } else {
                "  "
            };
            let mut row = div()
                .w_full()
                .h_5()
                .px_1()
                .flex()
                .items_center()
                .text_sm()
                .child(format!("{marker}{}", entry.label));
            if idx == dialog.selected_index {
                row = row.text_color(cx.theme().cyan);
            } else {
                row = row.text_color(cx.theme().foreground);
            }
            panel = panel.child(row);
        }

        Some(panel.into_any_element())
    }

    fn build_thread_blocks(&self) -> Vec<ThreadBlock> {
        let mut blocks = Vec::new();

        for (msg_ix, msg) in self.messages.iter().enumerate() {
            if !blocks.is_empty() {
                blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Spacer,
                    text: String::new(),
                    markdown_state: None,
                    markdown_id: None,
                    min_height: None,
                });
            }

            let label_kind = match msg.role {
                Role::User => ThreadBlockKind::UserLabel,
                Role::Assistant => ThreadBlockKind::AssistantLabel,
            };
            blocks.push(ThreadBlock {
                kind: label_kind,
                text: match msg.role {
                    Role::User => "You".to_string(),
                    Role::Assistant => "Assistant".to_string(),
                },
                markdown_state: None,
                markdown_id: None,
                min_height: None,
            });

            let states = self.message_markdown_states.get(msg_ix);
            Self::append_chunk_blocks(
                &msg.chunks,
                states,
                &format!("msg-{msg_ix}"),
                false,
                &mut blocks,
            );
        }

        if self.generating || !self.stream_chunks.is_empty() {
            if !blocks.is_empty() {
                blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Spacer,
                    text: String::new(),
                    markdown_state: None,
                    markdown_id: None,
                    min_height: None,
                });
            }

            blocks.push(ThreadBlock {
                kind: ThreadBlockKind::AssistantLabel,
                text: "Assistant".to_string(),
                markdown_state: None,
                markdown_id: None,
                min_height: None,
            });

            Self::append_chunk_blocks(
                &self.stream_chunks,
                Some(&self.stream_markdown_states),
                "stream",
                true,
                &mut blocks,
            );

            if self.generating {
                blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Cursor,
                    text: if self.cursor_blink_on { "▌" } else { " " }.to_string(),
                    markdown_state: None,
                    markdown_id: None,
                    min_height: None,
                });
            }
        }

        if blocks.is_empty() {
            blocks.push(ThreadBlock {
                kind: ThreadBlockKind::Hint,
                text: "Type a message and press Enter. Use Shift+Enter for newline.".to_string(),
                markdown_state: None,
                markdown_id: None,
                min_height: None,
            });
        }

        blocks
    }

    fn append_chunk_blocks(
        chunks: &[StreamChunk],
        states: Option<&Vec<Option<Entity<TextViewState>>>>,
        id_prefix: &str,
        streaming: bool,
        blocks: &mut Vec<ThreadBlock>,
    ) {
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                let prev_is_tool = matches!(chunks[i - 1], StreamChunk::Tool(_));
                let curr_is_tool = matches!(chunk, StreamChunk::Tool(_));
                if !prev_is_tool || !curr_is_tool {
                    blocks.push(ThreadBlock {
                        kind: ThreadBlockKind::Spacer,
                        text: String::new(),
                        markdown_state: None,
                        markdown_id: None,
                        min_height: None,
                    });
                }
            }

            match chunk {
                StreamChunk::Reasoning(s) => blocks.push(ThreadBlock {
                    kind: if streaming {
                        ThreadBlockKind::StreamingReasoning
                    } else {
                        ThreadBlockKind::ReasoningMarkdown
                    },
                    text: s.clone(),
                    markdown_state: states
                        .and_then(|states| states.get(i))
                        .and_then(|state| state.clone()),
                    markdown_id: Some(format!("{id_prefix}-{i}")),
                    min_height: None,
                }),
                StreamChunk::Text(s) => blocks.push(ThreadBlock {
                    kind: if streaming {
                        ThreadBlockKind::StreamingMarkdown
                    } else {
                        ThreadBlockKind::Markdown
                    },
                    text: s.clone(),
                    markdown_state: states
                        .and_then(|states| states.get(i))
                        .and_then(|state| state.clone()),
                    markdown_id: Some(format!("{id_prefix}-{i}")),
                    min_height: None,
                }),
                StreamChunk::Tool(s) => blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Tool,
                    text: s.clone(),
                    markdown_state: None,
                    markdown_id: None,
                    min_height: None,
                }),
            }
        }
    }

    fn sync_thread_list_window(&self, block_count: usize) {
        let current_count = self.thread_list.item_count();
        if block_count > current_count {
            self.thread_list
                .splice(current_count..current_count, block_count - current_count);
        } else if block_count < current_count {
            self.thread_list.splice(block_count..current_count, 0);
        }
    }

    fn apply_stream_height_floors(&mut self, blocks: &mut [ThreadBlock]) {
        if !self.generating {
            self.stream_block_height_floors.clear();
            return;
        }

        let mut active_ids = Vec::new();
        for (ix, block) in blocks.iter_mut().enumerate() {
            if !matches!(
                block.kind,
                ThreadBlockKind::StreamingMarkdown | ThreadBlockKind::StreamingReasoning
            ) {
                continue;
            }

            let Some(id) = block.markdown_id.as_ref() else {
                continue;
            };

            active_ids.push(id.clone());

            if let Some(bounds) = self.thread_list.bounds_for_item(ix) {
                let observed = bounds.size.height;
                self.stream_block_height_floors
                    .entry(id.clone())
                    .and_modify(|height| {
                        if observed > *height {
                            *height = observed;
                        }
                    })
                    .or_insert(observed);
            }

            block.min_height = self.stream_block_height_floors.get(id).copied();
        }

        self.stream_block_height_floors
            .retain(|id, _| active_ids.iter().any(|active| active == id));
    }

    fn render_block(block: ThreadBlock, cx: &mut gpui::App) -> AnyElement {
        match block.kind {
            ThreadBlockKind::Spacer => div().h_2().into_any_element(),
            ThreadBlockKind::UserLabel => div()
                .w_full()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().cyan)
                .child(block.text)
                .into_any_element(),
            ThreadBlockKind::AssistantLabel => div()
                .w_full()
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().green)
                .child(block.text)
                .into_any_element(),
            ThreadBlockKind::Markdown | ThreadBlockKind::ReasoningMarkdown => {
                let view = if let Some(state) = block.markdown_state {
                    TextView::new(&state).selectable(true)
                } else {
                    let id = block
                        .markdown_id
                        .unwrap_or_else(|| "thread-md-fallback".to_string());
                    TextView::markdown(id, block.text).selectable(true)
                };

                if matches!(block.kind, ThreadBlockKind::ReasoningMarkdown) {
                    div()
                        .w_full()
                        .text_color(cx.theme().muted_foreground)
                        .italic()
                        .child(view)
                        .into_any_element()
                } else {
                    div().w_full().child(view).into_any_element()
                }
            }
            ThreadBlockKind::StreamingMarkdown | ThreadBlockKind::StreamingReasoning => {
                let view = if let Some(state) = block.markdown_state {
                    TextView::new(&state).selectable(true)
                } else {
                    let id = block
                        .markdown_id
                        .unwrap_or_else(|| "thread-md-fallback".to_string());
                    TextView::markdown(id, block.text).selectable(true)
                };

                let mut container = div().w_full();
                if let Some(min_height) = block.min_height {
                    container = container.min_h(min_height);
                }

                if matches!(block.kind, ThreadBlockKind::StreamingReasoning) {
                    container
                        .text_color(cx.theme().muted_foreground)
                        .italic()
                        .child(view)
                        .into_any_element()
                } else {
                    container.child(view).into_any_element()
                }
            }
            ThreadBlockKind::Tool => div()
                .w_full()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(block.text)
                .into_any_element(),
            ThreadBlockKind::Cursor => div()
                .w_full()
                .text_sm()
                .text_color(cx.theme().green)
                .child(block.text)
                .into_any_element(),
            ThreadBlockKind::Hint => div()
                .w_full()
                .text_sm()
                .italic()
                .text_color(cx.theme().muted_foreground)
                .child(block.text)
                .into_any_element(),
        }
    }
}

impl Render for AgntGui {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe_auto_scroll_to_bottom();

        let mut blocks = self.build_thread_blocks();
        self.sync_thread_list_window(blocks.len());
        self.apply_stream_height_floors(&mut blocks);
        let thread_list = list(self.thread_list.clone(), {
            let blocks = blocks;
            move |ix, _window, cx| Self::render_block(blocks[ix].clone(), cx)
        })
        .size_full();
        let resume_dialog_panel = self.render_resume_dialog_panel(cx);
        let typeahead_panel = self.render_typeahead_panel(cx);
        let send_label = if self.generating {
            "Generating..."
        } else {
            "Send"
        };
        let input_row = div()
            .w_full()
            .capture_action(cx.listener(Self::on_typeahead_enter_capture))
            .capture_action(cx.listener(Self::on_typeahead_escape_capture))
            .capture_action(cx.listener(Self::on_typeahead_up_capture))
            .capture_action(cx.listener(Self::on_typeahead_down_capture))
            .child(
                h_flex()
                    .w_full()
                    .items_end()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.input)))
                    .child(
                        Button::new("send")
                            .primary()
                            .large()
                            .label(send_label)
                            .disabled(self.generating)
                            .on_click(cx.listener(Self::on_send_click)),
                    ),
            )
            .into_any_element();
        let mut input_section = v_flex().w_full().gap_2();
        if let Some(panel) = resume_dialog_panel {
            input_section = input_section.child(panel);
        }
        if let Some(panel) = typeahead_panel {
            input_section = input_section.child(panel);
        }
        let input_section = input_section.child(input_row).into_any_element();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .h_0()
                    .flex_1()
                    .w_full()
                    .relative()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(
                        div()
                            .id("thread-scroll-area")
                            .size_full()
                            .on_scroll_wheel(cx.listener(Self::on_thread_scroll))
                            .p_3()
                            .child(thread_list),
                    )
                    .vertical_scrollbar(&self.thread_list),
            )
            .child(input_section)
    }
}

pub fn run(agent: Agent, session_store: SharedSessionStore) {
    let app = gpui::Application::new();
    let mut agent = Some(agent);
    let mut session_store = Some(session_store);

    app.run(move |cx: &mut GpuiApp| {
        gpui_component::init(cx);
        cx.bind_keys([KeyBinding::new(
            "shift-enter",
            InputEnter { secondary: true },
            Some("Input"),
        )]);

        let Some(agent) = agent.take() else {
            cx.quit();
            return;
        };
        let Some(session_store) = session_store.take() else {
            cx.quit();
            return;
        };

        if cx
            .open_window(WindowOptions::default(), move |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });

                let view = cx.new(|cx| AgntGui::new(agent, session_store, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .is_err()
        {
            cx.quit();
        }
    });
}

pub fn launch(agent: Agent, session_store: SharedSessionStore) {
    tokio::task::block_in_place(|| {
        run(agent, session_store);
    });
}

fn byte_offset_to_position(text: &str, byte_offset: usize) -> Position {
    let mut line = 0u32;
    let mut character = 0u32;
    for ch in text[..byte_offset.min(text.len())].chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }

    Position::new(line, character)
}

fn diff_from_display_body(body: Option<&DisplayBody>) -> Option<&str> {
    match body {
        Some(DisplayBody::Diff(diff)) if !diff.is_empty() => Some(diff.as_str()),
        _ => None,
    }
}

fn push_tool_diff_chunks(
    chunks: &mut Vec<StreamChunk>,
    states: &mut Vec<Option<Entity<TextViewState>>>,
    diff: &str,
) {
    for line in diff.lines() {
        chunks.push(StreamChunk::Tool(line.to_string()));
        states.push(None);
    }
    if diff.ends_with('\n') {
        chunks.push(StreamChunk::Tool(String::new()));
        states.push(None);
    }
}
