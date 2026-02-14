use agnt_db::Session;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::session::session_label;

const DIM: Style = Style::new().fg(Color::DarkGray);
const ACTIVE: Style = Style::new().fg(Color::Yellow);

#[derive(Debug, Clone)]
pub struct ResumeSessionDialogEntry {
    pub session_id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct ResumeSessionDialogState {
    pub entries: Vec<ResumeSessionDialogEntry>,
    pub selected_index: usize,
}

impl ResumeSessionDialogState {
    pub fn new(entries: Vec<ResumeSessionDialogEntry>) -> Self {
        Self {
            entries,
            selected_index: 0,
        }
    }
}

pub fn build_dialog_entries(sessions: Vec<Session>) -> Vec<ResumeSessionDialogEntry> {
    sessions
        .into_iter()
        .map(|session| ResumeSessionDialogEntry {
            session_id: session.id.clone(),
            label: session_label(&session),
        })
        .collect()
}

pub fn move_selection(dialog: &mut ResumeSessionDialogState, direction: i32) {
    if dialog.entries.is_empty() {
        return;
    }

    if direction < 0 {
        dialog.selected_index = if dialog.selected_index == 0 {
            dialog.entries.len() - 1
        } else {
            dialog.selected_index - 1
        };
    } else {
        dialog.selected_index = (dialog.selected_index + 1) % dialog.entries.len();
    }
}

pub fn selected_session_id(dialog: &ResumeSessionDialogState) -> Option<&str> {
    dialog
        .entries
        .get(dialog.selected_index)
        .map(|entry| entry.session_id.as_str())
}

pub fn render(frame: &mut Frame, dialog: Option<&ResumeSessionDialogState>, area: Rect) {
    let Some(dialog) = dialog else {
        return;
    };
    if dialog.entries.is_empty() {
        return;
    }

    let max_visible_rows = 8usize;
    let dialog_width = area.width.saturating_sub(8).clamp(20, 90);
    let dialog_height = (dialog.entries.len().min(max_visible_rows) as u16 + 4).clamp(6, 16);
    let popup_area = centered_rect(dialog_width, dialog_height, area);

    let visible_rows = popup_area.height.saturating_sub(4) as usize;
    let start = if dialog.selected_index >= visible_rows && visible_rows > 0 {
        dialog.selected_index + 1 - visible_rows
    } else {
        0
    };
    let end = (start + visible_rows).min(dialog.entries.len());

    let mut lines = vec![Line::from(Span::styled(
        "Enter to resume, Esc to cancel",
        DIM,
    ))];
    for (idx, entry) in dialog.entries[start..end].iter().enumerate() {
        let absolute_index = start + idx;
        let marker = if absolute_index == dialog.selected_index {
            "› "
        } else {
            "  "
        };
        let style = if absolute_index == dialog.selected_index {
            ACTIVE
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, DIM),
            Span::styled(entry.label.clone(), style),
        ]));
    }

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .title(" Resume Session ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        popup_area,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = width.min(area.width);
    let popup_height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
