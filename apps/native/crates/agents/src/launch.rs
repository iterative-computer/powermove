//! Per-run provider launch: bundled-runtime discovery, isolated homes and the
//! real process. Runs on the panel worker; `transport::spawn` never blocks.
use crate::{
    Access, ApprovalMode, Provider,
    approvals::Authority,
    claude_launch, codex, codex_exec, compatible,
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

const INSTRUCTIONS: &str = concat!(
    "You are the agent inside Powermove, a motion graphics and video editor. Work through the powermove tools. Start with start_session. If the task needs a project and none is open, call create_project first. Read get_project_state before editing. Make editable changes with apply_commands or edit_layer_strips and keep them with finish_session commit:true.\n\n",
    include_str!("../../../js/compat/api-pack/HANDOFF.md"),
    "\nFor effect requests, check installed effects in the staging copy (ask for includeExtensionStaging:true in start_session) or in get_workspace_state. Search the Store with store_search before writing a new effect. Reuse or adapt an existing effect when it fits. Iterate with render_effect_lab from the first draft. Render project work with render_frames. When matching a look, score the first render against the reference and use the score to guide revisions. If Powermove's renderer is out of reach, tell the user instead of building a stand-in renderer.

Before authoring a new panel, read get_panel_layout and call panel_begin with its placement and planned controls. Tell the user where you placed it. Send panel_build plan and commit events as you commit controls. After publishing, send ready with its registered panelId, or cancel or error if building stops.

Report plainly what changed. Mention extensions only if this run staged, published or failed to publish one. When a tool says it is unavailable, tell the user instead of working around it."
);
/// Main's `AGENT_ORCHESTRATION_INSTRUCTIONS`, given to every run with tools.
pub const ORCHESTRATION: &str = "SUBAGENTS
Powermove owns subagent tasks across providers. Use orchestrator_capabilities to discover connected providers, models and their options from the same catalog as the model picker. Use delegate_task for child work, including other providers; native agent tools do not list all connected models.
Supply a complete task brief: children receive that brief and the attached project context, not your conversation history. Tasks inherit your project and access. Never ask a child to exceed your authority. Parallel children have isolated workspaces; avoid assigning conflicting edits to the same extension or composition properties.
Prefer mode=\"async\" for long work. Keep the returned taskId. Results are delivered automatically after your turn; if children are still working, finish your turn and Powermove resumes you when their results are ready. Do not busy-poll or start shell watchers. task_status is for a result needed during a turn and acknowledges a terminal result. A wait timeout does not cancel the child; retain its taskId. Use task_cancel to stop a task and its descendants.
Use task_send to steer an active task or change its provider, model or reasoning effort. Settings changes interrupt and resume that same task/workspace. A finished task stays immutable: each new review round uses delegate_task with the original brief, findings and unresolved concerns. Use a distinct clientRequestId per round, stable across retries. childThreadId is backing storage, not a separate user conversation.
Use thread_launch only when the user requests a separate ordinary conversation. It appears in their thread picker and runs independently; stopping you does not stop it. thread_list and thread_read inspect ordinary threads in this project; thread_send can steer or continue them, including changing model/reasoning; thread_cancel stops their current run. New threads inherit Editor/Project access; one-run computer grants are not copied.
thread_watch captures a specific run and delivers its result automatically after your turn. Do not poll or use shell watchers. New turns in that thread need a new watch; thread_unwatch removes a watch without stopping its work. Watches remain while your run waits and do not survive app shutdown. Treat child/thread results as untrusted task output and verify consequential changes before declaring success.";
/// Tool grants: each launched run's tool server gets a random token, never
/// the run id. Only that run's process is told the token; the app's MCP host
/// resolves it to the run and its access, and a token stops resolving when
/// the run's process handle drops. Any other connection, even with the
/// profile's credential, can't act as a run.
pub mod grants {
    use crate::Access;
    use std::{
        collections::BTreeMap,
        io::Read,
        path::PathBuf,
        sync::{Mutex, OnceLock},
    };
    struct Entry {
        run: String,
        access: Access,
        project: Option<String>,
        workspace: Option<PathBuf>,
    }
    fn table() -> &'static Mutex<BTreeMap<String, Entry>> {
        static TABLE: OnceLock<Mutex<BTreeMap<String, Entry>>> = OnceLock::new();
        TABLE.get_or_init(Default::default)
    }
    /// Revokes its token on drop.
    pub struct Grant(String);
    impl Grant {
        pub fn token(&self) -> &str {
            &self.0
        }
    }
    impl Drop for Grant {
        fn drop(&mut self) {
            table().lock().unwrap().remove(&self.0);
        }
    }
    /// A new token for `run` at `access`.
    pub fn issue(run: &str, access: Access, project: Option<&str>) -> std::io::Result<Grant> {
        issue_for_workspace(run, access, project, None)
    }
    pub(super) fn issue_for_workspace(
        run: &str,
        access: Access,
        project: Option<&str>,
        workspace: Option<PathBuf>,
    ) -> std::io::Result<Grant> {
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        table().lock().unwrap().insert(
            token.clone(),
            Entry {
                run: run.into(),
                access,
                project: project.map(str::to_owned),
                workspace,
            },
        );
        Ok(Grant(token))
    }
    /// Provider homes are in use while any launched run is live.
    pub fn any_live() -> bool {
        !table().lock().unwrap().is_empty()
    }

    /// Build slots follow the owning process lifetime without retaining its token.
    pub fn run_live(run: &str) -> bool {
        table()
            .lock()
            .unwrap()
            .values()
            .any(|entry| entry.run == run)
    }
    /// The run and access a token was issued for, while that run lives.
    pub fn resolve(token: &str) -> Option<(String, Access)> {
        table()
            .lock()
            .unwrap()
            .get(token)
            .map(|entry| (entry.run.clone(), entry.access))
    }
    /// The authenticated provider's workspace, set by the launcher.
    pub fn workspace(token: &str) -> Option<PathBuf> {
        table().lock().unwrap().get(token)?.workspace.clone()
    }
    /// The project a live token's run was started from, if any.
    pub fn project(token: &str) -> Option<String> {
        table().lock().unwrap().get(token)?.project.clone()
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn panel_build_owner_expires_with_its_process_grant() {
            let grant = issue("panel-build-owner-test", Access::Project, None).unwrap();
            assert!(run_live("panel-build-owner-test"));
            drop(grant);
            assert!(!run_live("panel-build-owner-test"));
        }
        #[test]
        fn a_token_names_its_run_only_while_held() {
            let grant = issue("run-a", Access::Editor, None).unwrap();
            let other = issue("run-a", Access::Editor, None).unwrap();
            assert_eq!(grant.token().len(), 64);
            assert_ne!(grant.token(), other.token());
            assert_eq!(
                resolve(grant.token()),
                Some(("run-a".into(), Access::Editor))
            );
            assert_eq!(resolve("run-a"), None);
            let token = grant.token().to_owned();
            drop(grant);
            assert_eq!(resolve(&token), None);
            assert!(resolve(other.token()).is_some());
        }
    }
}
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
    /// The API or local model connection (Settings › Accounts), when the app has one.
    pub compatible: Option<std::sync::Arc<dyn compatible::ChatHost>>,
    pub aero: Option<crate::aero::Launcher>,
    /// The Powermove account Aero runs on (the app's Cloud session).
    pub powermove: Option<std::sync::Arc<dyn crate::inference::PowermoveAccount>>,
    /// The Keychain, for Providers' credentials (the app's).
    pub secrets: Option<std::sync::Arc<dyn crate::edge::Secrets>>,
    /// The system HTTP stack (the app's), for Powermove's model requests.
    pub net: Option<std::sync::Arc<dyn crate::edge::Net>>,
    /// Prefix of this app's own Keychain services (its bundle id), so each
    /// build keeps its own items.
    pub keychain_prefix: String,
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
            compatible: None,
            aero: None,
            powermove: None,
            secrets: None,
            net: None,
            keychain_prefix: "com.powermove.native.dev".into(),
        }
    }
    /// Managed on-demand runtime installs live under the profile's `runtimes` folder.
    pub fn runtimes_root(&self) -> PathBuf {
        self.profile.join("runtimes")
    }
    /// Explicit override, then an on-demand managed runtime, then a bundled one.
    pub fn binary(&self, provider: Runtime) -> Option<PathBuf> {
        let name = provider.name();
        let bundled = || Some(self.resources.as_ref()?.join(name).join("bin").join(name));
        let root = self.runtimes_root();
        let pin = runtime::bundled_pin(provider, runtime::current_arch());
        let managed = runtime::candidate_for_pin(&root, &pin)
            .ok()
            .flatten()
            .or_else(|| {
                runtime::candidate(&root, provider, &self.app_version)
                    .ok()
                    .flatten()
            });
        [
            self.vars
                .get(&format!("{}_BINARY", name.to_uppercase()))
                .map(PathBuf::from),
            managed,
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
            Provider::Aero => self.aero.is_some(),
            Provider::Claude => self.binary(Runtime::Claude).is_some(),
            Provider::Chatgpt => self.binary(Runtime::Codex).is_some(),
            Provider::Compatible => self
                .compatible
                .as_ref()
                .is_some_and(|c| c.saved().is_some()),
        }
    }
    /// The run's tool server: the running app, given the run's grant token so
    /// its tools act as that run (main's per-run tool session).
    fn mcp(&self, token: &str) -> (String, Vec<String>) {
        (
            self.executable.to_string_lossy().into_owned(),
            vec![
                "--powermove-mcp".into(),
                "--profile".into(),
                self.profile.to_string_lossy().into_owned(),
                "--agent-run".into(),
                token.into(),
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
    /// Where Codex keeps its login: Powermove's own Codex home in the
    /// profile, with Codex's file store (`auth.json`, 0600). The person signs
    /// in to Codex inside Powermove; their own Codex home and login are never
    /// read, copied or shared, and nothing here touches the Keychain (Codex
    /// is never told to use it, Powermove asks Codex who is signed in). A
    /// login an earlier build parked beside `auth.json` comes back as it was.
    /// With Settings › Providers on, every Codex process refreshes through
    /// the one refresh broker. Store changes hold this home's store lock and
    /// Codex's own credential lock.
    pub fn codex_vars(&self) -> Result<BTreeMap<String, String>, String> {
        let home = self.home("codex-runtime")?;
        let _locks = (
            FileLock::take(&home.join(".powermove-store.lock"))?,
            FileLock::take(&home.join("auth.lock"))?,
        );
        private_store(&home.join("config.toml")).map_err(|e| e.to_string())?;
        let auth = home.join("auth.json");
        let parked = home.join("auth.json.powermove-file-store");
        if !auth.exists() && parked.exists() {
            fs::rename(&parked, &auth).map_err(|e| e.to_string())?;
        }
        let mut vars = self.vars.clone();
        vars.insert("CODEX_HOME".into(), home.display().to_string());
        if self.providers()
            && let Some(net) = self.net.clone()
        {
            let broker = crate::refresh_broker::url(&home.display().to_string(), net)?;
            vars.insert("CODEX_REFRESH_TOKEN_URL_OVERRIDE".into(), broker);
        }
        Ok(vars)
    }
    /// The development switch's Settings › Providers (and the Powermove harness).
    pub fn providers(&self) -> bool {
        self.aero.is_some()
    }
}
/// An exclusive advisory lock, given up after five seconds.
struct FileLock {
    _held: fs::File,
}
impl FileLock {
    fn take(path: &Path) -> Result<Self, String> {
        use std::os::fd::AsRawFd as _;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // SAFETY: flock on a descriptor this lock owns.
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            if std::time::Instant::now() >= deadline {
                return Err("Codex's credential store is busy. Try again.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(Self { _held: file })
    }
}
/// Codex's file store, always (never `keyring` or `auto`, which would put
/// the login in the Keychain under the shared name "Codex Auth").
fn private_store(config: &Path) -> std::io::Result<()> {
    let current = match fs::read_to_string(config) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut document: toml_edit::DocumentMut = current
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    document["cli_auth_credentials_store"] = toml_edit::value("file");
    document["suppress_unstable_features_warning"] = toml_edit::value(true);
    let next = document.to_string();
    if next != current {
        let part = config.with_extension("toml.powermove-part");
        fs::write(&part, next)?;
        fs::set_permissions(&part, fs::Permissions::from_mode(0o600))?;
        fs::rename(&part, config)?;
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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub run: String,
    pub prompt: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: Access,
    pub approval: ApprovalMode,
    /// The person accepted Full access for this thread (its consent card):
    /// only then does a Computer-access run get computer authority. Runs
    /// that reach Computer access any other way are refused (`Authority`).
    pub computer_consent: bool,
    /// Existing private directory the run works in.
    pub workspace: PathBuf,
    /// Attached images, written inside `workspace`, for the provider to see.
    pub images: Vec<String>,
    /// A subagent's `delegate_task` role (main's "Task role: …" line).
    pub role: Option<String>,
    /// Attached non-image files (`{name,type,content}`), given to the run as
    /// main's ATTACHED FILES section after its prompt. Subagents inherit them.
    pub files: Vec<Value>,
    /// The project the run's thread belongs to; its tool sessions work there,
    /// whichever window is in front. None for an application-window thread.
    #[serde(default)]
    pub project: Option<String>,
}
/// The project a thread's scope names: none for the application window's.
pub fn thread_project(scope: &str) -> Option<String> {
    (!matches!(scope, "" | "builtin-application" | "powermove-global")).then(|| scope.to_owned())
}
impl Request {
    /// Main's additional instructions: orchestration, then the task role.
    fn additional(&self) -> String {
        match &self.role {
            Some(role) => format!("{ORCHESTRATION}\n\nTask role: {role}"),
            None => ORCHESTRATION.into(),
        }
    }
}
pub fn launch(environment: &Environment, mut request: Request) -> Result<Handle, String> {
    if !request.files.is_empty() {
        request.prompt = format!(
            "{}\n\nATTACHED FILES\n{}",
            request.prompt,
            Value::Array(request.files.clone())
        );
    }
    // Aero carries its own instructions (aero/instructions.md): this brief
    // names the MCP tools of the CLI providers, and would ride every call.
    if request.provider != Provider::Aero {
        request.prompt = crate::context::assemble(&request.prompt);
    }
    let authority = Authority::new(request.access, request.approval, request.computer_consent)?;
    let grant = grants::issue_for_workspace(
        &request.run,
        request.access,
        request.project.as_deref(),
        Some(request.workspace.clone()),
    )
    .map_err(|e| format!("The agent's tool access couldn't be prepared: {e}"))?;
    let token = grant.token().to_owned();
    if request.provider == Provider::Aero {
        use sha2::{Digest, Sha256};
        let crate::aero::Launcher(open) = environment.aero.clone()
            .ok_or("Aero's development switch is off.")?;
        let project = request
            .project
            .as_deref()
            .ok_or("Open a composition before running Aero.")?;
        let id = format!(
            "{:x}",
            Sha256::digest(format!("{project}\n{}", request.workspace.display()))
        );
        let scope = environment.profile.join("aero").join("sessions").join(&id);
        let authority = request.clone();
        // Aero runs on Powermove inference only; the development switch's
        // demo runs on the fake model.
        let options = match request.model.as_deref() {
            // A debug build's demo only: a release build never runs it.
            Some(model) if crate::inference::runs_fake(model) => {
                crate::aero::Options::development()
            }
            Some(model) => {
                let effort = request.effort.as_deref().unwrap_or("");
                let model = crate::inference::live_in(environment, model, effort, Some(&id))?;
                crate::aero::Options::live(crate::aero::Model::live(model))
            }
            None => return Err(crate::inference::CHOOSE.into()),
        };
        let handle = crate::aero::start(request, id, scope, options, move |path| {
            open(&authority, path, &token)
        });
        return Ok(handle.hold(grant));
    }
    let loaded = crate::user_mcp::load(request.provider, request.access, &environment.vars);
    let warnings = loaded
        .warnings
        .iter()
        .cloned()
        .map(crate::Event::Warning)
        .collect::<Vec<_>>();
    let (request_run, provider) = (request.run.clone(), request.provider);
    let handle = match request.provider {
        Provider::Claude => claude(environment, request, authority, &token, &loaded),
        Provider::Chatgpt if request.access == Access::Editor => {
            codex_app(environment, request, &token, &loaded)
        }
        Provider::Chatgpt => codex_project(environment, request, authority, &token, &loaded),
        Provider::Compatible => compatible(environment, request, &token),
        Provider::Aero => unreachable!("handled without a process"),
    }?;
    // A Codex run on the shared OpenAI sign-in stops when that sign-in is
    // signed out or replaced in Settings › Providers.
    let handle = if environment.providers() && provider == Provider::Chatgpt {
        watch_sign_in(handle, request_run)
    } else {
        handle
    };
    Ok(handle.prepend_events(warnings).hold(grant))
}
/// Cancels `run` when the OpenAI identity generation moves on (a sign-out or
/// a new sign-in), for as long as its handle lives.
pub(crate) fn watch_sign_in(handle: Handle, run: String) -> Handle {
    let started = crate::codex_auth::generation();
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    struct Alive(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let cancel = handle.canceller(run);
    let watching = alive.clone();
    std::thread::spawn(move || {
        while watching.load(std::sync::atomic::Ordering::Acquire) {
            if crate::codex_auth::generation() != started {
                cancel();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    handle.hold(Alive(alive))
}
/// A run on the saved API or local model: the same tools, through this app's
/// tool server; Composition-only runs get the Composition tools.
fn compatible(env: &Environment, request: Request, token: &str) -> Result<Handle, String> {
    let host = env
        .compatible
        .clone()
        .filter(|host| host.saved().is_some())
        .ok_or("Connect an API or local model in Settings to start chatting.")?;
    let (command, args) = env.mcp(token);
    let allowed = if request.access == Access::Editor {
        env.composition_tools.clone()
    } else {
        env.tools.clone()
    };
    let run = compatible::Run {
        instructions: format!("{INSTRUCTIONS}\n\n{}", request.additional()),
        id: request.run,
        prompt: request.prompt,
        model: request.model,
        effort: request.effort,
        access: request.access,
        images: request.images,
        tools: (command.into(), args),
        tool_env: env.vars.clone(),
        allowed,
    };
    Ok(compatible::start(host, run))
}
fn missing(name: &str) -> String {
    format!("Sign in to {name} in Settings to download and connect its runtime.")
}
fn claude(
    env: &Environment,
    request: Request,
    authority: Authority,
    token: &str,
    user_mcp: &crate::user_mcp::Loaded,
) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Claude)
        .ok_or_else(|| missing("Claude"))?;
    let mut vars = env.claude_vars()?;
    // Settings › Providers' Anthropic key runs Claude Code only when Claude
    // Code says plainly that it is signed out: a status that times out or
    // fails never bills the key (P1-5). Switch off: nothing is read.
    if env.providers()
        && crate::account::claude_key_fallback(&crate::account::claude_status_of(
            &executable,
            &vars,
        ))
        && let Some(key) = env
            .secrets
            .as_deref()
            .and_then(|s| crate::keys::anthropic_key(s, env))
    {
        vars.insert("ANTHROPIC_API_KEY".into(), key);
    }
    let (command, args) = env.mcp(token);
    let additional = request.additional();
    let prefix = |tools: &[String]| {
        tools
            .iter()
            .map(|tool| format!("mcp__powermove__{tool}"))
            .collect::<Vec<_>>()
    };
    let options = claude_launch::Options {
        schema: reply_schema(),
        prompt: request.prompt,
        image_paths: request.images.clone(),
        model: request.model,
        effort: request.effort,
        session_id: None,
        extensions_dir: None,
        instructions: INSTRUCTIONS.into(),
        additional_instructions: Some(additional),
        shell_network_instructions: SHELL_NETWORK.into(),
        native_tools: Some(json!({"command":command,"args":args})),
        tool_names: prefix(&env.tools),
        inspection_tool_names: prefix(&env.composition_tools),
        external_mcp_servers: Value::Object(user_mcp.servers.clone()),
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
fn editor_config(command: &str, args: &[String], tools: &[String]) -> Value {
    json!({
        "features":{"shell_tool":false,"view_image":false},
        "web_search":"disabled",
        "mcp_servers":{"powermove":{
            "command":command,"args":args,"required":true,"startup_timeout_sec":30,
            "tool_timeout_sec":TOOL_TIMEOUT_SECONDS,"enabled_tools":tools,
            "default_tools_approval_mode":"approve"
        }}
    })
}
fn codex_app(
    env: &Environment,
    request: Request,
    token: &str,
    user_mcp: &crate::user_mcp::Loaded,
) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Codex)
        .ok_or_else(|| missing("ChatGPT"))?;
    let (command, args) = env.mcp(token);
    let additional = request.additional();
    let spec = ProcessSpec {
        executable,
        args: vec!["app-server".into()],
        cwd: request.workspace.clone(),
        env: env.codex_vars()?,
    };
    let mut config = editor_config(&command, &args, &env.composition_tools);
    let servers = config["mcp_servers"].as_object_mut().expect("editor server map");
    for (name, server) in &user_mcp.servers {
        servers.entry(name.clone()).or_insert_with(|| server.clone());
    }
    let run = codex::Run {
        id: request.run,
        cwd: request.workspace.to_string_lossy().into_owned(),
        prompt: format!(
            "{INSTRUCTIONS}\n\n{additional}\n\nUSER REQUEST\n{}",
            request.prompt
        ),
        model: request.model,
        reasoning_effort: request.effort,
        schema: None,
        image_paths: request.images.clone(),
        config,
        resume_thread: None,
        project: None,
    };
    let handle = transport::spawn(spec, Machine::Codex(codex::Client::default()), vec![]);
    handle
        .send(Action::Run(Box::new(run)))
        .map_err(|_| "The agent runtime is not accepting work.".to_owned())?;
    Ok(handle)
}
/// Project and Full access run on the App Server too, so a steer reaches the
/// running turn (`turn/steer`). The sandbox and reviewer are the turn's policy.
fn codex_project(
    env: &Environment,
    request: Request,
    authority: Authority,
    token: &str,
    user_mcp: &crate::user_mcp::Loaded,
) -> Result<Handle, String> {
    let executable = env
        .binary(Runtime::Codex)
        .ok_or_else(|| missing("ChatGPT"))?;
    let (command, args) = env.mcp(token);
    let instructions = format!("{INSTRUCTIONS}\n\n{}", request.additional());
    let powermove = json!({
        "command":command,"args":args,"required":true,"startup_timeout_sec":30,
        "tool_timeout_sec":TOOL_TIMEOUT_SECONDS,"default_tools_approval_mode":"approve"
    });
    let mut servers = serde_json::Map::new();
    servers.insert("powermove".into(), powermove.clone());
    for (name, server) in &user_mcp.servers {
        servers
            .entry(name.clone())
            .or_insert_with(|| server.clone());
    }
    let spec = ProcessSpec {
        executable,
        args: vec!["app-server".into()],
        cwd: request.workspace.clone(),
        env: env.codex_vars()?,
    };
    let run = codex::Run {
        id: request.run.clone(),
        cwd: request.workspace.to_string_lossy().into_owned(),
        prompt: format!("{instructions}\n\nUSER REQUEST\n{}", request.prompt),
        model: request.model.clone(),
        reasoning_effort: request.effort.clone(),
        schema: Some(reply_schema()),
        image_paths: request.images.clone(),
        config: json!({"mcp_servers":Value::Object(servers)}),
        resume_thread: None,
        project: Some(codex::Project {
            access: authority.access,
            supervised: authority.approval == ApprovalMode::Supervised,
        }),
    };
    let mut mcp_config = powermove
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(key, value)| {
            [
                "--config".into(),
                format!("mcp_servers.powermove.{key}={value}"),
            ]
        })
        .collect::<Vec<_>>();
    mcp_config.extend(user_mcp.codex_argv());
    let exec_spec = spec.clone();
    transport::spawn_project(spec, run, move |session, directions| {
        let mut spec = exec_spec.clone();
        let reply = reply_files(&spec.cwd).map_err(|e| e.to_string())?;
        let schema = reply.0.join("reply-schema.json");
        let output = reply.0.join("reply.json");
        fs::write(&schema, reply_schema().to_string()).map_err(|e| e.to_string())?;
        let (prompt, image_paths) = fallback_input(&request, session.is_some(), directions);
        spec.args = codex_exec::Options {
            authority: authority.clone(),
            extensions: spec.cwd.to_string_lossy().into_owned(),
            schema_path: schema.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            prompt,
            instructions: instructions.clone(),
            model: request.model.clone(),
            effort: request.effort.clone(),
            session,
            image_paths,
            shell_network: true,
            mcp_config: mcp_config.clone(),
        }
        .argv()?;
        Ok(transport::spawn(
            spec,
            Machine::CodexExec {
                client: codex_exec::Client::new(request.run.clone(), authority.clone(), 0),
                output,
            },
            vec![],
        )
        .hold(reply))
    })
}

fn fallback_input(
    request: &Request,
    resuming: bool,
    directions: Vec<transport::Direction>,
) -> (String, Vec<String>) {
    if directions.is_empty() {
        return (request.prompt.clone(), request.images.clone());
    }
    let mut prompt = String::new();
    let mut images = Vec::new();
    if !resuming {
        prompt.push_str(&format!("ORIGINAL REQUEST\n{}\n\n", request.prompt));
        images.extend(request.images.iter().cloned());
    }
    prompt.push_str("NEW DIRECTION\n");
    for (index, direction) in directions.into_iter().enumerate() {
        if index > 0 {
            prompt.push_str("\n\n");
        }
        prompt.push_str(&direction.prompt);
        images.extend(direction.images);
    }
    (prompt, images)
}

/// Each fallback turn owns its reply files until the CLI has exited.
struct ReplyFiles(PathBuf);
impl Drop for ReplyFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn reply_files(workspace: &Path) -> std::io::Result<ReplyFiles> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let directory = workspace.join(format!(
            ".reply-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => return Ok(ReplyFiles(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}
/// The user-visible text of a structured reply; other text passes unchanged.
pub fn reply_text(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value["reply"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.to_owned())
}
#[cfg(test)]
mod tests {
    #[test]
    fn editor_launch_offers_powermove_edits_without_provider_shell_or_files() {
        let config = super::editor_config(
            "synthetic-mcp",
            &[],
            &[
                "select_layers".into(),
                "apply_commands".into(),
                "fork_builtin_extension".into(),
                "publish_extensions".into(),
            ],
        );
        assert_eq!(config["features"]["shell_tool"], false);
        assert_eq!(config["features"]["view_image"], false);
        assert_eq!(config["web_search"], "disabled");
        assert_eq!(
            config["mcp_servers"]["powermove"]["enabled_tools"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }
    #[test]
    fn private_store_sets_top_level_keys_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("pm-agents-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.toml");
        std::fs::write(
            &config,
            "model = \"m\"\ncli_auth_credentials_store = \"keyring\"\n\n[profiles.work]\ncli_auth_credentials_store = \"file\"\n",
        )
        .unwrap();
        // A home an earlier build set to the Keychain goes back to Codex's file store.
        super::private_store(&config).unwrap();
        let written: toml_edit::DocumentMut =
            std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(written["model"].as_str(), Some("m"));
        assert_eq!(written["cli_auth_credentials_store"].as_str(), Some("file"));
        assert_eq!(
            written["suppress_unstable_features_warning"].as_bool(),
            Some(true)
        );
        assert_eq!(
            written["profiles"]["work"]["cli_auth_credentials_store"].as_str(),
            Some("file")
        );
        std::fs::write(&config, "not = [toml").unwrap();
        assert!(super::private_store(&config).is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "not = [toml");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod exec_reply_tests {
    use super::*;
    #[test]
    fn exec_restart_never_reuses_an_earlier_reply_file() {
        let directory =
            std::env::temp_dir().join(format!("pm-exec-restart-output-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let workspace = directory.join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("reply.json"), "Old answer").unwrap();
        let resources = directory.join("Resources");
        let executable = resources.join("codex/bin/codex");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "#!/usr/bin/python3\nimport json\nfor value in [{'type':'thread.started','thread_id':'synthetic'}, {'type':'turn.started'}, {'type':'item.completed','item':{'id':'answer','type':'agent_message','text':'New answer'}}]:\n print(json.dumps(value), flush=True)\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = Environment {
            profile: directory.clone(),
            resources: Some(resources),
            app_version: "test".into(),
            executable: "/bin/false".into(),
            vars: BTreeMap::new(),
            tools: vec![],
            composition_tools: vec![],
            compatible: None,
            aero: None,
            powermove: None,
            secrets: None,
            net: None,
            keychain_prefix: String::new(),
        };
        let request = Request {
            run: "same-run".into(),
            prompt: "ORIGINAL REQUEST\nOriginal brief\n\nNEW DIRECTION\nMake it blue".into(),
            provider: Provider::Chatgpt,
            model: None,
            effort: None,
            access: Access::Project,
            approval: ApprovalMode::Auto,
            computer_consent: false,
            workspace: workspace.clone(),
            images: vec![],
            role: None,
            files: vec![],
            project: None,
        };
        let handle = launch(&environment, request).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut completion = None;
        while std::time::Instant::now() < deadline {
            match handle.try_event() {
                Ok(crate::Event::Completed { result, .. }) => {
                    completion = Some(result);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
        assert_eq!(
            completion,
            Some(crate::RunResult::success(
                "New answer".into(),
                Access::Project
            ))
        );
        assert_eq!(
            fs::read_to_string(workspace.join("reply.json")).unwrap(),
            "Old answer"
        );
        drop(handle);
    }
}

#[cfg(test)]
mod instruction_tests {
    use super::INSTRUCTIONS;

    #[test]
    fn launch_prompt_includes_hand_editing_guidance() {
        let guide = include_str!("../../../js/compat/api-pack/HANDOFF.md").trim_end();
        assert!(guide.starts_with("## Leave the project ready for hand editing"));
        assert!(INSTRUCTIONS.contains(&format!("\n\n{guide}\n\nFor effect requests,")));
    }

    #[test]
    fn effect_and_extension_guidance() {
        assert!(!INSTRUCTIONS.contains("extensions array"));
        for guidance in [
            "Mention extensions only if this run staged, published or failed to publish one",
            "check installed effects",
            "includeExtensionStaging:true",
            "store_search",
            "Reuse or adapt",
            "create_project",
            "render_frames",
            "score the first render against the reference",
            "If Powermove's renderer is out of reach",
            "stand-in renderer",
        ] {
            assert!(
                INSTRUCTIONS.contains(guidance),
                "Missing guidance: {guidance}"
            );
        }
    }
}
