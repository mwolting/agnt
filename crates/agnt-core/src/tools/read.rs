use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use crate::tool::Tool;

#[derive(Deserialize)]
pub struct ReadInput {
    /// The file path to read, relative to the working directory.
    pub path: String,
}

impl Describe for ReadInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![Property {
                name: "path".into(),
                schema: Schema::String {
                    description: Some(
                        "File path to read, relative to the working directory".into(),
                    ),
                    enumeration: None,
                },
            }],
            required: vec!["path".into()],
        }
    }
}

/// Tool that reads a file from disk relative to the working directory.
#[derive(Clone)]
pub struct ReadTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for ReadTool {
    type Input = ReadInput;

    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a file from disk. Returns the file contents as text."
    }

    async fn call(&self, input: ReadInput) -> Result<String, agnt_llm::Error> {
        let path = self.cwd.join(&input.path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;
        Ok(content)
    }
}
