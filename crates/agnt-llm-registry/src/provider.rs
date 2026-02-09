//! Provider registration metadata.

use crate::auth::AuthMethod;
use crate::model_source::ModelSource;
use serde::Serialize;
use serde_json::Value;

/// Provider registration metadata.
#[derive(Debug, Clone)]
pub struct ProviderRegistration {
    pub id: String,
    pub name: String,
    /// Compatible npm package names for models routed through factories.
    pub npm_packages: Vec<String>,
    pub api_endpoint: Option<String>,
    /// Provider-specific options, passed through to provider factories.
    pub(crate) factory_options: Option<Value>,
    pub auth_method: AuthMethod,
    pub model_source: ModelSource,
}

impl ProviderRegistration {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            npm_packages: Vec::new(),
            api_endpoint: None,
            factory_options: None,
            auth_method: AuthMethod::default(),
            model_source: ModelSource::default(),
        }
    }

    /// Serialize and store provider-specific factory options.
    pub fn set_factory_options<T>(&mut self, options: &T) -> Result<(), serde_json::Error>
    where
        T: Serialize + ?Sized,
    {
        self.factory_options = Some(serde_json::to_value(options)?);
        Ok(())
    }
}
