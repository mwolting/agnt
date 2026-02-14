use std::path::Path;
use std::sync::Arc;

use agnt_core::{Agent, ConversationState};
use agnt_db::{AppendTurnInput, CreateSessionInput, Session, Store};
use agnt_llm::stream::Usage;
use agnt_llm::{AssistantPart, Message, UserPart};
use parking_lot::Mutex;
use serde_json::Value;

pub type SharedSessionStore = Arc<Mutex<SessionStore>>;

const SESSION_TITLE_MAX_CHARS: usize = 80;

pub struct SessionStore {
    store: Arc<Mutex<Store>>,
    project_id: String,
    active_session_id: Option<String>,
}

impl SessionStore {
    pub fn open_for_project_root(
        store: Arc<Mutex<Store>>,
        project_root: &Path,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let project = {
            let mut db = store.lock();
            db.sessions().upsert_project(project_root, None)?
        };

        Ok(Self {
            store,
            project_id: project.id,
            active_session_id: None,
        })
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>, Box<dyn std::error::Error>> {
        let mut db = self.store.lock();
        Ok(db
            .sessions()
            .list_sessions_for_project(&self.project_id, limit)?)
    }

    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    pub fn clear_active_session(&mut self) {
        self.active_session_id = None;
    }

    pub fn create_session(
        &mut self,
        title: Option<String>,
    ) -> Result<Session, Box<dyn std::error::Error>> {
        let session = {
            let mut db = self.store.lock();
            db.sessions().create_session(CreateSessionInput {
                project_id: self.project_id.clone(),
                title,
            })?
        };

        self.active_session_id = Some(session.id.clone());
        Ok(session)
    }

    pub fn ensure_active_session(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.active_session_id.is_none() {
            self.create_session(None)?;
        }
        Ok(())
    }

    pub fn activate_session(
        &mut self,
        session_id: &str,
    ) -> Result<Option<ConversationState>, Box<dyn std::error::Error>> {
        let session = {
            let mut db = self.store.lock();
            db.sessions()
                .get_session(session_id)?
                .ok_or_else(|| format!("session not found: {session_id}"))?
        };

        if session.project_id != self.project_id {
            return Err(format!(
                "session '{session_id}' does not belong to project '{}'",
                self.project_id
            )
            .into());
        }

        self.active_session_id = Some(session.id.clone());
        self.load_active_conversation_state()
    }

    pub fn resume_most_recent_session(
        &mut self,
    ) -> Result<Option<ConversationState>, Box<dyn std::error::Error>> {
        let latest_session_id = {
            let mut db = self.store.lock();
            db.sessions()
                .list_sessions_for_project(&self.project_id, 1)?
                .into_iter()
                .next()
                .map(|session| session.id)
        };

        let Some(session_id) = latest_session_id else {
            return Ok(None);
        };

        self.active_session_id = Some(session_id);
        self.load_active_conversation_state()
    }

    pub fn load_active_conversation_state(
        &mut self,
    ) -> Result<Option<ConversationState>, Box<dyn std::error::Error>> {
        let Some(session_id) = self.active_session_id.as_deref() else {
            return Ok(None);
        };

        let turn = {
            let mut db = self.store.lock();
            db.sessions().current_turn(session_id)?
        };

        let Some(turn) = turn else {
            return Ok(None);
        };

        Ok(Some(serde_json::from_value(turn.conversation_state)?))
    }

    pub fn persist_turn_from_agent(
        &mut self,
        agent: &Agent,
        usage: &Usage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session_id) = self.active_session_id.clone() else {
            return Err("no active session selected".into());
        };

        let snapshot = agent.conversation_state();
        let (user_parts, assistant_parts) = extract_latest_turn_parts(&snapshot.messages)?;
        let session_title = derive_session_title(&snapshot.messages);

        let mut db = self.store.lock();
        db.sessions().append_turn(AppendTurnInput {
            session_id: session_id.clone(),
            parent_turn_id: None,
            user_parts,
            assistant_parts,
            conversation_state: serde_json::to_value(&snapshot)?,
            usage: Some(serde_json::to_value(usage)?),
        })?;

        if let Some(title) = session_title.as_deref() {
            db.sessions()
                .set_session_title_if_missing(&session_id, title)?;
        }

        Ok(())
    }
}

pub fn session_label(session: &Session) -> String {
    if let Some(title) = &session.title {
        return format!("{title} ({})", session.id);
    }
    format!("Session {}", session.id)
}

fn extract_latest_turn_parts(
    messages: &[Message],
) -> Result<(Value, Value), Box<dyn std::error::Error>> {
    let user_idx = messages
        .iter()
        .rposition(|m| matches!(m, Message::User { .. }))
        .ok_or("cannot persist turn: no user message found")?;

    let user_parts = match &messages[user_idx] {
        Message::User { parts } => serde_json::to_value(parts)?,
        _ => return Err("cannot persist turn: invalid user message shape".into()),
    };

    let mut assistant_parts: Vec<AssistantPart> = Vec::new();
    for message in &messages[user_idx + 1..] {
        if let Message::Assistant { parts } = message {
            assistant_parts.extend(parts.clone());
        }
    }
    if assistant_parts.is_empty() {
        return Err("cannot persist turn: no assistant content found for latest user turn".into());
    }

    Ok((user_parts, serde_json::to_value(assistant_parts)?))
}

fn derive_session_title(messages: &[Message]) -> Option<String> {
    let first_user_parts = messages.iter().find_map(|message| match message {
        Message::User { parts } => Some(parts),
        _ => None,
    })?;

    let title_text = first_user_parts
        .iter()
        .filter_map(|part| match part {
            UserPart::Text(text) => Some(text.text.trim()),
            UserPart::Image(_) => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if title_text.is_empty() {
        return None;
    }

    let normalized = title_text.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized.is_empty() {
        return None;
    }

    Some(truncate_with_ellipsis(&normalized, SESSION_TITLE_MAX_CHARS))
}

fn truncate_with_ellipsis(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }

    let mut truncated = input.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}
