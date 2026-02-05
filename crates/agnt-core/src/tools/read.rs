use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};
use crate::tool::{Tool, ToolOutput};

#[derive(Clone, Deserialize)]
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

/// Structured output from reading a file.
pub struct ReadOutput {
    pub path: String,
    pub content: String,
}

impl ToolOutput for ReadOutput {
    fn to_llm(&self) -> String {
        self.content.clone()
    }
}

/// Tool that reads a file from disk relative to the working directory.
#[derive(Clone)]
pub struct ReadTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for ReadTool {
    type Input = ReadInput;
    type Output = ReadOutput;

    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a file from disk. Returns the file contents as text."
    }

    async fn call(&self, input: ReadInput) -> Result<ReadOutput, agnt_llm::Error> {
        let path = self.cwd.join(&input.path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;
        Ok(ReadOutput {
            path: input.path,
            content,
        })
    }

    fn render_input(&self, input: &ReadInput) -> ToolCallDisplay {
        ToolCallDisplay {
            title: format!("Read {}", input.path),
            body: None,
        }
    }

    fn render_output(&self, _input: &ReadInput, output: &ReadOutput) -> ToolResultDisplay {
        let lines = output.content.lines().count();
        ToolResultDisplay {
            title: format!("{lines} lines"),
            body: Some(DisplayBody::Code {
                language: lang_from_ext(&output.path),
                content: output.content.clone(),
            }),
        }
    }
}

/// Guess a language name from a file extension for syntax highlighting.
fn lang_from_ext(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?;
    let lang = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "sh" | "bash" => "bash",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        "xml" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}
