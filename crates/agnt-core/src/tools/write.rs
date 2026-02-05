use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use crate::tool::Tool;

#[derive(Deserialize)]
pub struct WriteInput {
    /// The file path to write, relative to the working directory.
    pub path: String,
    /// The full content to write to the file (replaces existing content).
    pub content: String,
}

impl Describe for WriteInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![
                Property {
                    name: "path".into(),
                    schema: Schema::String {
                        description: Some(
                            "File path to write, relative to the working directory".into(),
                        ),
                        enumeration: None,
                    },
                },
                Property {
                    name: "content".into(),
                    schema: Schema::String {
                        description: Some(
                            "The full content to write to the file (replaces existing content)"
                                .into(),
                        ),
                        enumeration: None,
                    },
                },
            ],
            required: vec!["path".into(), "content".into()],
        }
    }
}

/// Tool that writes (or overwrites) a file on disk.
#[derive(Clone)]
pub struct WriteTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for WriteTool {
    type Input = WriteInput;

    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, or replaces its content if it does. Creates parent directories as needed."
    }

    async fn call(&self, input: WriteInput) -> Result<String, agnt_llm::Error> {
        let path = self.cwd.join(&input.path);

        // Create parent directories if they don't exist.
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", parent.display())))?;
        }

        tokio::fs::write(&path, &input.content)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;

        Ok(format!("wrote {} bytes to {}", input.content.len(), input.path))
    }
}
