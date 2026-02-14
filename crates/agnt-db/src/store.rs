use std::path::Path;

use crate::database::Database;
use crate::error::Result;
use crate::provider_credentials::ProviderCredentials;
use crate::sessions::Sessions;

pub struct Store {
    db: Database,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db: Database::open(path.as_ref())?,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            db: Database::open_in_memory()?,
        })
    }

    pub fn sessions(&mut self) -> Sessions<'_> {
        Sessions { db: &mut self.db }
    }

    pub fn provider_credentials(&mut self) -> ProviderCredentials<'_> {
        ProviderCredentials { db: &mut self.db }
    }
}
