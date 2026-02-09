use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub root_dir: PathBuf,
    pub name: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: Option<String>,
    pub root_turn_id: Option<String>,
    pub current_turn_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub session_id: String,
    pub parent_turn_id: Option<String>,
    pub user_parts: serde_json::Value,
    pub assistant_parts: serde_json::Value,
    pub conversation_state: serde_json::Value,
    pub usage: Option<serde_json::Value>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOp {
    pub seq: i64,
    pub session_id: String,
    pub op_type: String,
    pub payload: serde_json::Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnPathItem {
    pub turn: Turn,
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionInput {
    pub project_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendTurnInput {
    pub session_id: String,
    /// Parent to branch from. `None` uses the session's current checkout turn.
    pub parent_turn_id: Option<String>,
    pub user_parts: serde_json::Value,
    pub assistant_parts: serde_json::Value,
    pub conversation_state: serde_json::Value,
    pub usage: Option<serde_json::Value>,
}
