use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use crate::event::{ToolCallDisplay, ToolResultDisplay};
use crate::tool::{Tool, ToolOutput};

#[derive(Clone, Deserialize)]
pub struct EditInput {
    /// The file path to edit, relative to the working directory.
    pub path: String,
    /// The exact text to find in the file. Must match exactly once.
    pub old: String,
    /// The replacement text.
    pub new: String,
}

impl Describe for EditInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![
                Property {
                    name: "path".into(),
                    schema: Schema::String {
                        description: Some(
                            "File path to edit, relative to the working directory".into(),
                        ),
                        enumeration: None,
                    },
                },
                Property {
                    name: "old".into(),
                    schema: Schema::String {
                        description: Some(
                            "The exact text to find in the file. Must match exactly once.".into(),
                        ),
                        enumeration: None,
                    },
                },
                Property {
                    name: "new".into(),
                    schema: Schema::String {
                        description: Some("The replacement text".into()),
                        enumeration: None,
                    },
                },
            ],
            required: vec!["path".into(), "old".into(), "new".into()],
        }
    }
}

/// Structured output from editing a file.
pub struct EditOutput {
    pub path: String,
}

impl ToolOutput for EditOutput {
    fn to_llm(&self) -> String {
        format!("edited {}", self.path)
    }
}

/// Tool that performs an exact-match find-and-replace in a file.
/// The `old` string must appear exactly once in the file.
#[derive(Clone)]
pub struct EditTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for EditTool {
    type Input = EditInput;
    type Output = EditOutput;

    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact match of `old` with `new`. The `old` string must appear exactly once in the file."
    }

    async fn call(&self, input: EditInput) -> Result<EditOutput, agnt_llm::Error> {
        let path = self.cwd.join(&input.path);

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;

        let count = content.matches(&input.old).count();
        if count == 0 {
            return Err(agnt_llm::Error::Other(format!(
                "old string not found in {}",
                input.path
            )));
        }
        if count > 1 {
            return Err(agnt_llm::Error::Other(format!(
                "old string found {count} times in {} (must be exactly 1)",
                input.path
            )));
        }

        let new_content = content.replacen(&input.old, &input.new, 1);
        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;

        Ok(EditOutput { path: input.path })
    }

    fn render_input(&self, input: &EditInput) -> ToolCallDisplay {
        ToolCallDisplay {
            title: format!("Edit {}", input.path),
            body: None,
        }
    }

    fn render_output(&self, _input: &EditInput, output: &EditOutput) -> ToolResultDisplay {
        ToolResultDisplay {
            title: format!("Edited {}", output.path),
            body: None,
        }
    }
}
