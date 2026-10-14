//! `codex:run` requests and results (`CodexRunRequest` / `CodexRunResult`
//! in `shared/ipc.ts`, validated like `isCodexRunRequest`).

use anyhow::{Result, bail};
use bytes::Bytes;
use serde_json::{Value, json};

use crate::ids::is_project_id;
use crate::router::Call;

pub const PROMPT_CHARS: usize = 200_000;
pub const IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const IMAGES: usize = 12;
const ATTACHMENTS: usize = 6;
const ATTACHMENT_BYTES: usize = 100 * 1024;
const PROJECT_JSON_BYTES: usize = 32 * 1024 * 1024;
pub const GLOBAL_PROJECT: &str = "powermove-global";
pub const EFFORTS: [&str; 7] = ["none", "low", "medium", "high", "xhigh", "max", "ultra"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    ChatGpt,
    Claude,
    Compatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Editor,
    Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Editor,
    Project,
    Computer,
}

impl Access {
    /// The workspace authority (`authorityForAccess`).
    pub fn authority(self) -> &'static str {
        if self == Access::Computer { "computer" } else { "project" }
    }
}

/// `REQUEST_ID = /^[a-z0-9-]{8,80}$/`
pub fn is_request_id(value: &str) -> bool {
    (8..=80).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `/^[A-Za-z0-9_-]{1,120}$/`, the same rule as project ids.
fn is_thread_id(value: &str) -> bool {
    is_project_id(value)
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub id: String,
    pub provider: Provider,
    pub thread_id: Option<String>,
    pub mode: Mode,
    pub prompt: String,
    pub schema: Option<Value>,
    pub images: Vec<Bytes>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub access: Access,
    pub project_id: String,
    pub project_name: String,
    pub project_json: Option<String>,
    /// `context: 'app'`: app-wide extension work with no composition.
    pub app_context: bool,
    pub attachments: Vec<(String, Bytes)>,
    pub consent_token: Option<String>,
    pub supervised: bool,
}

fn optional_string(value: &Value, name: &str) -> Result<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => bail!("invalid {name}"),
    }
}

impl RunRequest {
    pub fn parse(call: &Call) -> Result<Self> {
        let value = call.arg(0);
        let text = |key: &str| value[key].as_str();
        let id = text("id").filter(|id| is_request_id(id)).ok_or_else(|| anyhow::anyhow!("invalid request id"))?.to_owned();
        let provider = match &value["provider"] {
            Value::Null => Provider::ChatGpt,
            Value::String(name) if name == "chatgpt" => Provider::ChatGpt,
            Value::String(name) if name == "claude" => Provider::Claude,
            Value::String(name) if name == "compatible" => Provider::Compatible,
            _ => bail!("invalid provider"),
        };
        let thread_id = match &value["threadId"] {
            Value::Null => None,
            Value::String(thread) if is_thread_id(thread) => Some(thread.clone()),
            _ => bail!("invalid threadId"),
        };
        let mode = match text("mode") {
            Some("editor") => Mode::Editor,
            Some("autonomous") => Mode::Autonomous,
            _ => bail!("invalid mode"),
        };
        let access = match text("access") {
            Some("editor") => Access::Editor,
            Some("project") => Access::Project,
            Some("computer") => Access::Computer,
            _ => bail!("invalid access"),
        };
        let app_context = match &value["context"] {
            Value::Null => false,
            Value::String(context) if context == "project" => false,
            Value::String(context) if context == "app" => true,
            _ => bail!("invalid context"),
        };
        let prompt = text("prompt").filter(|prompt| !prompt.is_empty() && prompt.chars().count() <= PROMPT_CHARS).ok_or_else(|| anyhow::anyhow!("invalid prompt"))?.to_owned();
        let schema = match &value["schema"] {
            Value::Null => None,
            schema @ Value::Object(_) => Some(schema.clone()),
            _ => bail!("invalid schema"),
        };
        let images = value["images"].as_array().filter(|images| images.len() <= IMAGES).ok_or_else(|| anyhow::anyhow!("invalid images"))?;
        let images = images.iter().map(|image| call.bytes(image)).collect::<Result<Vec<_>>>()?;
        if images.iter().any(|image| image.len() > IMAGE_BYTES) {
            bail!("invalid images");
        }
        let model = optional_string(&value["model"], "model")?;
        let reasoning_effort = optional_string(&value["reasoningEffort"], "reasoningEffort")?;
        if reasoning_effort.as_deref().is_some_and(|effort| !EFFORTS.contains(&effort)) {
            bail!("invalid reasoningEffort");
        }
        let project_id = text("projectId").filter(|id| is_project_id(id)).ok_or_else(|| anyhow::anyhow!("invalid projectId"))?.to_owned();
        let project_name = text("projectName").ok_or_else(|| anyhow::anyhow!("invalid projectName"))?.to_owned();
        let project_json = optional_string(&value["projectJSON"], "projectJSON")?;
        if project_json.as_ref().is_some_and(|json| json.len() > PROJECT_JSON_BYTES) {
            bail!("invalid projectJSON");
        }
        let attachments = value["attachments"].as_array().filter(|items| items.len() <= ATTACHMENTS).ok_or_else(|| anyhow::anyhow!("invalid attachments"))?;
        let attachments = attachments
            .iter()
            .map(|item| Ok((item["name"].as_str().ok_or_else(|| anyhow::anyhow!("invalid attachment"))?.to_owned(), call.bytes(&item["data"])?)))
            .collect::<Result<Vec<_>>>()?;
        if attachments.iter().any(|(_, data)| data.len() > ATTACHMENT_BYTES) {
            bail!("invalid attachments");
        }
        let consent_token = optional_string(&value["consentToken"], "consentToken")?;
        let supervised = match &value["approval"] {
            Value::Null => false,
            Value::String(approval) if approval == "supervised" => true,
            Value::String(approval) if approval == "auto" => false,
            _ => bail!("invalid approval"),
        };
        if app_context && (mode != Mode::Autonomous || access != Access::Project || project_id != GLOBAL_PROJECT || project_json.as_deref() != Some("{}")) {
            bail!("invalid app context");
        }
        if mode == Mode::Autonomous && project_json.is_none() {
            bail!("autonomous runs need a project snapshot");
        }
        Ok(Self { id, provider, thread_id, mode, prompt, schema, images, model, reasoning_effort, access, project_id, project_name, project_json, app_context, attachments, consent_token, supervised })
    }

