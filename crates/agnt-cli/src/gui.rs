use std::time::Duration;

use agnt_core::{Agent, AgentEvent};
use gpui::{
    AnyElement, App as GpuiApp, AppContext, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement, Pixels, Render, ScrollHandle, ScrollWheelEvent,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, WindowOptions, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Root, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Enter as InputEnter, Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState},
    v_flex,
};

use crate::session::SharedSessionStore;
use crate::tui::app::{DisplayMessage, Role, StreamChunk, display_messages_from_history};

#[derive(Clone, Copy)]
enum ThreadBlockKind {
    UserLabel,
    AssistantLabel,
    Markdown,
    ReasoningMarkdown,
    Tool,
    Cursor,
    Hint,
    Spacer,
}

struct ThreadBlock {
    kind: ThreadBlockKind,
    text: String,
    markdown_state: Option<Entity<TextViewState>>,
    markdown_id: Option<String>,
}

struct AgntGui {
    agent: Agent,
    session_store: SharedSessionStore,
    input: Entity<InputState>,
    thread_scroll: ScrollHandle,
    messages: Vec<DisplayMessage>,
    message_markdown_states: Vec<Vec<Option<Entity<TextViewState>>>>,
    stream_chunks: Vec<StreamChunk>,
    stream_markdown_states: Vec<Option<Entity<TextViewState>>>,
    generating: bool,
    cursor_blink_on: bool,
    stick_to_bottom: bool,
    stream_task: Task<()>,
    _blink_task: Task<()>,
    _input_subscription: Subscription,
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

        input.update(cx, |input, cx| input.focus(window, cx));

        Self {
            agent,
            session_store,
            input,
            thread_scroll: ScrollHandle::new(),
            messages,
            message_markdown_states,
            stream_chunks: Vec::new(),
            stream_markdown_states: Vec::new(),
            generating: false,
            cursor_blink_on: true,
            stick_to_bottom: true,
            stream_task: Task::ready(()),
            _blink_task: blink_task,
            _input_subscription: input_subscription,
        }
    }

    fn build_markdown_states(
        messages: &[DisplayMessage],
        cx: &mut Context<Self>,
    ) -> Vec<Vec<Option<Entity<TextViewState>>>> {
        let mut all_states = Vec::with_capacity(messages.len());
        for message in messages {
            let mut states = Vec::with_capacity(message.chunks.len());
            for chunk in &message.chunks {
                let state = match chunk {
                    StreamChunk::Text(text) | StreamChunk::Reasoning(text) => {
                        let text = text.clone();
                        Some(cx.new(move |cx| TextViewState::markdown(&text, cx)))
                    }
                    StreamChunk::Tool(_) => None,
                };
                states.push(state);
            }
            all_states.push(states);
        }
        all_states
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
        let offset = self.thread_scroll.offset().y;
        let max = self.thread_scroll.max_offset().height;
        (max + offset).abs()
    }

    fn maybe_auto_scroll_to_bottom(&mut self) {
        if !self.stick_to_bottom && self.distance_from_bottom() <= px(24.) {
            self.stick_to_bottom = true;
        }

        if self.stick_to_bottom {
            self.thread_scroll.scroll_to_bottom();
        }
    }

    fn submit_from_input(
        &mut self,
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generating {
            return;
        }

        let text = state.read(cx).value();
        let text = text.trim().to_string();
        if text.is_empty() {
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
                    this.finalize_response();
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
                }
            }
            AgentEvent::ToolCallStart { display, .. } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}...]", display.title)));
                self.stream_markdown_states.push(None);
            }
            AgentEvent::ToolCallDone { display, .. } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[{}]", display.title)));
                self.stream_markdown_states.push(None);
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
                    self.stream_markdown_states.push(None);
                }
                self.finalize_response();
                self.generating = false;
            }
            AgentEvent::Error { error } => {
                self.stream_chunks
                    .push(StreamChunk::Tool(format!("[error: {error}]")));
                self.stream_markdown_states.push(None);
                self.finalize_response();
                self.generating = false;
            }
        }

        self.maybe_auto_scroll_to_bottom();
        cx.notify();
    }

    fn finalize_response(&mut self) {
        let chunks = std::mem::take(&mut self.stream_chunks);
        let states = std::mem::take(&mut self.stream_markdown_states);
        if !chunks.is_empty() {
            self.messages.push(DisplayMessage {
                role: Role::Assistant,
                chunks,
            });
            self.message_markdown_states.push(states);
        }
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
            });

            let states = self.message_markdown_states.get(msg_ix);
            Self::append_chunk_blocks(&msg.chunks, states, &format!("msg-{msg_ix}"), &mut blocks);
        }

        if self.generating || !self.stream_chunks.is_empty() {
            if !blocks.is_empty() {
                blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Spacer,
                    text: String::new(),
                    markdown_state: None,
                    markdown_id: None,
                });
            }

            blocks.push(ThreadBlock {
                kind: ThreadBlockKind::AssistantLabel,
                text: "Assistant".to_string(),
                markdown_state: None,
                markdown_id: None,
            });

            Self::append_chunk_blocks(
                &self.stream_chunks,
                Some(&self.stream_markdown_states),
                "stream",
                &mut blocks,
            );

            if self.generating {
                blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Cursor,
                    text: if self.cursor_blink_on { "▌" } else { " " }.to_string(),
                    markdown_state: None,
                    markdown_id: None,
                });
            }
        }

        if blocks.is_empty() {
            blocks.push(ThreadBlock {
                kind: ThreadBlockKind::Hint,
                text: "Type a message and press Enter. Use Shift+Enter for newline.".to_string(),
                markdown_state: None,
                markdown_id: None,
            });
        }

        blocks
    }

    fn append_chunk_blocks(
        chunks: &[StreamChunk],
        states: Option<&Vec<Option<Entity<TextViewState>>>>,
        id_prefix: &str,
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
                    });
                }
            }

            match chunk {
                StreamChunk::Reasoning(s) => blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::ReasoningMarkdown,
                    text: s.clone(),
                    markdown_state: states
                        .and_then(|states| states.get(i))
                        .and_then(|state| state.clone()),
                    markdown_id: Some(format!("{id_prefix}-{i}")),
                }),
                StreamChunk::Text(s) => blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Markdown,
                    text: s.clone(),
                    markdown_state: states
                        .and_then(|states| states.get(i))
                        .and_then(|state| state.clone()),
                    markdown_id: Some(format!("{id_prefix}-{i}")),
                }),
                StreamChunk::Tool(s) => blocks.push(ThreadBlock {
                    kind: ThreadBlockKind::Tool,
                    text: s.clone(),
                    markdown_state: None,
                    markdown_id: None,
                }),
            }
        }
    }

    fn render_block(block: ThreadBlock, cx: &mut Context<Self>) -> AnyElement {
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

        let blocks = self.build_thread_blocks();
        let send_label = if self.generating {
            "Generating..."
        } else {
            "Send"
        };

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
                            .track_scroll(&self.thread_scroll)
                            .overflow_y_scroll()
                            .on_scroll_wheel(cx.listener(Self::on_thread_scroll))
                            .p_3()
                            .child(
                                v_flex().w_full().gap_1().children(
                                    blocks
                                        .into_iter()
                                        .map(|block| Self::render_block(block, cx)),
                                ),
                            ),
                    )
                    .vertical_scrollbar(&self.thread_scroll),
            )
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
