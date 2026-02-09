use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredCredential {
    ApiKey {
        api_key: String,
    },
    OAuthPkce {
        access_token: String,
        refresh_token: String,
        expires_at_ms: u64,
        #[serde(default)]
        metadata: HashMap<String, String>,
    },
}

pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn load(&self, provider_id: &str) -> Result<Option<StoredCredential>, Error> {
        let entry = keyring::Entry::new(&self.service, provider_id)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(serde_json::from_str(&value)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, provider_id: &str, credential: &StoredCredential) -> Result<(), Error> {
        let entry = keyring::Entry::new(&self.service, provider_id)?;
        let value = serde_json::to_string(credential)?;
        entry.set_password(&value)?;
        Ok(())
    }
}