    /// The project revision the run starts from (`projectRevision`).
    pub fn base_revision(&self) -> u64 {
        let Some(json) = &self.project_json else { return 0 };
        serde_json::from_str::<Value>(json).ok().and_then(|value| value["revision"].as_f64()).filter(|revision| revision.is_finite()).map_or(0, |revision| revision.max(0.0).trunc() as u64)
    }

    /// Thread conversations get their own workspace (`thread-<id>`).
    pub fn workspace_id(&self) -> String {
        match &self.thread_id {
            Some(thread) if thread.starts_with("agent-thread-") => format!("thread-{thread}"),
            // Parallel subagents get isolated workspaces (`subagent-<childThreadId>`).
            Some(thread) if self.id.starts_with("subagent-") => format!("subagent-{thread}"),
            _ => self.project_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunResult {
    Done {
        text: String,
        access: &'static str,
        live_edits: bool,
        live_history_id: Option<String>,
        /// The run's reported extension changes (`AgentExtensionChange[]`).
        extensions: Option<Value>,
        /// The undo record of the promoted extension changes.
        extension_change_set_id: Option<String>,
    },
    Failed {
        error: String,
        cancelled: bool,
    },
}

impl RunResult {
    pub fn done(text: String, access: &'static str) -> Self {
        Self::Done { text, access, live_edits: false, live_history_id: None, extensions: None, extension_change_set_id: None }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self::Failed { error: error.into(), cancelled: false }
    }

    pub fn cancelled(error: impl Into<String>) -> Self {
        Self::Failed { error: error.into(), cancelled: true }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Done { .. })
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Done { text, access, live_edits, live_history_id, extensions, extension_change_set_id } => {
                let mut value = json!({ "ok": true, "text": text, "access": access });
                if let Some(extensions) = extensions {
                    value["extensions"] = extensions.clone();
                }
                if let Some(id) = extension_change_set_id {
                    value["extensionChangeSetId"] = json!(id);
                }
                if *live_edits {
                    value["liveEditsApplied"] = json!(true);
                    if let Some(id) = live_history_id {
                        value["liveEditHistoryId"] = json!(id);
                    }
                }
                value
            }
            Self::Failed { error, cancelled } => json!({ "ok": false, "error": error, "cancelled": cancelled }),
        }
    }
}

/// `amendLiveResult`: drop final commands the live tools already applied, and add a warning note.
pub fn amend_live_result(text: &str, drop_commands: bool, warning: Option<&str>) -> String {
    let Ok(Value::Object(mut value)) = serde_json::from_str::<Value>(text) else { return text.to_owned() };
    let duplicated = drop_commands && value.get("commands").and_then(Value::as_array).is_some_and(|commands| !commands.is_empty());
    if !duplicated && warning.is_none() {
        return text.to_owned();
    }
    let mut notes: Vec<Value> = value.get("notes").and_then(Value::as_array).map(|notes| notes.iter().filter(|note| note.is_string()).cloned().collect()).unwrap_or_default();
    if drop_commands {
        value.insert("commands".into(), json!([]));
    }
    if duplicated {
        notes.push(json!("Powermove ignored final commands that duplicated edits already applied through the live tool transaction."));
    }
    if let Some(warning) = warning {
        notes.push(json!(warning));
    }
    value.insert("notes".into(), Value::Array(notes));
    Value::Object(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amends_live_results() {
        let text = r#"{"commands":[{"type":"x"}],"notes":["a"]}"#;
        let amended: Value = serde_json::from_str(&amend_live_result(text, true, Some("w"))).unwrap();
        assert_eq!(amended["commands"], json!([]));
        assert_eq!(amended["notes"].as_array().unwrap().len(), 3);
        assert_eq!(amend_live_result(r#"{"commands":[]}"#, true, None), r#"{"commands":[]}"#);
    }
}
