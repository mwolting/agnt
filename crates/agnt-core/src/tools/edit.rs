use std::io::ErrorKind;

use agnt_llm::{Describe, Schema};
use serde::Deserialize;
use similar::{ChangeTag, TextDiff};

use super::hashline::{FileLines, hashline, replacement_lines, resolve_anchor};
use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};
use crate::tool::{Tool, ToolOutput};

const TOOL_DESCRIPTION: &str = include_str!("../../resources/tools/edit.md");

#[derive(Clone, Deserialize)]
pub struct EditInput {
    /// The file path to edit, relative to the working directory.
    pub path: String,
    /// Ordered edit operations.
    pub operations: Vec<EditOperation>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOperation {
    Replace {
        /// Hashline anchor in `line:hash` format.
        anchor: String,
        /// Replacement content (can be multi-line).
        content: String,
    },
    InsertBefore {
        /// Hashline anchor in `line:hash` format.
        anchor: String,
        /// Content to insert before the anchor line (can be multi-line).
        content: String,
    },
    InsertAfter {
        /// Hashline anchor in `line:hash` format.
        anchor: String,
        /// Content to insert after the anchor line (can be multi-line).
        content: String,
    },
    Delete {
        /// Hashline anchor in `line:hash` format.
        anchor: String,
    },
    ReplaceRange {
        /// Start hashline anchor in `line:hash` format.
        start: String,
        /// End hashline anchor in `line:hash` format.
        end: String,
        /// Replacement content (can be multi-line).
        content: String,
    },
    DeleteRange {
        /// Start hashline anchor in `line:hash` format.
        start: String,
        /// End hashline anchor in `line:hash` format.
        end: String,
    },
    RewriteFile {
        /// Full file content to write (creates or replaces the file).
        content: String,
    },
    MoveFile {
        /// Destination path, relative to the working directory.
        to: String,
    },
    DeleteFile,
}

impl EditOperation {
    fn kind(&self) -> &'static str {
        match self {
            Self::Replace { .. } => "replace",
            Self::InsertBefore { .. } => "insert_before",
            Self::InsertAfter { .. } => "insert_after",
            Self::Delete { .. } => "delete",
            Self::ReplaceRange { .. } => "replace_range",
            Self::DeleteRange { .. } => "delete_range",
            Self::RewriteFile { .. } => "rewrite_file",
            Self::MoveFile { .. } => "move_file",
            Self::DeleteFile => "delete_file",
        }
    }
}

impl Describe for EditInput {
    fn describe() -> Schema {
        Schema::Raw(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to edit, relative to the working directory"
                },
                "operations": {
                    "type": "array",
                    "description": "Ordered list of edit operations",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["replace"] },
                                    "anchor": { "type": "string", "description": "Hashline anchor in line:hash format" },
                                    "content": { "type": "string", "description": "Replacement content (multi-line allowed)" }
                                },
                                "required": ["op", "anchor", "content"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["insert_before"] },
                                    "anchor": { "type": "string", "description": "Hashline anchor in line:hash format" },
                                    "content": { "type": "string", "description": "Content to insert (multi-line allowed)" }
                                },
                                "required": ["op", "anchor", "content"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["insert_after"] },
                                    "anchor": { "type": "string", "description": "Hashline anchor in line:hash format" },
                                    "content": { "type": "string", "description": "Content to insert (multi-line allowed)" }
                                },
                                "required": ["op", "anchor", "content"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["delete"] },
                                    "anchor": { "type": "string", "description": "Hashline anchor in line:hash format" }
                                },
                                "required": ["op", "anchor"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["replace_range"] },
                                    "start": { "type": "string", "description": "Start hashline anchor in line:hash format" },
                                    "end": { "type": "string", "description": "End hashline anchor in line:hash format" },
                                    "content": { "type": "string", "description": "Replacement content (multi-line allowed)" }
                                },
                                "required": ["op", "start", "end", "content"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["delete_range"] },
                                    "start": { "type": "string", "description": "Start hashline anchor in line:hash format" },
                                    "end": { "type": "string", "description": "End hashline anchor in line:hash format" }
                                },
                                "required": ["op", "start", "end"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["rewrite_file"] },
                                    "content": { "type": "string", "description": "Full file content to write" }
                                },
                                "required": ["op", "content"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["move_file"] },
                                    "to": { "type": "string", "description": "Destination path, relative to the working directory" }
                                },
                                "required": ["op", "to"]
                            },
                            {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "op": { "type": "string", "enum": ["delete_file"] }
                                },
                                "required": ["op"]
                            }
                        ]
                    }
                }
            },
            "required": ["path", "operations"]
        }))
    }
}

