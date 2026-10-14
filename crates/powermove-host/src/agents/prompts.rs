//! Prompt text and the editor tool catalog, generated from the TypeScript
//! sources by `scripts/windows/agent-prompts.ts` into
//! `<resources>/agent/prompts.json` (see `scripts/windows/stage-api-pack.ps1`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompts {
    instructions: Instructions,
    pub result_schema: Value,
    pub testing_instructions: String,
    pub shell_network_instructions: String,
    pub tools: Vec<ToolSpec>,
    pub app_tool_names: Vec<String>,
    pub inspection_tool_names: Vec<String>,
    pub outside_sandbox_tool: ToolSpec,
    pub outside_sandbox_call_ms: u64,
    /// `AGENT_ORCHESTRATION_INSTRUCTIONS`, added when a run can delegate.
    #[serde(default)]
    pub orchestration_instructions: String,
    /// Compatible-model system prompt parts (`AGENT_RESPONSE_STYLE`,
    /// `EFFECT_AUTHORING_INSTRUCTIONS`, `EDITOR_EXTENSION_INSTRUCTIONS`).
    #[serde(default)]
    pub response_style: String,
    #[serde(default)]
    pub effect_authoring_instructions: String,
    #[serde(default)]
    pub editor_extension_instructions: String,
}

#[derive(Debug, Deserialize)]
struct Instructions {
    /// access ("project" | "computer") → profile bits ("101") → template.
    project: HashMap<String, HashMap<String, String>>,
    app: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct InstructionInputs<'a> {
    pub project_name: &'a str,
    pub artifact_path: &'a str,
    pub access: &'a str,
    pub extensions_dir: &'a Path,
    pub app_context: bool,
    pub request: &'a str,
}

impl Prompts {
    pub fn load(resources_dir: &Path) -> Result<Self> {
        let file = resources_dir.join("agent").join("prompts.json");
        let text = std::fs::read_to_string(&file).with_context(|| format!("read {}; run scripts/windows/stage-api-pack.ps1", file.display()))?;
        serde_json::from_str(&text).context("invalid agent prompts")
    }

    /// `agentInstructions()` from `codex/instructions.ts`.
    pub fn agent_instructions(&self, inputs: &InstructionInputs) -> String {
        let template = if inputs.app_context {
            &self.instructions.app
        } else {
            let (extensions, footage, creative) = task_profile(inputs.request);
            let key = format!("{}{}{}", extensions as u8, footage as u8, creative as u8);
            let access = if inputs.access == "computer" { "computer" } else { "project" };
            match self.instructions.project.get(access).and_then(|profiles| profiles.get(&key)) {
                Some(template) => template,
                None => &self.instructions.app,
            }
        };
        fill(template, &[
            ("projectName", inputs.project_name),
            ("artifactPath", inputs.artifact_path),
            ("extensionsDir", &inputs.extensions_dir.display().to_string()),
        ])
    }

    pub fn tool(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

/// One pass over `{{pm:key}}` placeholders, so a value can never introduce another.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{pm:([A-Za-z]+)\}\}").unwrap());
    PLACEHOLDER
        .replace_all(template, |captures: &regex::Captures| {
            values.iter().find(|(key, _)| *key == &captures[1]).map(|(_, value)| (*value).to_owned()).unwrap_or_default()
        })
        .into_owned()
}

/// `agentTaskProfile()` from `shared/agent-guidance.ts`: (extensions, footage, creative).
pub fn task_profile(request: &str) -> (bool, bool, bool) {
    static BRIEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\n(?:CONVERSATION|SELECTED REGION|SEMANTIC WORKSPACE|LIVE COMPOSITION)").unwrap());
    static EXTENSIONS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i-u)\b(extensions?|plugins?|mods?|effects?|shaders?|integrations?|sandbox|panels?|workspaces?|tools?)\b").unwrap());
    static FOOTAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i-u)\b(video|audio|footage|clip|cut|trim|speech|caption|transcrib|music|voice|sound)").unwrap());
    static CREATIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i-u)\b(animate|animation|motion|scene|composition|design|title|text|shape|style|polish|logo|transition|effect|video|footage|caption|color|colour|spacing|timing|pace|pacing|make|create|refine|improve|finish|build)\b").unwrap());
    let brief = BRIEF.split(request).next().unwrap_or_default();
    (EXTENSIONS.is_match(brief), FOOTAGE.is_match(brief), CREATIVE.is_match(brief))
}

/// The files of `<resources>/api-pack`, as relative POSIX names and text.
pub fn api_pack_files(resources_dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, directory: &Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(directory) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, out);
            } else if let (Ok(relative), Ok(text)) = (path.strip_prefix(root), std::fs::read_to_string(&path)) {
                let name = relative.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                out.push((name, text));
            }
        }
    }
    let root = resources_dir.join("api-pack");
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_requests_like_the_renderer() {
        assert_eq!(task_profile("Trim the clip"), (false, true, false));
        assert_eq!(task_profile("animate the logo"), (false, false, true));
        assert_eq!(task_profile("write a panel extension"), (true, false, false));
        assert_eq!(task_profile("hello\n\nCONVERSATION\nvideo"), (false, false, false));
    }

    #[test]
    fn fills_placeholders_in_one_pass() {
        let text = fill("a {{pm:projectName}} b {{pm:artifactPath}}", &[("projectName", "{{pm:artifactPath}}"), ("artifactPath", "x")]);
        assert_eq!(text, "a {{pm:artifactPath}} b x");
    }
}
