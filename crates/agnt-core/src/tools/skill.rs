use std::fs;
use std::path::{Path, PathBuf};

use agnt_llm::{Describe, Property, Schema};
use serde::Deserialize;

use crate::event::{DisplayBody, ToolCallDisplay, ToolResultDisplay};
use crate::tool::Tool;

#[derive(Clone, Deserialize)]
pub struct SkillInput {
    /// Skill name to load from `.agents/skills`.
    pub name: String,
}

impl Describe for SkillInput {
    fn describe() -> Schema {
        Schema::Object {
            description: None,
            properties: vec![Property {
                name: "name".into(),
                schema: Schema::String {
                    description: Some("Skill name to load from .agents/skills".into()),
                    enumeration: None,
                },
            }],
            required: vec!["name".into()],
        }
    }
}

#[derive(Clone)]
struct SkillEntry {
    name: String,
    description: String,
    path: PathBuf,
}

#[derive(Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// Tool for loading local skills from `.agents/skills`.
#[derive(Clone)]
pub struct SkillTool {
    pub(crate) skills_dir: PathBuf,
    description: String,
}

impl SkillTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        let description = build_tool_description(&skills_dir);
        Self {
            skills_dir,
            description,
        }
    }
}

impl Tool for SkillTool {
    type Input = SkillInput;
    type Output = String;

    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        &self.description
    }

    async fn call(&self, input: SkillInput) -> Result<String, agnt_llm::Error> {
        let skills = discover_skills(&self.skills_dir)?;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(agnt_llm::Error::Other(
                "skill name cannot be empty".to_string(),
            ));
        }

        load_skill(&skills, name)
    }

    fn render_input(&self, input: &SkillInput) -> ToolCallDisplay {
        ToolCallDisplay {
            title: format!("Load skill {}", input.name.trim()),
            body: None,
        }
    }

    fn render_output(&self, _input: &SkillInput, output: &String) -> ToolResultDisplay {
        ToolResultDisplay {
            title: "Loaded skill".to_string(),
            body: Some(DisplayBody::Code {
                language: Some("markdown".to_string()),
                content: output.clone(),
            }),
        }
    }
}

fn build_tool_description(skills_dir: &Path) -> String {
    let mut description = String::from("Load a local skill by name from .agents/skills.");

    match discover_skills(skills_dir) {
        Ok(skills) if skills.is_empty() => {
            description.push_str(&format!(" No skills found in {}.", skills_dir.display()));
        }
        Ok(skills) => {
            description.push_str(" Available skills:\n");
            for skill in skills {
                description.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
        }
        Err(err) => {
            description.push_str(&format!(
                " Failed to index skills in {}: {err}",
                skills_dir.display()
            ));
        }
    }

    description
}

fn discover_skills(skills_dir: &Path) -> Result<Vec<SkillEntry>, agnt_llm::Error> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }
    if !skills_dir.is_dir() {
        return Err(agnt_llm::Error::Other(format!(
            "{} is not a directory",
            skills_dir.display()
        )));
    }

    let mut skills = Vec::new();
    let entries = fs::read_dir(skills_dir)
        .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", skills_dir.display())))?;

    for entry in entries {
        let entry = entry.map_err(|e| agnt_llm::Error::Other(e.to_string()))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md_path = path.join("SKILL.md");
        if !skill_md_path.is_file() {
            continue;
        }

        let content = fs::read_to_string(&skill_md_path)
            .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", skill_md_path.display())))?;
        let fallback_name = entry.file_name().to_string_lossy().into_owned();
        let (name, description) = parse_skill_metadata(&content, &fallback_name);

        skills.push(SkillEntry {
            name,
            description,
            path: skill_md_path,
        });
    }

    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

fn parse_skill_metadata(content: &str, fallback_name: &str) -> (String, String) {
    let (frontmatter, body) = match split_frontmatter(content) {
        Some((yaml, body)) => (serde_yaml::from_str::<SkillFrontmatter>(yaml).ok(), body),
        None => (None, content),
    };

    let name = frontmatter
        .as_ref()
        .and_then(|f| f.name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| fallback_name.to_string());

    let description = frontmatter
        .as_ref()
        .and_then(|f| f.description.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| first_body_line(body))
        .unwrap_or_else(|| "No description provided.".to_string());
    (name, description)
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let body_start = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;

    let mut index = 0usize;
    for line in body_start.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == "---" {
            let yaml = &body_start[..index];
            let body = &body_start[index + line.len()..];
            return Some((yaml, body));
        }
        index += line.len();
    }

    None
}

fn first_body_line(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }

    None
}

fn load_skill(skills: &[SkillEntry], name: &str) -> Result<String, agnt_llm::Error> {
    let selected = skills.iter().find(|skill| skill.name == name).or_else(|| {
        skills
            .iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(name))
    });

    let Some(skill) = selected else {
        let known = if skills.is_empty() {
            "(none)".to_string()
        } else {
            skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(agnt_llm::Error::Other(format!(
            "unknown skill `{name}`. Available skills: {known}"
        )));
    };

    let content = fs::read_to_string(&skill.path)
        .map_err(|e| agnt_llm::Error::Other(format!("{}: {e}", skill.path.display())))?;

    Ok(format!(
        "# {}\n\n{}\n\n---\nSource: {}",
        skill.name,
        content,
        skill.path.display()
    ))
}
