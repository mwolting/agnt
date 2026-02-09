//! Provider model metadata sources.

use std::sync::Arc;

use crate::error::Error;
use crate::spec::ModelSpec;

/// Provider model metadata source.
#[derive(Clone, Default)]
pub enum ModelSource {
    /// Resolve models from models.dev.
    #[default]
    ModelsDev,
    /// Statically declared model metadata.
    Static(Vec<ModelSpec>),
    /// Dynamic model loader callback.
    Dynamic(Arc<dyn ModelLoader>),
}

impl std::fmt::Debug for ModelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelSource::ModelsDev => f.write_str("ModelsDev"),
            ModelSource::Static(models) => f
                .debug_tuple("Static")
                .field(&format!("{} models", models.len()))
                .finish(),
            ModelSource::Dynamic(_) => f.write_str("Dynamic(<loader>)"),
        }
    }
}

/// Callback used for dynamic model metadata resolution.
pub trait ModelLoader: Send + Sync {
    fn load_models(&self, provider_id: &str) -> Result<Vec<ModelSpec>, Error>;
}

impl<F> ModelLoader for F
where
    F: Fn(&str) -> Result<Vec<ModelSpec>, Error> + Send + Sync,
{
    fn load_models(&self, provider_id: &str) -> Result<Vec<ModelSpec>, Error> {
        (self)(provider_id)
    }
}