/// Structured output from editing a file.
pub struct EditOutput {
    pub input_path: String,
    pub path: String,
    pub deleted: bool,
    pub operations_applied: usize,
    pub final_diff: String,
}

impl ToolOutput for EditOutput {
    fn to_llm(&self) -> String {
        let summary = if self.deleted {
            format!("deleted {}", self.path)
        } else if self.input_path != self.path {
            format!(
                "edited {} -> {} with {} operation(s)",
                self.input_path, self.path, self.operations_applied
            )
        } else {
            format!(
                "edited {} with {} operation(s)",
                self.path, self.operations_applied
            )
        };

        if self.final_diff.is_empty() {
            summary
        } else {
            format!(
                "{summary}\n\nfinal diff (hashline-formatted):\n{}",
                self.final_diff
            )
        }
    }
}

/// Tool that applies hashline-anchored and file-level edit operations.
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
        TOOL_DESCRIPTION
    }

    async fn call(&self, input: EditInput) -> Result<EditOutput, agnt_llm::Error> {
        if input.operations.is_empty() {
            return Err(agnt_llm::Error::Other(
                "operations must contain at least one entry".to_string(),
            ));
        }

        let input_path = input.path.trim();
        if input_path.is_empty() {
            return Err(agnt_llm::Error::Other("path cannot be empty".to_string()));
        }

        let mut state = EditState::load(self.cwd.clone(), input_path).await?;
        let initial_snapshot = snapshot_state(&state);
        for (idx, operation) in input.operations.iter().enumerate() {
            apply_operation(operation, &mut state).map_err(|err| {
                agnt_llm::Error::Other(format!(
                    "operation {} ({}) failed: {err}",
                    idx + 1,
                    operation.kind()
                ))
            })?;
        }

        let deleted = state.file.is_none();
        let final_path = state.current_path.clone();
        let final_snapshot = snapshot_state(&state);
        let final_diff = render_unified_patch(&initial_snapshot, &final_snapshot);
        state.persist().await?;

        Ok(EditOutput {
            input_path: input_path.to_string(),
            path: final_path,
            deleted,
            operations_applied: input.operations.len(),
            final_diff,
        })
    }

    fn render_input(&self, input: &EditInput) -> ToolCallDisplay {
        ToolCallDisplay {
            title: format!(
                "Edit {} ({} operations)",
                input.path,
                input.operations.len()
            ),
            body: None,
        }
    }

    fn render_output(&self, _input: &EditInput, output: &EditOutput) -> ToolResultDisplay {
        let title = if output.deleted {
            format!("Deleted {}", output.path)
        } else if output.input_path != output.path {
            format!(
                "Edited {} -> {} ({} operations)",
                output.input_path, output.path, output.operations_applied
            )
        } else {
            format!(
                "Edited {} ({} operations)",
                output.path, output.operations_applied
            )
        };

        let body = render_diff_body(&output.final_diff);
        ToolResultDisplay { title, body }
    }
}

struct EditState {
    cwd: std::path::PathBuf,
    input_path: String,
    current_path: String,
    initial_file_existed: bool,
    file: Option<FileLines>,
}

impl EditState {
    async fn load(cwd: std::path::PathBuf, path: &str) -> Result<Self, agnt_llm::Error> {
        let abs_path = cwd.join(path);
        let file = read_file_if_exists(&abs_path).await?;
        Ok(Self {
            cwd,
            input_path: path.to_string(),
            current_path: path.to_string(),
            initial_file_existed: file.is_some(),
            file,
        })
    }

