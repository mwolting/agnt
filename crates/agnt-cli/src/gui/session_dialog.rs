use agnt_db::Session;

use crate::session::session_label;

#[derive(Clone)]
pub struct ResumeDialogEntry {
    pub session_id: String,
    pub label: String,
}

pub struct ResumeDialogState {
    pub entries: Vec<ResumeDialogEntry>,
    pub selected_index: usize,
}

impl ResumeDialogState {
    pub fn new(entries: Vec<ResumeDialogEntry>) -> Self {
        Self {
            entries,
            selected_index: 0,
        }
    }
}

pub fn build_dialog_entries(sessions: Vec<Session>) -> Vec<ResumeDialogEntry> {
    sessions
        .into_iter()
        .map(|session| ResumeDialogEntry {
            session_id: session.id.clone(),
            label: session_label(&session),
        })
        .collect()
}

pub fn move_selection(dialog: &mut ResumeDialogState, direction: i32) {
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

pub fn selected_session_id(dialog: &ResumeDialogState) -> Option<&str> {
    dialog
        .entries
        .get(dialog.selected_index)
        .map(|entry| entry.session_id.as_str())
}
