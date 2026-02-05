use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, AppState, Role};

const USER_COLOR: Color = Color::Cyan;
const ASSISTANT_COLOR: Color = Color::Green;
const DIM: Style = Style::new().fg(Color::DarkGray);

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let input_height = calculate_input_height(app, area.width);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1), // separator
        Constraint::Length(input_height),
    ])
    .split(area);

    render_messages(frame, app, chunks[0]);
    render_separator(frame, chunks[1]);
    render_input(frame, app, chunks[2]);
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

        for text_line in msg.content.lines() {
            logical_lines.push(Line::raw(text_line.to_string()));
        }
    }

    // Streaming / typing indicator
    if matches!(app.state, AppState::Generating { .. }) {
        if !logical_lines.is_empty() {
            logical_lines.push(Line::raw(""));
        }
        logical_lines.push(Line::from(Span::styled(
            "Assistant".to_string(),
            Style::default()
                .fg(ASSISTANT_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        let cursor_char = if app.cursor_blink_on { "█" } else { " " };
        if app.current_response.is_empty() {
            logical_lines.push(Line::from(Span::styled(
                cursor_char.to_string(),
                Style::default().fg(ASSISTANT_COLOR),
            )));
        } else {
            for text_line in app.current_response.lines() {
                logical_lines.push(Line::raw(text_line.to_string()));
            }
            if let Some(last) = logical_lines.last_mut() {
                let mut spans = last.spans.clone();
                spans.push(Span::styled(
                    cursor_char.to_string(),
                    Style::default().fg(ASSISTANT_COLOR),
                ));
                *last = Line::from(spans);
            }
        }
    } else if !app.current_response.is_empty() {
        if !logical_lines.is_empty() {
            logical_lines.push(Line::raw(""));
        }
        logical_lines.push(Line::from(Span::styled(
            "Assistant".to_string(),
            Style::default()
                .fg(ASSISTANT_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        for text_line in app.current_response.lines() {
            logical_lines.push(Line::raw(text_line.to_string()));
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

fn render_input(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let input_text = if app.input.is_empty() && matches!(app.state, AppState::Idle) {
        Text::from(Span::styled("> Type a message...", DIM))
    } else {
        let mut lines: Vec<Line> = Vec::new();
        for (i, text_line) in app.input.lines().enumerate() {
            let prefix = if i == 0 { "> " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                Span::raw(text_line.to_string()),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "> ",
                Style::default().fg(Color::DarkGray),
            )));
        }
        Text::from(lines)
    };

    let input_widget = Paragraph::new(input_text);
    frame.render_widget(input_widget, area);

    let inner_width = area.width.saturating_sub(2) as usize;
    let (cursor_row, cursor_col) = cursor_position(&app.input, app.cursor_pos, inner_width);
    frame.set_cursor_position((area.x + 2 + cursor_col as u16, area.y + cursor_row as u16));
}

fn calculate_input_height(app: &App, width: u16) -> u16 {
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let line_count = if app.input.is_empty() {
        1
    } else {
        app.input
            .lines()
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