    async fn persist(&mut self) -> Result<(), agnt_llm::Error> {
        let input_abs = self.cwd.join(&self.input_path);
        let final_abs = self.cwd.join(&self.current_path);
        let moved = input_abs != final_abs;

        match self.file.as_mut() {
            Some(file) => {
                if moved && path_exists(&final_abs).await? {
                    return Err(agnt_llm::Error::Other(format!(
                        "destination already exists: {}",
                        self.current_path
                    )));
                }

                if file.lines.is_empty() {
                    file.trailing_newline = false;
                }

                if let Some(parent) = final_abs.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        agnt_llm::Error::Other(format!("{}: {e}", parent.display()))
                    })?;
                }

                tokio::fs::write(&final_abs, file.render())
                    .await
                    .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", final_abs.display())))?;

                if moved && self.initial_file_existed {
                    remove_file_if_exists(&input_abs).await?;
                }
            }
            None => {
                if self.initial_file_existed {
                    remove_file_if_exists(&input_abs).await?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
struct FileSnapshot {
    path: String,
    exists: bool,
    lines: Vec<String>,
}

fn snapshot_state(state: &EditState) -> FileSnapshot {
    match &state.file {
        Some(file) => FileSnapshot {
            path: state.current_path.clone(),
            exists: true,
            lines: file.lines.clone(),
        },
        None => FileSnapshot {
            path: state.current_path.clone(),
            exists: false,
            lines: Vec::new(),
        },
    }
}

fn render_diff_body(diff: &str) -> Option<DisplayBody> {
    if diff.is_empty() {
        None
    } else {
        Some(DisplayBody::Diff(diff.to_string()))
    }
}

// 5 lines of context means nearby hunks separated by <10 unchanged lines naturally coalesce.
const HUNK_CONTEXT_LINES: usize = 5;

fn render_unified_patch(before: &FileSnapshot, after: &FileSnapshot) -> String {
    let mut patch = String::new();
    patch.push_str(&format!(
        "--- {}\n",
        diff_label("a", &before.path, before.exists)
    ));
    patch.push_str(&format!(
        "+++ {}\n",
        diff_label("b", &after.path, after.exists)
    ));

    let before_text = before.lines.join("\n");
    let after_text = after.lines.join("\n");
    let diff = TextDiff::from_lines(&before_text, &after_text);
    let groups = diff.grouped_ops(HUNK_CONTEXT_LINES);

    if groups.is_empty() {
        patch.push_str("@@ -0,0 +0,0 @@\n");
        patch.push_str(" (no content changes)\n");
        return patch;
    }

    for group in groups {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let old_start = first.old_range().start;
        let old_end = last.old_range().end;
        let new_start = first.new_range().start;
        let new_end = last.new_range().end;
        let old_len = old_end.saturating_sub(old_start);
        let new_len = new_end.saturating_sub(new_start);

        patch.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk_start_line(old_start, old_len),
            old_len,
            hunk_start_line(new_start, new_len),
            new_len
        ));

        for op in group {
            for change in diff.iter_changes(&op) {
                let line = change_line_content(change.value());
                match change.tag() {
                    ChangeTag::Equal => {
                        if let Some(old_idx) = change.old_index() {
                            patch.push(' ');
                            patch.push_str(&hashline(old_idx + 1, line));
                            patch.push('\n');
                        }
                    }
                    ChangeTag::Delete => {
                        if let Some(old_idx) = change.old_index() {
                            patch.push('-');
                            patch.push_str(&hashline(old_idx + 1, line));
                            patch.push('\n');
                        }
                    }
                    ChangeTag::Insert => {
                        if let Some(new_idx) = change.new_index() {
                            patch.push('+');
                            patch.push_str(&hashline(new_idx + 1, line));
                            patch.push('\n');
                        }
                    }
                }
            }
        }
    }

    patch
}

fn hunk_start_line(start: usize, count: usize) -> usize {
    if count == 0 { start } else { start + 1 }
}

