use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::database::Database;
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    pub provider_id: String,
    pub credential_value: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub struct ProviderCredentials<'db> {
    pub(crate) db: &'db mut Database,
}

impl ProviderCredentials<'_> {
    pub fn get(&self, provider_id: &str) -> Result<Option<String>> {
        self.db
            .conn
            .query_row(
                "SELECT credential_value
                 FROM provider_credentials
                 WHERE provider_id = ?1",
                params![provider_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_record(&self, provider_id: &str) -> Result<Option<ProviderCredential>> {
        self.db
            .conn
            .query_row(
                "SELECT provider_id, credential_value, created_at_ms, updated_at_ms
                 FROM provider_credentials
                 WHERE provider_id = ?1",
                params![provider_id],
                |row| {
                    Ok(ProviderCredential {
                        provider_id: row.get(0)?,
                        credential_value: row.get(1)?,
                        created_at_ms: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert(&mut self, provider_id: &str, credential_value: &str) -> Result<()> {
        let now = now_ms();
        self.db.conn.execute(
            "INSERT INTO provider_credentials (
                provider_id, credential_value, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id) DO UPDATE SET
                credential_value = excluded.credential_value,
                updated_at_ms = excluded.updated_at_ms",
            params![provider_id, credential_value, now, now],
        )?;
        Ok(())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
