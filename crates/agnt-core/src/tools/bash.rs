use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;
use tokio::process::Command;

use crate::tool::Tool;

#[derive(Deserialize)]
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

/// Tool that runs a bash command in the working directory and returns
/// stdout + stderr.
#[derive(Clone)]
pub struct BashTool {
    pub(crate) cwd: std::path::PathBuf,
}

impl Tool for BashTool {
    type Input = BashInput;

    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a bash command and return the combined stdout and stderr."
    }

    async fn call(&self, input: BashInput) -> Result<String, agnt_llm::Error> {
        let output = Command::new("bash")
            .arg("-c")
            .arg(&input.command)
            .current_dir(&self.cwd)
            .output()
            .await
            .map_err(|e| agnt_llm::Error::Other(format!("failed to spawn bash: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr:\n");
            result.push_str(&stderr);
        }

        if !output.status.success() {
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into());
            result.push_str(&format!("\n[exit code: {code}]"));
        }

        if result.is_empty() {
            result.push_str("(no output)");
        }

        Ok(result)
    }
}