fn change_line_content(value: &str) -> &str {
    value.trim_end_matches('\n').trim_end_matches('\r')
}

fn diff_label(prefix: &str, path: &str, exists: bool) -> String {
    if exists {
        format!("{prefix}/{path}")
    } else {
        "/dev/null".to_string()
    }
}
fn apply_operation(operation: &EditOperation, state: &mut EditState) -> Result<(), String> {
    match operation {
        EditOperation::Replace { .. }
        | EditOperation::InsertBefore { .. }
        | EditOperation::InsertAfter { .. }
        | EditOperation::Delete { .. }
        | EditOperation::ReplaceRange { .. }
        | EditOperation::DeleteRange { .. } => {
            let file = state
                .file
                .as_mut()
                .ok_or_else(|| format!("`{}` does not exist", state.current_path))?;
            apply_line_operation(operation, &mut file.lines)
        }
        EditOperation::RewriteFile { content } => {
            state.file = Some(FileLines::parse(content));
            Ok(())
        }
        EditOperation::MoveFile { to } => {
            if state.file.is_none() {
                return Err(format!("cannot move missing file `{}`", state.current_path));
            }
            let destination = to.trim();
            if destination.is_empty() {
                return Err("destination path cannot be empty".to_string());
            }
            state.current_path = destination.to_string();
            Ok(())
        }
        EditOperation::DeleteFile => {
            if state.file.is_none() {
                return Err(format!(
                    "cannot delete missing file `{}`",
                    state.current_path
                ));
            }
            state.file = None;
            Ok(())
        }
    }
}

fn apply_line_operation(operation: &EditOperation, lines: &mut Vec<String>) -> Result<(), String> {
    match operation {
        EditOperation::Replace { anchor, content } => {
            let idx = resolve_anchor(anchor, lines)?;
            lines.splice(idx..=idx, replacement_lines(content));
        }
        EditOperation::InsertBefore { anchor, content } => {
            let idx = resolve_anchor(anchor, lines)?;
            lines.splice(idx..idx, replacement_lines(content));
        }
        EditOperation::InsertAfter { anchor, content } => {
            let idx = resolve_anchor(anchor, lines)?;
            lines.splice(idx + 1..idx + 1, replacement_lines(content));
        }
        EditOperation::Delete { anchor } => {
            let idx = resolve_anchor(anchor, lines)?;
            lines.remove(idx);
        }
        EditOperation::ReplaceRange {
            start,
            end,
            content,
        } => {
            let (start_idx, end_idx) = resolve_range(start, end, lines)?;
            lines.splice(start_idx..=end_idx, replacement_lines(content));
        }
        EditOperation::DeleteRange { start, end } => {
            let (start_idx, end_idx) = resolve_range(start, end, lines)?;
            lines.drain(start_idx..=end_idx);
        }
        _ => unreachable!("file-level operation routed to line-operation handler"),
    }

    Ok(())
}

fn resolve_range(start: &str, end: &str, lines: &[String]) -> Result<(usize, usize), String> {
    let start_idx = resolve_anchor(start, lines)?;
    let end_idx = resolve_anchor(end, lines)?;
    if start_idx > end_idx {
        return Err(format!(
            "range anchors are reversed (`{start}` resolves after `{end}`)"
        ));
    }
    Ok((start_idx, end_idx))
}

async fn read_file_if_exists(path: &std::path::Path) -> Result<Option<FileLines>, agnt_llm::Error> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(FileLines::parse(&content))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(agnt_llm::Error::Other(format!("{}: {err}", path.display()))),
    }
}

async fn path_exists(path: &std::path::Path) -> Result<bool, agnt_llm::Error> {
    match tokio::fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(agnt_llm::Error::Other(format!("{}: {err}", path.display()))),
    }
}

async fn remove_file_if_exists(path: &std::path::Path) -> Result<(), agnt_llm::Error> {
    match tokio::fs::remove_file(path).await {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(agnt_llm::Error::Other(format!("{}: {err}", path.display()))),
    }
}
