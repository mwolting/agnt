use std::path::Path;

use rusqlite::Connection;

use crate::error::Result;
use crate::migration;

pub(crate) struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        prepare_db_file(path)?;

        let mut conn = Connection::open(path)?;
        configure_connection(&conn)?;
        migration::apply(&mut conn)?;

        Ok(Self { conn })
    }

    pub(crate) fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure_connection(&conn)?;
        migration::apply(&mut conn)?;

        Ok(Self { conn })
    }
}

fn prepare_db_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(())
}
