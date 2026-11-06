//! Per-run provider launch: bundled-runtime discovery, isolated homes and the
//! real process. Runs on the panel worker; `transport::spawn` never blocks.
use crate::{
    Access, ApprovalMode, Provider,
    approvals::Authority,
    claude_launch, codex, codex_exec,
    runtime::{self, Runtime},
    transport::{self, Action, Handle, Machine, ProcessSpec},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

const INSTRUCTIONS: &str = "You are the agent inside Powermove, a motion graphics and video editor. Work through the powermove tools: call start_session first, read get_project_state before editing, make editable changes with apply_commands or edit_layer_strips, and end with finish_session commit:true to keep them. Report plainly what changed. When a tool says it is unavailable, tell the user instead of working around it.";
/// A render or export is one tool call.
const TOOL_TIMEOUT_SECONDS: u32 = 3600;
const SHELL_NETWORK: &str = "Shell commands can reach the network.";
/// Claude Code needs an object schema, so its reply arrives as one field.
fn reply_schema() -> Value {
    json!({"type":"object","properties":{"reply":{"type":"string"}},"required":["reply"],"additionalProperties":false})
}

/// What a run may use from this installation.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub profile: PathBuf,
    /// `Contents/Resources` of the app bundle, when running from one.
    pub resources: Option<PathBuf>,
    pub app_version: String,
    /// The running app, which bridges a provider's MCP stdio to its tools.
    pub executable: PathBuf,
    pub vars: BTreeMap<String, String>,
    pub tools: Vec<String>,
    /// The Composition-only subset of `tools`.
    pub composition_tools: Vec<String>,
}
impl Environment {
    /// Reads this process. Safe mode would drop the run's own MCP server.
    pub fn current(profile: PathBuf, tools: Vec<String>, composition_tools: Vec<String>) -> Self {
        let executable = std::env::current_exe().unwrap_or_default();
        let resources = executable
            .parent()
            .and_then(Path::parent)
            .map(|contents| contents.join("Resources"))
            .filter(|path| path.is_dir());
        let mut vars = std::env::vars().collect::<BTreeMap<_, _>>();
        vars.remove("CLAUDE_CODE_SAFE_MODE");
        Self {
            profile,
            resources,
            app_version: env!("CARGO_PKG_VERSION").into(),
            executable,
            vars,
            tools,
            composition_tools,
        }
    }
    /// Explicit override, then an updated managed runtime, then the bundled one.
    pub fn binary(&self, provider: Runtime) -> Option<PathBuf> {
        let name = provider.name();
        let bundled = || Some(self.resources.as_ref()?.join(name).join("bin").join(name));
        let managed =
            runtime::candidate(&self.profile.join("runtimes"), provider, &self.app_version);
        [
            self.vars
                .get(&format!("{}_BINARY", name.to_uppercase()))
                .map(PathBuf::from),
            managed.ok().flatten(),
            bundled(),
        ]
        .into_iter()
        .flatten()
        .find(|path| executable(path))
    }
    /// Whether a run can start. Being signed in is the runtime's own answer,
    /// which a run reports in its conversation.
    pub fn available(&self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.binary(Runtime::Claude).is_some(),
            Provider::Chatgpt => self.binary(Runtime::Codex).is_some(),
            Provider::Compatible => false,
        }
    }
    fn mcp(&self) -> (String, Vec<String>) {
        (
            self.executable.to_string_lossy().into_owned(),
            vec![
                "--powermove-mcp".into(),
                "--profile".into(),
                self.profile.to_string_lossy().into_owned(),
            ],
        )
    }
    fn home(&self, name: &str) -> Result<PathBuf, String> {
        let home = self.profile.join(name);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&home)
            .map_err(|e| e.to_string())?;
        Ok(home)
    }
    /// Claude Code owns every credential in here. Logout can't reach a terminal login.
    pub fn claude_vars(&self) -> Result<BTreeMap<String, String>, String> {
        let mut vars = self.vars.clone();
        let home = self.home("claude-runtime")?;
        vars.insert("CLAUDE_CONFIG_DIR".into(), home.display().to_string());
        Ok(vars)
    }
    /// File-only storage keeps account calls on Powermove's own `auth.json`. An
    /// existing Codex login is copied once; after that its absence means sign-out.
    pub fn codex_vars(&self) -> Result<BTreeMap<String, String>, String> {
        let home = self.home("codex-runtime")?;
        private_store(&home.join("config.toml")).map_err(|e| e.to_string())?;
        let owner = home.join(".powermove-auth-owned");
        let auth = home.join("auth.json");
        let source = self
            .vars
            .get("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| self.vars.get("HOME").map(|h| Path::new(h).join(".codex")))
            .map(|root| root.join("auth.json"));
        if !owner.exists() {
            if !auth.exists()
                && let Some(source) = source.filter(|path| path.is_file())
            {
                fs::copy(source, &auth).map_err(|e| e.to_string())?;
                fs::set_permissions(&auth, fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            fs::write(&owner, "powermove\n").map_err(|e| e.to_string())?;
        }
        let mut vars = self.vars.clone();
        vars.insert("CODEX_HOME".into(), home.display().to_string());
        Ok(vars)
    }
}
fn private_store(config: &Path) -> std::io::Result<()> {
    let current = fs::read_to_string(config).unwrap_or_default();
    let mut next = current.clone();
    for (key, line) in [
        (
            "cli_auth_credentials_store",
            "cli_auth_credentials_store = \"file\"",
        ),
        (
            "suppress_unstable_features_warning",
            "suppress_unstable_features_warning = true",
        ),
    ] {
        let set = next.lines().any(|l| l.trim_start().starts_with(key));
        next = if set {
            next.lines()
                .map(|l| {
                    if l.trim_start().starts_with(key) {
                        line
                    } else {
                        l
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            format!("{line}\n{next}")
        };
    }
    if next != current {
        fs::write(config, next)?;
    }
    fs::set_permissions(config, fs::Permissions::from_mode(0o600))
}
fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
/// The shape both providers' model ids share.
pub fn is_valid_model_id(id: &str) -> bool {
    id.len() <= 128
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/[]-".contains(c))
}
pub struct Request {
    pub run: String,
    pub prompt: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: Access,
    pub approval: ApprovalMode,
    /// Existing private directory the run works in.
    pub workspace: PathBuf,
}
pub fn launch(environment: &Environment, request: Request) -> Result<Handle, String> {
    let authority = Authority::new(request.access, request.approval, false)?;
    match request.provider {
        Provider::Claude => claude(environment, request, authority),
        Provider::Chatgpt if request.access == Access::Editor => codex_app(environment, request),
        Provider::Chatgpt => codex_project(environment, request, authority),
        Provider::Compatible => Err(
            "Compatible providers can't run in this build yet. Choose ChatGPT or Claude.".into(),
        ),
    }
}
fn missing(name: &str) -> String {
    format!("Powermove's built-in {name} runtime is missing. Reinstall Powermove.")
}
fn claude(env: &Environment, request: Request, authority: Authority) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Claude)
        .ok_or_else(|| missing("Claude"))?;
    let vars = env.claude_vars()?;
    let (command, args) = env.mcp();
    let prefix = |tools: &[String]| {
        tools
            .iter()
            .map(|tool| format!("mcp__powermove__{tool}"))
            .collect::<Vec<_>>()
    };
    let options = claude_launch::Options {
        schema: reply_schema(),
        prompt: request.prompt,
        image_paths: vec![],
        model: request.model,
        effort: request.effort,
        session_id: None,
        extensions_dir: None,
        instructions: INSTRUCTIONS.into(),
        additional_instructions: None,
        shell_network_instructions: SHELL_NETWORK.into(),
        native_tools: Some(json!({"command":command,"args":args})),
        tool_names: prefix(&env.tools),
        inspection_tool_names: prefix(&env.composition_tools),
        external_mcp_servers: json!({}),
        ask_outside_sandbox: true,
    };
    let spec = ProcessSpec {
        executable,
        args: vec![],
        cwd: request.workspace,
        env: vars,
    };
    options.launch(spec, request.run, authority)
}
fn codex_app(env: &Environment, request: Request) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Codex)
        .ok_or_else(|| missing("ChatGPT"))?;
    let (command, args) = env.mcp();
    let spec = ProcessSpec {
        executable,
        args: vec!["app-server".into()],
        cwd: request.workspace.clone(),
        env: env.codex_vars()?,
    };
    let run = codex::Run {
        id: request.run,
        cwd: request.workspace.to_string_lossy().into_owned(),
        prompt: format!("{INSTRUCTIONS}\n\nUSER REQUEST\n{}", request.prompt),
        model: request.model,
        reasoning_effort: request.effort,
        schema: None,
        image_paths: vec![],
        config: json!({"mcp_servers":{"powermove":{
            "command":command,"args":args,"required":true,"startup_timeout_sec":30,
            "tool_timeout_sec":TOOL_TIMEOUT_SECONDS,"enabled_tools":env.composition_tools,
            "default_tools_approval_mode":"approve"
        }}}),
        resume_thread: None,
    };
    let handle = transport::spawn(spec, Machine::Codex(codex::Client::default()), vec![]);
    handle
        .send(Action::Run(Box::new(run)))
        .map_err(|_| "The agent runtime is not accepting work.".to_owned())?;
    Ok(handle)
}
/// Project access runs the autonomous CLI, which owns its sandbox and reviewer.
fn codex_project(
    env: &Environment,
    request: Request,
    authority: Authority,
) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Codex)
        .ok_or_else(|| missing("ChatGPT"))?;
    let schema = request.workspace.join("reply-schema.json");
    let output = request.workspace.join("reply.json");
    fs::write(&schema, reply_schema().to_string()).map_err(|e| e.to_string())?;
    let (command, args) = env.mcp();
    let mcp_config = [
        format!("mcp_servers.powermove.command={}", json!(command)),
        format!("mcp_servers.powermove.args={}", json!(args)),
        "mcp_servers.powermove.required=true".into(),
        "mcp_servers.powermove.default_tools_approval_mode=\"approve\"".into(),
        "mcp_servers.powermove.startup_timeout_sec=30".into(),
        format!("mcp_servers.powermove.tool_timeout_sec={TOOL_TIMEOUT_SECONDS}"),
    ]
    .into_iter()
    .flat_map(|pair| ["--config".to_owned(), pair])
    .collect();
    let options = codex_exec::Options {
        authority: authority.clone(),
        extensions: request.workspace.to_string_lossy().into_owned(),
        schema_path: schema.to_string_lossy().into_owned(),
        output_path: output.to_string_lossy().into_owned(),
        prompt: request.prompt,
        instructions: INSTRUCTIONS.into(),
        model: request.model,
        effort: request.effort,
        session: None,
        image_paths: vec![],
        shell_network: true,
        mcp_config,
    };
    let spec = ProcessSpec {
        executable,
        args: options.argv()?,
        cwd: request.workspace,
        env: env.codex_vars()?,
    };
    Ok(transport::spawn(
        spec,
        Machine::CodexExec {
            client: codex_exec::Client::new(request.run, authority, 0),
            output,
        },
        vec![],
    ))
}
/// The user-visible text of a structured reply; other text passes unchanged.
pub fn reply_text(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value["reply"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.to_owned())
}
