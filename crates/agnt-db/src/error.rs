#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("turn not found: {0}")]
    TurnNotFound(String),

    #[error("turn '{turn_id}' does not belong to session '{session_id}'")]
    TurnSessionMismatch { session_id: String, turn_id: String },

    #[error("parent turn '{parent_turn_id}' does not belong to session '{session_id}'")]
    ParentTurnSessionMismatch {
        session_id: String,
        parent_turn_id: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
