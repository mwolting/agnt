use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::app::{App, AppState, Role, StreamChunk};
use crate::tui::session_dialog;
use crate::typeahead::{
    ActiveTypeahead, TypeaheadItem, TypeaheadMatchSet, TypeaheadWindowItem,
    build_typeahead_window_items,
};

const USER_COLOR: Color = Color::Cyan;
const ASSISTANT_COLOR: Color = Color::Green;
const REASONING_STYLE: Style = Style::new()
    .fg(Color::DarkGray)
    .add_modifier(Modifier::ITALIC);
const DIM: Style = Style::new().fg(Color::DarkGray);
const TYPEAHEAD_HEADER: Style = Style::new().fg(Color::Yellow);
const TYPEAHEAD_ACTIVE: Style = Style::new().fg(Color::Yellow);

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let typeahead = app.typeahead_matches();
    let selected_index = app.typeahead_selected_index();
    let window_start = app.typeahead_window_start();
    let typeahead_height = calculate_typeahead_height(typeahead.as_ref());
    let input_height = calculate_input_height(app, area.width);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1), // separator
        Constraint::Length(typeahead_height),
        Constraint::Length(input_height),
    ])
    .split(area);

    render_messages(frame, app, chunks[0]);
    render_separator(frame, chunks[1]);
    render_typeahead(
        frame,
        typeahead.as_ref(),
        selected_index,
        window_start,
        chunks[2],
    );
    render_input(frame, app, chunks[3]);
    session_dialog::render(frame, app.resume_dialog.as_ref(), area);
}

/// Manually wrap a styled line to fit within `width` columns.
/// Returns one or more Lines that each fit within the width.
fn wrap_line(line: &Line, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::raw("")];
    }

    // Flatten all spans into a single list of (char, Style) to handle
    // span boundaries mid-wrap.
    let mut chars: Vec<(char, Style)> = Vec::new();
    for span in &line.spans {
        let style = span.style;
        for ch in span.content.chars() {
            chars.push((ch, style));
        }
    }

    if chars.is_empty() {
        return vec![Line::raw("")];
    }

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut col = 0;
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = chars[0].1;

    for (ch, style) in &chars {
        if *style != current_style {
            if !current_text.is_empty() {
                current_spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    current_style,
                ));
            }
            current_style = *style;
        }

        if col >= width {
            // Flush current line
            if !current_text.is_empty() {
                current_spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    current_style,
                ));
            }
            result.push(Line::from(std::mem::take(&mut current_spans)));
            col = 0;
        }

        current_text.push(*ch);
        col += 1;
    }

    // Flush remaining
    if !current_text.is_empty() {
        current_spans.push(Span::styled(current_text, current_style));
    }
    if !current_spans.is_empty() {
        result.push(Line::from(current_spans));
    }

    if result.is_empty() {
        result.push(Line::raw(""));
    }

    result
}

fn is_empty_line(line: &Line) -> bool {
    line.spans.iter().all(|span| span.content.is_empty())
}

/// Append styled lines for a slice of [`StreamChunk`]s.
fn render_chunks(chunks: &[StreamChunk], lines: &mut Vec<Line<'static>>) {
    for (i, chunk) in chunks.iter().enumerate() {
        // Blank line between chunks, except consecutive Tool chunks
        // (start + done belong together).
        if i > 0 {
            let prev_is_tool = matches!(chunks[i - 1], StreamChunk::Tool(_));
            let curr_is_tool = matches!(chunk, StreamChunk::Tool(_));
            if !prev_is_tool || !curr_is_tool {
                lines.push(Line::raw(""));
            }
        }

        match chunk {
            StreamChunk::Reasoning(s) => {
                for text_line in s.lines() {
                    lines.push(Line::from(Span::styled(
                        text_line.to_string(),
                        REASONING_STYLE,
                    )));
                }
                if s.ends_with('\n') {
                    lines.push(Line::raw(""));
                }
            }
            StreamChunk::Text(s) => {
                for text_line in s.lines() {
                    lines.push(Line::raw(text_line.to_string()));
                }
                if s.ends_with('\n') {
                    lines.push(Line::raw(""));
                }
            }
            StreamChunk::Tool(s) => {
                if s.is_empty() {
                    lines.push(Line::raw(""));
                } else {
                    for text_line in s.lines() {
                        lines.push(Line::from(Span::styled(text_line.to_string(), DIM)));
                    }
                    if s.ends_with('\n') {
                        lines.push(Line::raw(""));
                    }
                }
            }
        }
    }
}

