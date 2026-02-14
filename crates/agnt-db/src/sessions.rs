use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::database::Database;
use crate::error::{Error, Result};

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

pub struct Sessions<'db> {
    pub(crate) db: &'db mut Database,
}

impl Sessions<'_> {
    pub fn upsert_project(
        &mut self,
        root_dir: impl AsRef<Path>,
        name: Option<String>,
    ) -> Result<Project> {
        let root_dir = path_to_string(root_dir.as_ref());
        let now = now_ms();

        let tx = self.db.conn.transaction()?;

        let existing = tx
            .query_row(
                "SELECT id, root_dir, name, created_at_ms, updated_at_ms
                 FROM projects
                 WHERE root_dir = ?1",
                params![root_dir],
                row_to_project,
            )
            .optional()?;

        let project = if let Some(mut project) = existing {
            if name.is_some() && project.name != name {
                tx.execute(
                    "UPDATE projects
                     SET name = ?2, updated_at_ms = ?3
                     WHERE id = ?1",
                    params![project.id, name, now],
                )?;
                project.name = name;
                project.updated_at_ms = now;
            }
            project
        } else {
            let id = generate_id(&tx, "proj")?;
            tx.execute(
                "INSERT INTO projects (id, root_dir, name, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, root_dir, name, now, now],
            )?;
            Project {
                id,
                root_dir: PathBuf::from(root_dir),
                name,
                created_at_ms: now,
                updated_at_ms: now,
            }
        };

        tx.commit()?;
        Ok(project)
    }

    pub fn project_by_root_dir(&self, root_dir: impl AsRef<Path>) -> Result<Option<Project>> {
        let root_dir = path_to_string(root_dir.as_ref());
        self.db
            .conn
            .query_row(
                "SELECT id, root_dir, name, created_at_ms, updated_at_ms
                 FROM projects
                 WHERE root_dir = ?1",
                params![root_dir],
                row_to_project,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn get_project(&self, project_id: &str) -> Result<Option<Project>> {
        self.db
            .conn
            .query_row(
                "SELECT id, root_dir, name, created_at_ms, updated_at_ms
                 FROM projects
                 WHERE id = ?1",
                params![project_id],
                row_to_project,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn create_session(&mut self, input: CreateSessionInput) -> Result<Session> {
        let now = now_ms();
        let tx = self.db.conn.transaction()?;

        ensure_project_exists(&tx, &input.project_id)?;

        let id = generate_id(&tx, "sess")?;
        tx.execute(
            "INSERT INTO sessions (
                id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5)",
            params![id, input.project_id, input.title, now, now],
        )?;

        insert_session_op(
            &tx,
            &id,
            "session.created",
            &json!({
                "session_id": id.clone(),
                "project_id": input.project_id.clone(),
                "title": input.title.clone(),
            }),
            now,
        )?;

        let session = tx.query_row(
            "SELECT id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
             FROM sessions
             WHERE id = ?1",
            params![id],
            row_to_session,
        )?;

        tx.commit()?;
        Ok(session)
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.db
            .conn
            .query_row(
                "SELECT id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
                 FROM sessions
                 WHERE id = ?1",
                params![session_id],
                row_to_session,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn list_sessions_for_project(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<Session>> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
             FROM sessions
             WHERE project_id = ?1
             ORDER BY updated_at_ms DESC
             LIMIT ?2",
        )?;

        let iter = stmt.query_map(params![project_id, limit as i64], row_to_session)?;
        collect_rows(iter)
    }

    pub fn set_session_title_if_missing(&mut self, session_id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }

        let now = now_ms();
        let tx = self.db.conn.transaction()?;

        ensure_session_exists(&tx, session_id)?;

        let changed = tx.execute(
            "UPDATE sessions
             SET title = ?2, updated_at_ms = ?3
             WHERE id = ?1
               AND (title IS NULL OR trim(title) = '')",
            params![session_id, title, now],
        )?;

        if changed > 0 {
            insert_session_op(
                &tx,
                session_id,
                "session.title_set",
                &json!({ "title": title }),
                now,
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn append_turn(&mut self, input: AppendTurnInput) -> Result<Turn> {
        let now = now_ms();
        let tx = self.db.conn.transaction()?;

        let session = tx
            .query_row(
                "SELECT id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
                 FROM sessions
                 WHERE id = ?1",
                params![input.session_id],
                row_to_session,
            )
            .optional()?
            .ok_or_else(|| Error::SessionNotFound(input.session_id.clone()))?;

        let parent_turn_id = input.parent_turn_id.or(session.current_turn_id.clone());
        if let Some(parent_turn_id) = parent_turn_id.as_deref() {
            match ensure_turn_belongs_to_session(&tx, &input.session_id, parent_turn_id) {
                Ok(()) => {}
                Err(Error::TurnNotFound(_)) => {
                    return Err(Error::TurnNotFound(parent_turn_id.to_string()));
                }
                Err(Error::TurnSessionMismatch { .. }) => {
                    return Err(Error::ParentTurnSessionMismatch {
                        session_id: input.session_id.clone(),
                        parent_turn_id: parent_turn_id.to_string(),
                    });
                }
                Err(err) => return Err(err),
            }
        }

        let turn_id = generate_id(&tx, "turn")?;
        let user_parts_json = serde_json::to_string(&input.user_parts)?;
        let assistant_parts_json = serde_json::to_string(&input.assistant_parts)?;
        let conversation_state_json = serde_json::to_string(&input.conversation_state)?;
        let usage_json = input
            .usage
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        tx.execute(
            "INSERT INTO turns (
                id, session_id, parent_turn_id,
                user_parts_json, assistant_parts_json, conversation_state_json, usage_json, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                turn_id,
                input.session_id,
                parent_turn_id,
                user_parts_json,
                assistant_parts_json,
                conversation_state_json,
                usage_json,
                now
            ],
        )?;

        let root_turn_id = session.root_turn_id.clone().or(Some(turn_id.clone()));
        tx.execute(
            "UPDATE sessions
             SET root_turn_id = ?2, current_turn_id = ?3, updated_at_ms = ?4
             WHERE id = ?1",
            params![input.session_id, root_turn_id, turn_id, now],
        )?;

        insert_session_op(
            &tx,
            &input.session_id,
            "turn.appended",
            &json!({
                "turn_id": turn_id.clone(),
                "parent_turn_id": parent_turn_id.clone(),
                "user_parts": input.user_parts.clone(),
                "assistant_parts": input.assistant_parts.clone(),
                "conversation_state": input.conversation_state.clone(),
                "usage": input.usage.clone(),
            }),
            now,
        )?;

        let turn = tx.query_row(
            "SELECT
                id, session_id, parent_turn_id,
                user_parts_json, assistant_parts_json, conversation_state_json, usage_json, created_at_ms
             FROM turns
             WHERE id = ?1",
            params![turn_id],
            row_to_turn,
        )?;

        tx.commit()?;
        Ok(turn)
    }

    pub fn get_turn(&self, turn_id: &str) -> Result<Option<Turn>> {
        self.db
            .conn
            .query_row(
                "SELECT
                    id, session_id, parent_turn_id,
                    user_parts_json, assistant_parts_json, conversation_state_json, usage_json, created_at_ms
                 FROM turns
                 WHERE id = ?1",
                params![turn_id],
                row_to_turn,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn checkout_turn(&mut self, session_id: &str, turn_id: &str) -> Result<Session> {
        let now = now_ms();
        let tx = self.db.conn.transaction()?;

        ensure_session_exists(&tx, session_id)?;
        ensure_turn_belongs_to_session(&tx, session_id, turn_id)?;

        tx.execute(
            "UPDATE sessions
             SET current_turn_id = ?2, updated_at_ms = ?3
             WHERE id = ?1",
            params![session_id, turn_id, now],
        )?;

        insert_session_op(
            &tx,
            session_id,
            "session.checkout",
            &json!({ "turn_id": turn_id }),
            now,
        )?;

        let session = tx.query_row(
            "SELECT id, project_id, title, root_turn_id, current_turn_id, created_at_ms, updated_at_ms
             FROM sessions
             WHERE id = ?1",
            params![session_id],
            row_to_session,
        )?;
        tx.commit()?;
        Ok(session)
    }

    pub fn current_turn(&self, session_id: &str) -> Result<Option<Turn>> {
        self.db
            .conn
            .query_row(
                "SELECT
                    t.id, t.session_id, t.parent_turn_id,
                    t.user_parts_json, t.assistant_parts_json, t.conversation_state_json, t.usage_json, t.created_at_ms
                 FROM sessions s
                 JOIN turns t ON t.id = s.current_turn_id
                 WHERE s.id = ?1",
                params![session_id],
                row_to_turn,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn turn_path_to_current(&self, session_id: &str) -> Result<Vec<TurnPathItem>> {
        let mut stmt = self.db.conn.prepare(
            "WITH RECURSIVE chain(id, parent_turn_id, depth) AS (
                SELECT t.id, t.parent_turn_id, 0
                FROM turns t
                JOIN sessions s ON s.current_turn_id = t.id
                WHERE s.id = ?1
                UNION ALL
                SELECT p.id, p.parent_turn_id, chain.depth + 1
                FROM turns p
                JOIN chain ON chain.parent_turn_id = p.id
             )
             SELECT
                t.id, t.session_id, t.parent_turn_id,
                t.user_parts_json, t.assistant_parts_json, t.conversation_state_json, t.usage_json, t.created_at_ms,
                chain.depth
             FROM chain
             JOIN turns t ON t.id = chain.id
             ORDER BY chain.depth DESC",
        )?;

        let iter = stmt.query_map(params![session_id], |row| {
            let turn = row_to_turn(row)?;
            let depth: i64 = row.get(8)?;
            Ok(TurnPathItem {
                turn,
                depth: depth as u32,
            })
        })?;
        collect_rows(iter)
    }

    pub fn list_session_ops(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<SessionOp>> {
        let mut stmt = self.db.conn.prepare(
            "SELECT seq, session_id, op_type, payload_json, created_at_ms
             FROM session_ops
             WHERE session_id = ?1
               AND seq > COALESCE(?2, 0)
             ORDER BY seq ASC
             LIMIT ?3",
        )?;
        let iter = stmt.query_map(
            params![session_id, after_seq, limit as i64],
            row_to_session_op,
        )?;
        collect_rows(iter)
    }
}

fn ensure_project_exists(tx: &Transaction<'_>, project_id: &str) -> Result<()> {
    let exists = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
            params![project_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n != 0)?;
    if exists {
        Ok(())
    } else {
        Err(Error::ProjectNotFound(project_id.to_string()))
    }
}

fn ensure_session_exists(tx: &Transaction<'_>, session_id: &str) -> Result<()> {
    let exists = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n != 0)?;
    if exists {
        Ok(())
    } else {
        Err(Error::SessionNotFound(session_id.to_string()))
    }
}

fn ensure_turn_belongs_to_session(
    tx: &Transaction<'_>,
    session_id: &str,
    turn_id: &str,
) -> Result<()> {
    let owner = tx
        .query_row(
            "SELECT session_id FROM turns WHERE id = ?1",
            params![turn_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    match owner {
        None => Err(Error::TurnNotFound(turn_id.to_string())),
        Some(owner_session) if owner_session == session_id => Ok(()),
        Some(_) => Err(Error::TurnSessionMismatch {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
        }),
    }
}

fn insert_session_op(
    tx: &Transaction<'_>,
    session_id: &str,
    op_type: &str,
    payload: &serde_json::Value,
    created_at_ms: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO session_ops (session_id, op_type, payload_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            session_id,
            op_type,
            serde_json::to_string(payload)?,
            created_at_ms
        ],
    )?;
    Ok(())
}

fn generate_id(tx: &Transaction<'_>, prefix: &str) -> rusqlite::Result<String> {
    tx.query_row("SELECT lower(hex(randomblob(16)))", [], |row| {
        let suffix: String = row.get(0)?;
        Ok(format!("{prefix}_{suffix}"))
    })
}

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    let root_dir: String = row.get(1)?;
    Ok(Project {
        id: row.get(0)?,
        root_dir: PathBuf::from(root_dir),
        name: row.get(2)?,
        created_at_ms: row.get(3)?,
        updated_at_ms: row.get(4)?,
    })
}

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        root_turn_id: row.get(3)?,
        current_turn_id: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}

fn row_to_turn(row: &Row<'_>) -> rusqlite::Result<Turn> {
    Ok(Turn {
        id: row.get(0)?,
        session_id: row.get(1)?,
        parent_turn_id: row.get(2)?,
        user_parts: parse_json_column(row, 3)?,
        assistant_parts: parse_json_column(row, 4)?,
        conversation_state: parse_json_column(row, 5)?,
        usage: parse_optional_json_column(row, 6)?,
        created_at_ms: row.get(7)?,
    })
}

fn row_to_session_op(row: &Row<'_>) -> rusqlite::Result<SessionOp> {
    Ok(SessionOp {
        seq: row.get(0)?,
        session_id: row.get(1)?,
        op_type: row.get(2)?,
        payload: parse_json_column(row, 3)?,
        created_at_ms: row.get(4)?,
    })
}

fn parse_json_column(row: &Row<'_>, idx: usize) -> rusqlite::Result<serde_json::Value> {
    let raw: String = row.get(idx)?;
    serde_json::from_str(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(e)))
}

fn parse_optional_json_column(
    row: &Row<'_>,
    idx: usize,
) -> rusqlite::Result<Option<serde_json::Value>> {
    let raw: Option<String> = row.get(idx)?;
    match raw {
        None => Ok(None),
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(e))),
    }
}

fn collect_rows<T, F>(iter: rusqlite::MappedRows<'_, F>) -> Result<Vec<T>>
where
    F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut rows = Vec::new();
    for row in iter {
        rows.push(row?);
    }
    Ok(rows)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
