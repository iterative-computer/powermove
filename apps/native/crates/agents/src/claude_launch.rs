//! Native Claude Code launch contract from owner-pinned main c7f46a056.
//! The host supplies its existing isolated home, native MCP descriptor, model
//! discovery and instruction text. No Node launcher or auth migration is used.
use crate::{Access, ApprovalMode, approvals::Authority, claude, transport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub schema: Value,
    pub prompt: String,
    pub image_paths: Vec<String>,
    pub model: Option<String>,
    /// Already normalized by the provider's discovered effort catalog.
    pub effort: Option<String>,
    pub session_id: Option<String>,
    pub extensions_dir: Option<String>,
    pub instructions: String,
    pub additional_instructions: Option<String>,
    pub shell_network_instructions: String,
    pub native_tools: Option<Value>,
    pub tool_names: Vec<String>,
    pub inspection_tool_names: Vec<String>,
    pub external_mcp_servers: Value,
    /// True only when the host can display and settle fallback approval cards.
    pub ask_outside_sandbox: bool,
}
const PROJECT_TOOLS: &str = "Read,Glob,Grep,Bash,WebSearch,WebFetch,Skill,Agent,Task";
const EDITOR_TOOLS: &str = "Read,Glob,Grep,Skill,Agent,Task";
const DISALLOWED: &str = "Bash,Monitor,PowerShell,Write,Edit,NotebookEdit";
const ASKING: &str = "APPROVALS\nThe sandbox confines writes to this workspace. When a command genuinely needs to run outside it, such as installing a font or writing to the user's home folder, retry that one command with dangerouslyDisableSandbox: true and a clear description. Powermove asks the user to approve it; if they decline, do not retry it.";
const AUTO: &str = "APPROVALS\nThe sandbox confines writes to this workspace. When a command genuinely needs to run outside it, such as installing a font or writing to the user's home folder, retry that one command with dangerouslyDisableSandbox: true and a clear description. An automatic reviewer approves routine requests and may ask the user; if a request is declined, do not retry it.";
impl Options {
    pub fn argv(&self, authority: &Authority) -> Result<Vec<String>, String> {
        if self.instructions.is_empty() || !self.schema.is_object() {
            return Err("Claude requires run instructions and a result schema".into());
        }
        let mut external = self
            .external_mcp_servers
            .as_object()
            .cloned()
            .ok_or("Invalid external MCP servers")?;
        external.remove("powermove");
        let external_tools = external
            .keys()
            .map(|n| format!("mcp__{n}__*"))
            .collect::<Vec<_>>();
        if let Some(native) = &self.native_tools {
            let mut server = json!({"type":"stdio"});
            server.as_object_mut().unwrap().extend(
                native
                    .as_object()
                    .ok_or("Invalid native MCP descriptor")?
                    .clone(),
            );
            external.insert("powermove".into(), server);
        }
        let asking = authority.access == Access::Project && self.ask_outside_sandbox;
        let auto = authority.access == Access::Project && authority.approval == ApprovalMode::Auto;
        let mut args = vec![
            "--print".into(),
            "--input-format".into(),
            "stream-json".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--permission-prompt-tool".into(),
            "stdio".into(),
            "--include-partial-messages".into(),
            "--verbose".into(),
            "--mcp-config".into(),
            json!({"mcpServers":external}).to_string(),
            "--json-schema".into(),
            self.schema.to_string(),
        ];
        for (flag, value) in [("--model", &self.model), ("--effort", &self.effort)] {
            if let Some(value) = value.as_ref().filter(|s| !s.is_empty()) {
                args.extend([flag.into(), value.clone()]);
            }
        }
        if authority.access == Access::Editor {
            args.push("--no-session-persistence".into());
        }
        if let Some(session) = self.session_id.as_ref().filter(|s| !s.is_empty()) {
            args.extend(["--resume".into(), session.clone()]);
        }
        if authority.computer_allowed() {
            args.extend([
                "--dangerously-skip-permissions".into(),
                "--tools".into(),
                "default".into(),
            ]);
        } else {
            let editor = authority.access == Access::Editor;
            let mut sandbox = json!({"enabled":true,"autoAllowBashIfSandboxed":!editor,
                "allowUnsandboxedCommands":asking,"failIfUnavailable":true});
            if !editor {
                sandbox["network"] = json!({"allowedDomains":["*"],"allowLocalBinding":true});
            }
            let mut tools = if editor {
                EDITOR_TOOLS.to_owned()
            } else if asking {
                PROJECT_TOOLS
                    .split(',')
                    .filter(|t| *t != "Bash")
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                PROJECT_TOOLS.to_owned()
            };
            if self.native_tools.is_some() {
                tools.push(',');
                tools.push_str(
                    &if editor {
                        &self.inspection_tool_names
                    } else {
                        &self.tool_names
                    }
                    .join(","),
                );
            }
            for tool in external_tools {
                tools.push(',');
                tools.push_str(&tool);
            }
            args.extend([
                "--permission-mode".into(),
                if editor {
                    "default"
                } else if auto {
                    "auto"
                } else {
                    "acceptEdits"
                }
                .into(),
                "--settings".into(),
                json!({"sandbox":sandbox}).to_string(),
                "--tools".into(),
                "default".into(),
                "--allowedTools".into(),
                tools,
            ]);
            if editor {
                args.extend(["--disallowedTools".into(), DISALLOWED.into()]);
            }
        }
        if let Some(path) = self.extensions_dir.as_ref().filter(|s| !s.is_empty()) {
            args.extend(["--add-dir".into(), path.clone()]);
        }
        let mut system = self.instructions.clone();
        if authority.access == Access::Project {
            system.push_str("\n\nSHELL NETWORK\n");
            system.push_str(&self.shell_network_instructions);
            if asking {
                system.push_str("\n\n");
                system.push_str(if auto { AUTO } else { ASKING });
            }
        }
        system.push_str("\n\nReturn the final answer only through the requested JSON schema.");
        if let Some(extra) = self
            .additional_instructions
            .as_ref()
            .filter(|s| !s.is_empty())
        {
            system.push_str("\n\n");
            system.push_str(extra);
        }
        args.extend(["--system-prompt".into(), system]);
        Ok(args)
    }
    /// Builds the real process worker. The executable and isolated environment
    /// come from runtime discovery; startup/IO/reaping never block the caller.
    pub fn launch(
        &self,
        mut process: transport::ProcessSpec,
        run: String,
        authority: Authority,
    ) -> Result<transport::Handle, String> {
        process.args = self.argv(&authority)?;
        Ok(transport::spawn(
            process,
            transport::Machine::Claude(claude::Client::new(run, authority, 0)),
            vec![claude::user_message(&self.prompt, &self.image_paths)],
        ))
    }
}