/// Build the logical lines for the messages area, then wrap them.
fn build_message_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut logical_lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        if !logical_lines.is_empty() {
            logical_lines.push(Line::raw(""));
        }

        let (label, color) = match msg.role {
            Role::User => ("You", USER_COLOR),
            Role::Assistant => ("Assistant", ASSISTANT_COLOR),
        };

        logical_lines.push(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));

        render_chunks(&msg.chunks, &mut logical_lines);
    }

    // Streaming / typing indicator
    let is_generating = matches!(app.state, AppState::Generating { .. });
    if is_generating || !app.stream_chunks.is_empty() {
        if !logical_lines.is_empty() {
            logical_lines.push(Line::raw(""));
        }
        logical_lines.push(Line::from(Span::styled(
            "Assistant".to_string(),
            Style::default()
                .fg(ASSISTANT_COLOR)
                .add_modifier(Modifier::BOLD),
        )));

        render_chunks(&app.stream_chunks, &mut logical_lines);

        // Blinking cursor (only while generating).
        if is_generating {
            let cursor_char = if app.cursor_blink_on { "█" } else { " " };
            let cursor_span = Span::styled(
                cursor_char.to_string(),
                Style::default().fg(ASSISTANT_COLOR),
            );

            if app.stream_chunks.is_empty() {
                // Nothing yet — cursor on its own line.
                logical_lines.push(Line::from(cursor_span));
            } else {
                // Check if the last chunk ended with a newline or is a Tool
                // line — if so the cursor belongs on a fresh line.
                let needs_new_line = match app.stream_chunks.last() {
                    Some(StreamChunk::Tool(_)) => true,
                    Some(StreamChunk::Text(s) | StreamChunk::Reasoning(s)) => s.ends_with('\n'),
                    None => false,
                };
                let needs_new_line =
                    needs_new_line && !logical_lines.last().is_some_and(is_empty_line);

                if needs_new_line {
                    logical_lines.push(Line::from(cursor_span));
                } else if let Some(last) = logical_lines.last_mut() {
                    let mut spans = last.spans.clone();
                    spans.push(cursor_span);
                    *last = Line::from(spans);
                }
            }
        }
    }

    if logical_lines.is_empty() {
        logical_lines.push(Line::from(Span::styled(
            "Type a message and press Enter to start.".to_string(),
            DIM,
        )));
    }

    // Pre-wrap all lines so rendered height == lines.len()
    logical_lines
        .iter()
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

fn render_messages(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let width = area.width as usize;
    let visible = area.height as usize;
    let mut lines = build_message_lines(app, width);
    let content_height = lines.len();

    // Anchor to bottom: pad top if content is shorter than viewport.
    if content_height < visible {
        let padding = visible - content_height;
        let mut padded = vec![Line::raw(""); padding];
        padded.append(&mut lines);
        lines = padded;
    }

    // Now lines.len() >= visible. Scroll math is exact since we pre-wrapped.
    let total = lines.len();
    let max_scroll = total.saturating_sub(visible);
    let prev_max = app.max_scroll;
    app.max_scroll = max_scroll as u16;

    // If scrolled up and content grew, bump offset to keep viewport stable.
    if app.scroll_offset > 0 && max_scroll > prev_max as usize {
        app.scroll_offset += (max_scroll - prev_max as usize) as u16;
    }
    app.scroll_offset = app.scroll_offset.min(max_scroll as u16);

    // Slice the visible window directly — no Paragraph::scroll needed.
    let scroll = max_scroll - app.scroll_offset as usize;
    let visible_lines = &lines[scroll..scroll + visible.min(total)];

    let text = Text::from(visible_lines.to_vec());
    let messages_widget = Paragraph::new(text);
    frame.render_widget(messages_widget, area);
}

fn render_separator(frame: &mut Frame, area: ratatui::layout::Rect) {
    let line = Line::from(Span::styled("─".repeat(area.width as usize), DIM));
    frame.render_widget(Paragraph::new(line), area);
}

fn render_typeahead(
    frame: &mut Frame,
    active: Option<&ActiveTypeahead>,
    selected_index: usize,
    window_start: usize,
    area: ratatui::layout::Rect,
) {
    if area.height == 0 {
        return;
    }

    let Some(active) = active else {
        return;
    };

    match active {
        ActiveTypeahead::Command(set) => {
            render_match_set(frame, set, selected_index, window_start, area)
        }
        ActiveTypeahead::Mention(set) => {
            render_match_set(frame, set, selected_index, window_start, area)
        }
    }
}

