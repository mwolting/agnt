use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use super::hashline::{DEFAULT_READ_LIMIT, FileLines, MAX_READ_LIMIT, hashline};
use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};
use crate::tool::{Tool, ToolOutput};

const TOOL_DESCRIPTION: &str = include_str!("../../resources/tools/read.md");

#[derive(Clone, Deserialize)]
pub struct ReadInput {
    /// The file path to read, relative to the working directory.
    pub path: String,
    /// 0-based line offset to start reading from.
    pub offset: Option<usize>,
    /// Max number of lines to return.
    pub limit: Option<usize>,
}

impl Describe for ReadInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![
                Property {
                    name: "path".into(),
                    schema: Schema::String {
                        description: Some(
                            "File path to read, relative to the working directory".into(),
                        ),
                        enumeration: None,
                    },
                },
                Property {
                    name: "offset".into(),
                    schema: Schema::Integer {
                        description: Some("0-based line offset to start reading from".into()),
                    },
                },
                Property {
                    name: "limit".into(),
                    schema: Schema::Integer {
                        description: Some(format!(
                            "Maximum number of lines to return (default {DEFAULT_READ_LIMIT}, max {MAX_READ_LIMIT})"
                        )),
                    },
                },
            ],
            required: vec!["path".into()],
        }
    }
}

/// Structured output from reading a file.
pub struct ReadOutput {
    pub path: String,
    pub content: String,
    pub offset: usize,
    pub limit: usize,
    pub returned_lines: usize,
    pub total_lines: usize,
    pub has_more: bool,
}

impl ToolOutput for ReadOutput {
    fn to_llm(&self) -> String {
        let mut body = format!(
            "path: {}\nformat: line:hash|content\noffset: {}\nlimit: {}\nreturned_lines: {}\ntotal_lines: {}\nhas_more: {}",
            self.path,
            self.offset,
            self.limit,
            self.returned_lines,
            self.total_lines,
            self.has_more
        );

        if self.has_more {
            body.push_str(&format!(
                "\nnext_offset: {}",
                self.offset + self.returned_lines
            ));
        }

        if self.content.is_empty() {
            body.push_str("\n\n(no lines in requested range)");
        } else {
            body.push_str("\n\n");
            body.push_str(&self.content);
        }

        body
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
        TOOL_DESCRIPTION
    }

    async fn call(&self, input: ReadInput) -> Result<ReadOutput, agnt_llm::Error> {
        let path = self.cwd.join(&input.path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", path.display())))?;

        let lines = FileLines::parse(&content).lines;
        let total_lines = lines.len();
        let offset = input.offset.unwrap_or(0).min(total_lines);
        let requested_limit = input.limit.unwrap_or(DEFAULT_READ_LIMIT);
        if requested_limit == 0 {
            return Err(agnt_llm::Error::Other(
                "limit must be at least 1".to_string(),
            ));
        }

        let limit = requested_limit.min(MAX_READ_LIMIT);
        let end = offset.saturating_add(limit).min(total_lines);
        let returned_lines = end.saturating_sub(offset);
        let has_more = end < total_lines;

        let content = lines[offset..end]
            .iter()
            .enumerate()
            .map(|(i, line)| hashline(offset + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ReadOutput {
            path: input.path,
            content,
            offset,
            limit,
            returned_lines,
            total_lines,
            has_more,
        })
    }

    fn render_input(&self, input: &ReadInput) -> ToolCallDisplay {
        let offset = input.offset.unwrap_or(0);
        let limit = input.limit.unwrap_or(DEFAULT_READ_LIMIT);
        ToolCallDisplay {
            title: format!("Read {} (offset {}, limit {})", input.path, offset, limit),
            body: None,
        }
    }

    fn render_output(&self, _input: &ReadInput, output: &ReadOutput) -> ToolResultDisplay {
        let mut title = if output.returned_lines == 0 {
            format!(
                "0 lines (offset {} / {})",
                output.offset, output.total_lines
            )
        } else {
            let start = output.offset + 1;
            let end = output.offset + output.returned_lines;
            format!(
                "{} lines ({}-{} / {})",
                output.returned_lines, start, end, output.total_lines
            )
        };
        if output.has_more {
            title.push_str(" • more available");
        }

        ToolResultDisplay {
            title,
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
