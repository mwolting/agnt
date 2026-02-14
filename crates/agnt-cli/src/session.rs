use std::path::Path;
use std::sync::Arc;

use agnt_core::{Agent, ConversationState};
use agnt_db::{AppendTurnInput, CreateSessionInput, Session, Store};
use agnt_llm::stream::Usage;
use agnt_llm::{AssistantPart, Message};
use parking_lot::Mutex;
use serde_json::Value;

pub type SharedSessionStore = Arc<Mutex<SessionStore>>;

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

        let mut db = self.store.lock();
        db.sessions().append_turn(AppendTurnInput {
            session_id,
            parent_turn_id: None,
            user_parts,
            assistant_parts,
            conversation_state: serde_json::to_value(&snapshot)?,
            usage: Some(serde_json::to_value(usage)?),
        })?;
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