fn render_input(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Render prompt marker in the first two columns.
    let prompt_width = area.width.min(2);
    if prompt_width > 0 {
        let prompt = "> ";
        let prompt_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y,
            width: prompt_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(prompt, Style::default().fg(Color::DarkGray))),
            prompt_area,
        );
    }

    // Text is rendered in the remaining width, wrapped without trimming.
    let text_area = ratatui::layout::Rect {
        x: area.x.saturating_add(2),
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    };

    if text_area.width > 0 {
        let input_text = if app.input.is_empty() && matches!(app.state, AppState::Idle) {
            Text::from(Span::styled("Type a message...", DIM))
        } else {
            Text::raw(app.input.as_str())
        };

        let input_widget = Paragraph::new(input_text).wrap(Wrap { trim: false });
        frame.render_widget(input_widget, text_area);
    }

    let inner_width = text_area.width.max(1) as usize;
    let (cursor_row, cursor_col) = cursor_position(&app.input, app.cursor_pos, inner_width);
    let max_row = area.height.saturating_sub(1) as usize;
    let clamped_row = cursor_row.min(max_row) as u16;
    let cursor_x = text_area
        .x
        .saturating_add(cursor_col.min(inner_width.saturating_sub(1)) as u16);
    frame.set_cursor_position((cursor_x, area.y + clamped_row));
}

fn calculate_typeahead_height(active: Option<&ActiveTypeahead>) -> u16 {
    let Some(active) = active else {
        return 0;
    };

    let match_count = match active {
        ActiveTypeahead::Command(set) => set.matches.len(),
        ActiveTypeahead::Mention(set) => set.matches.len(),
    };

    if match_count == 0 {
        return 2;
    }

    (1 + match_count.min(4)) as u16
}

fn render_match_set<T: TypeaheadItem>(
    frame: &mut Frame,
    set: &TypeaheadMatchSet<T>,
    selected_index: usize,
    window_start: usize,
    area: ratatui::layout::Rect,
) {
    let header = if set.query.is_empty() {
        "Suggestions".to_string()
    } else {
        format!("Suggestions for `{}`", set.query)
    };
    let mut lines = vec![Line::from(Span::styled(header, TYPEAHEAD_HEADER))];

    let max_items = area.height.saturating_sub(1) as usize;
    if set.matches.is_empty() {
        let status = if set.loading && set.show_loading {
            match set.leader {
                '@' => {
                    if set.query.is_empty() {
                        "  indexing files..."
                    } else {
                        "  searching..."
                    }
                }
                _ => "  loading...",
            }
        } else if set.loading {
            "  "
        } else {
            "  no matches"
        };
        lines.push(Line::from(Span::styled(status, DIM)));
    } else {
        let rows = build_typeahead_window_items(set.matches.len(), window_start, max_items);
        for row in rows {
            match row {
                TypeaheadWindowItem::Value(absolute_index) => {
                    push_match_line(&mut lines, set, absolute_index, selected_index);
                }
                TypeaheadWindowItem::Divider => {
                    let divider_width = area.width.saturating_sub(2) as usize;
                    lines.push(Line::from(Span::styled(
                        format!("  {}", "─".repeat(divider_width.max(1))),
                        DIM,
                    )));
                }
            }
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn push_match_line<T: TypeaheadItem>(
    lines: &mut Vec<Line<'static>>,
    set: &TypeaheadMatchSet<T>,
    absolute_index: usize,
    selected_index: usize,
) {
    let item = &set.matches[absolute_index];
    let marker = if absolute_index == selected_index {
        "› "
    } else {
        "  "
    };
    let token = item.token_text();
    let token_style = if absolute_index == selected_index {
        TYPEAHEAD_ACTIVE
    } else {
        Style::default()
    };

    let mut spans = vec![Span::styled(marker, DIM), Span::styled(token, token_style)];

    if let Some(description) = item.description() {
        spans.push(Span::styled(format!("  {description}"), DIM));
    }

    lines.push(Line::from(spans));
}

fn calculate_input_height(app: &App, width: u16) -> u16 {
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let line_count = if app.input.is_empty() {
        1
    } else {
        app.input
            .split('\n')
            .map(|line| {
                let len = line.len().max(1);
                len.div_ceil(inner_width) as u16
            })
            .sum::<u16>()
            .max(1)
    };
    line_count.clamp(2, 8)
}

fn cursor_position(input: &str, byte_pos: usize, width: usize) -> (usize, usize) {
    let width = width.max(1);
    let before_cursor = &input[..byte_pos.min(input.len())];
    let mut row = 0;
    let mut col = 0;

    for ch in before_cursor.chars() {
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
            if col >= width {
                row += 1;
                col = 0;
            }
        }
    }

    (row, col)
}
