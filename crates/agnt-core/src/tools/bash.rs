use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;
use tokio::process::Command;

use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};
use crate::tool::{Tool, ToolOutput};

#[derive(Clone, Deserialize)]
pub struct BashInput {
    /// The bash command to run.
    pub command: String,
}

impl Describe for BashInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![Property {
                name: "command".into(),
                schema: Schema::String {
                    description: Some("The bash command to run".into()),
                    enumeration: None,
                },
            }],
            required: vec!["command".into()],
        }
    }
}

/// Structured output from running a bash command.
pub struct BashOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl ToolOutput for BashOutput {
    fn to_llm(&self) -> String {
        let mut result = String::new();

        if !self.stdout.is_empty() {
            result.push_str(&self.stdout);
        }
        if !self.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr:\n");
            result.push_str(&self.stderr);
        }

        if let Some(code) = self.exit_code
            && code != 0
        {
            result.push_str(&format!("\n[exit code: {code}]"));
        }

        if result.is_empty() {
            result.push_str("(no output)");
        }

        result
    }
}

/// Tool that runs a bash command in the working directory and returns
/// stdout + stderr.
#[derive(Clone)]
pub struct BashTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for BashTool {
    type Input = BashInput;
    type Output = BashOutput;

    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a bash command and return the combined stdout and stderr."
    }

    async fn call(&self, input: BashInput) -> Result<BashOutput, agnt_llm::Error> {
        let output = Command::new("bash")
            .arg("-c")
            .arg(&input.command)
            .current_dir(&self.cwd)
            .output()
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("failed to spawn bash: {e}")))?;

        Ok(BashOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
        })
    }

    fn render_input(&self, input: &BashInput) -> ToolCallDisplay {
        ToolCallDisplay {
            title: format!("Run `{}`", input.command),
            body: None,
        }
    }

    fn render_output(&self, _input: &BashInput, output: &BashOutput) -> ToolResultDisplay {
        let title = match output.exit_code {
            Some(0) => "exit code 0".to_string(),
            Some(code) => format!("exit code {code}"),
            None => "killed by signal".to_string(),
        };

        // Show stdout as code, mention stderr in title if present.
        let body = if !output.stdout.is_empty() || !output.stderr.is_empty() {
            let mut content = String::new();
            if !output.stdout.is_empty() {
                content.push_str(&output.stdout);
            }
            if !output.stderr.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str("stderr:\n");
                content.push_str(&output.stderr);
            }
            Some(DisplayBody::Code {
                language: None,
                content,
            })
        } else {
            None
        };

        ToolResultDisplay { title, body }
    }
}
