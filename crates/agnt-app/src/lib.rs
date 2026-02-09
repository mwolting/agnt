use std::path::PathBuf;

use directories::ProjectDirs;

const APP_QUALIFIER: &str = "dev";
const APP_ORGANIZATION: &str = "agnt";
const APP_NAME: &str = "agnt";
const SESSION_DB_FILENAME: &str = "sessions.sqlite3";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not resolve user data directory")]
    MissingUserDataDir,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// App-local user data directory (for durable application state).
pub fn user_data_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
        .ok_or(Error::MissingUserDataDir)?;
    Ok(dirs.data_local_dir().to_path_buf())
}

pub fn ensure_user_data_dir() -> Result<PathBuf> {
    let dir = user_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn session_db_path() -> Result<PathBuf> {
    Ok(ensure_user_data_dir()?.join(SESSION_DB_FILENAME))
}
