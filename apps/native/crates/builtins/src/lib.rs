//! Application v4 embedding. JavaScript owns presentation; the kernel remains
//! the only document writer. GPUI consumes immutable accepted surfaces.
#![cfg_attr(test, recursion_limit = "512")]
mod agent_host;
pub use agent_host::NoticeSink;
mod agent_tasks;
mod chrome;
pub use agent_host::AgentCarry;
pub use models::APPLICATION_AGENT_STORE;
pub use settings::{RESOLUTIONS, set_notification_status};
#[cfg(test)]
mod agent_tests;
mod data;
pub mod delivery;
pub mod media_import;
mod models;
mod properties;
mod settings;
mod timeline_easing;
pub mod virtualization;
use data::Backend;
use pm_kernel::KernelHandle;
use pm_runtime_v8::*;
use pm_surface::{
    Surface, events::EventDetail, host_api::*, host_data::HostDataService, interaction::Interaction,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// A built-in panel, from `js/v4/app/panels.json`: the one list the runtime
/// mounts, forks copy and the build bundles.
pub struct BuiltinPanel {
    pub id: String,
    /// Parks while closed: its isolate goes until it opens again. The viewer
    /// and timeline are always shown, the agent holds live runs, and window
    /// panels close with their window.
    pub parks: bool,
    /// Mounts with no project open (the application window).
    pub application: bool,
    /// Shown in its own window rather than docked.
    pub window: bool,
}
pub static BUILTIN_PANELS: std::sync::LazyLock<Vec<BuiltinPanel>> =
    std::sync::LazyLock::new(|| {
        let list: Vec<Value> = serde_json::from_str(include_str!("../../../js/v4/app/panels.json"))
            .expect("panels.json is a list");
        list.iter()
            .map(|p| {
                let flag = |name: &str| p[name].as_bool().unwrap_or(false);
                BuiltinPanel {
                    id: p["id"].as_str().expect("panel id").to_owned(),
                    parks: flag("parks"),
                    application: flag("application"),
                    window: flag("window"),
                }
            })
            .collect()
    });
pub fn builtin_panel(id: &str) -> Option<&'static BuiltinPanel> {
    BUILTIN_PANELS.iter().find(|p| p.id == id)
}
const BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/builtins.js"));
/// Each built-in panel's source as a v4 extension tree (`scripts/builtin-sources.ts`).
const SOURCES: &str = include_str!(concat!(env!("OUT_DIR"), "/builtin-sources.json"));

/// The shipped built-in sources as folders under `root`, written once per build
/// and never changed after, so forks always copy what this app runs.
pub fn builtin_sources(root: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    use std::{fs, os::unix::fs::PermissionsExt};
    let stamp = {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        SOURCES.hash(&mut hash);
        format!("{:016x}", hash.finish())
    };
    let dir = root.join(&stamp);
    if dir.join(".complete").exists() {
        return Ok(dir);
    }
    let trees: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(SOURCES).map_err(std::io::Error::other)?;
    let scratch = root.join(format!(".{stamp}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    for (id, files) in trees {
        for (path, text) in files {
            let file = scratch.join(&id).join(path);
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, text)?;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o444))?;
        }
    }
    fs::write(scratch.join(".complete"), stamp.as_bytes())?;
    match fs::rename(&scratch, &dir) {
        Ok(()) => Ok(dir),
        // Another window wrote the same build's copy first.
        Err(_) if dir.join(".complete").exists() => {
            let _ = fs::remove_dir_all(&scratch);
            Ok(dir)
        }
        Err(error) => Err(error),
    }
}

/// A v4 extension's viewer overlay publishes as `<panel>@viewer`.
pub const VIEWER_OVERLAY: &str = "@viewer";
/// A `themes.activate` request: its number, panel and theme id (`None`: the
/// built-in look).
pub type ThemeRequest = (u64, String, Option<String>);
/// A deduplicated agent-tool thread request: fingerprint, thread, run.
type ThreadRequest = (String, String, Option<String>);
#[derive(Clone, Debug)]
pub struct Publication {
    pub panel: String,
    pub generation: u64,
    pub surface: Arc<Surface>,
}
#[derive(Clone, Debug)]
pub struct PlatformRequest {
    pub id: u64,
    pub panel: String,
    pub generation: u64,
    pub operation: String,
    pub input: Value,
}
struct Entry {
    generation: u64,
    backend: Arc<Mutex<Backend>>,
    router: HostRouter<HostDataService<data::Data>>,
    surface: Surface,
    interaction: Interaction,
    /// A v4 extension's viewer overlay (`overlays.viewer`), display only.
    overlay: Surface,
}
/// Completed byte receipts stay native; the JSX panel receives metadata only.
#[derive(Clone)]
pub struct RetainedAsset {
    pub id: String,
    pub metadata: pm_model::Value,
    pub original: pm_kernel::media_archive::Original,
    pub previous: Option<pm_kernel::media_archive::Original>,
}

/// One host/pool per project window. A realm and native state exist separately
/// for every extension; there is no editor-isolate shortcut for v4 built-ins.
pub struct Builtins {
    pool: RuntimePool,
    broker: Arc<Broker>,
    /// Built-in panels by id; a fork that replaces one runs in its place here.
    realms: Mutex<BTreeMap<String, Realm>>,
    extension_realms: Mutex<BTreeMap<String, Realm>>,
    /// User forks running in place of a built-in (`replace`): the fork's id
    /// and module graph by panel, so a parked fork starts again as itself.
    forks: Mutex<BTreeMap<String, (String, Value)>>,
}
struct Broker {
    /// Third-party v4 surface extensions mounted in this window.
    extensions: Mutex<std::collections::BTreeSet<String>>,
    project: String,
    application: bool,
    kernel: KernelHandle,
    /// Each panel's entry has its own lock: one panel's slow poll never holds
    /// another's calls (all nine built-ins share this broker).
    entries: Mutex<BTreeMap<String, Arc<Mutex<Entry>>>>,
    publications: Mutex<BTreeMap<String, Publication>>,
    platform: Mutex<Vec<PlatformRequest>>,
    errors: Mutex<Vec<String>>,
    facts: Mutex<Value>,
    platform_replies: Mutex<BTreeMap<u64, Value>>,
    timeline_requests: Mutex<BTreeMap<u64, (String, u64)>>,
    platform_ids: std::sync::atomic::AtomicU64,
    wake: Arc<dyn Fn() + Send + Sync>,
    agent: Arc<agent_host::AgentLink>,
    /// Runs carried in from the previous application window, for its Agent panel.
    carried: Mutex<Option<agent_host::SharedAgent>>,
    /// Set once `carry_agent` hands the runs on: closing no longer stops them.
    agent_released: std::sync::atomic::AtomicBool,
    /// Agent-tool thread requests by caller and clientRequestId: the request's
    /// fingerprint, its thread and run (main dedupes retries the same way).
    thread_requests: Mutex<BTreeMap<String, ThreadRequest>>,
    /// Commands each extension realm registered (`commands.register`), by panel,
    /// with a revision that moves on every change.
    commands: Mutex<(u64, BTreeMap<String, Value>)>,
    /// Themes each extension realm offers (`themes.register`), by panel.
    themes: Mutex<(u64, BTreeMap<String, Value>)>,
    /// The latest `themes.activate` in this window: a counter, and the last
    /// request (its number, panel and theme id, `None` for the built-in look).
    theme_request: Mutex<(u64, Option<ThemeRequest>)>,
    /// Realms with a clip painter (`timeline.paint`), and a revision that
    /// moves whenever one arrives or leaves.
    painters: Mutex<(u64, std::collections::BTreeSet<String>)>,
    /// The `timeline.chrome` writer (`chrome.rs`), started on first use.
    chrome: Mutex<Option<std::sync::mpsc::Sender<chrome::ChromeEdit>>>,
    /// Set while the window closes: its realms' ends are not removals.
    closing: std::sync::atomic::AtomicBool,
    /// Built-in panels parked while closed (`Builtins::park`).
    parked: Mutex<std::collections::BTreeSet<String>>,
    /// Viewer overlays an extension took down (`overlays.viewer`'s dispose).
    overlays_cleared: Mutex<Vec<String>>,
}
impl Builtins {
    pub fn new(
        kernel: KernelHandle,
        project: &str,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::create(kernel, project, false, wake, None)
    }
    /// Application-owned Agent/Settings without inventing an open document.
    /// `carry` holds runs from an earlier application window; its panel shows them.
    pub fn application(
        kernel: KernelHandle,
        wake: impl Fn() + Send + Sync + 'static,
        carry: Option<AgentCarry>,
    ) -> Result<Self> {
        Self::create(kernel, "builtin-application", true, wake, carry)
    }
    /// Keep the application Agent panel's live runs going past this window, e.g.
    /// when a run opens a project. `None` when nothing is running.
    pub fn carry_agent(&self) -> Option<AgentCarry> {
        if !self.broker.application {
            return None;
        }
        let entry = self.broker.entry("agent")?;
        let state = entry
            .lock()
            .unwrap()
            .backend
            .lock()
            .unwrap()
            .native
            .agent
            .clone()?;
        let carry = AgentCarry::take(&self.broker.agent, &state)?;
        self.broker
            .agent_released
            .store(true, std::sync::atomic::Ordering::Release);
        Some(carry)
    }
    /// thread_launch, thread_send and thread_cancel from an agent tool, through
    /// this window's Agent panel and its provider runs (main's controlThread):
    /// the composer's own send, steer, answer and stop effects, without
    /// switching the person's visible thread. New threads and new turns run at
    /// the caller's access (`callerAccess`); a caller never steers a thread
    /// with broader access. Call it off the main thread: saves and launches
    /// reach the kernel and disk, and a reconfiguration waits for a stop.
    pub fn agent_thread_control(
        &self,
        tool: &str,
        args: &Value,
    ) -> std::result::Result<Value, String> {
        use pm_agents::Access;
        use pm_agents::Provider;
        let closed = || {
            "Open the Agent panel in this project's window before controlling its threads."
                .to_owned()
        };
        let missing = || "This thread does not exist in the current project.".to_owned();
        let failed = |e: pm_panel_agentpanel::Error| format!("{e:?}");
        let (agent, link) = {
            let backend = self.backend("agent").map_err(|_| closed())?;
            let backend = backend.lock().unwrap();
            let agent = backend.native.agent.clone().ok_or_else(closed)?;
            (agent, backend.native.agent_link.clone())
        };
        // An unknown caller gets the narrowest access: planning.
        let caller = match args["callerAccess"].as_str() {
            Some("project") => "project",
            _ => "editor",
        };
        let caller_run = args["callerRunId"].as_str().unwrap_or_default();
        let thread_arg = args["threadId"].as_str().unwrap_or_default().to_owned();
        let describe = |thread: &str, extra: Value| -> std::result::Result<Value, String> {
            let state = agent.lock().unwrap();
            let session = state.panel.session(thread).map_err(|_| missing())?;
            let mut out = json!({"threadId":thread,"runId":session.run.as_ref().map(|r| r.id.clone()),
                "busy":session.run.is_some(),"provider":session.settings.provider.name(),
                "model":session.settings.model,"reasoningEffort":session.settings.effort,
                "access":if session.settings.access == Access::Editor { "editor" } else { "project" },
                "status":if session.run.is_some() { "running" } else if tool == "thread_cancel" { "cancelled" } else { "idle" }});
            if let (Some(out), Some(extra)) = (out.as_object_mut(), extra.as_object()) {
                out.extend(extra.clone());
            }
            Ok(out)
        };
        let run_effect = |effect: &pm_panel_agentpanel::model::Effect| {
            agent_host::run_effect(
                &agent,
                &link,
                &self.broker.kernel,
                self.broker.application,
                effect,
            )
        };
        let save = |state: &mut agent_host::AgentState| {
            state
                .panel
                .save(
                    &mut models::KernelStore::new(
                        self.broker.kernel.clone(),
                        self.broker.application,
                    ),
                    agent_host::now_ms(),
                )
                .map_err(|e| format!("{e:?}"))
        };
        if tool == "thread_cancel" {
            let effect = {
                let mut state = agent.lock().unwrap();
                let running = state
                    .panel
                    .session(&thread_arg)
                    .map_err(|_| missing())?
                    .run
                    .is_some();
                running
                    .then(|| state.panel.stop(&thread_arg))
                    .transpose()
                    .map_err(failed)?
            };
            if let Some(effect) = effect {
                run_effect(&effect)?;
            }
            link.touch();
            return describe(&thread_arg, json!({}));
        }
        if self.broker.application && caller == "editor" {
            return Err("Start an app thread with Project access. Planning threads need an attached project.".into());
        }
        // A retried request returns its first answer; a reused id refuses.
        let target = &args["target"];
        let fingerprint = json!({"title":args["title"],"message":args["message"],"target":target,"access":caller}).to_string();
        let request_key = args["clientRequestId"].as_str().map(|id| {
            format!(
                "{tool}\u{0}{caller_run}\u{0}{}\u{0}{id}",
                if tool == "thread_send" {
                    thread_arg.as_str()
                } else {
                    ""
                }
            )
        });
        if let Some(key) = &request_key
            && let Some((seen, thread, run)) = self
                .broker
                .thread_requests
                .lock()
                .unwrap()
                .get(key)
                .cloned()
        {
            if seen != fingerprint {
                return Err(if tool == "thread_launch" {
                    "This clientRequestId belongs to another thread launch.".into()
                } else {
                    "This clientRequestId belongs to another message.".into()
                });
            }
            return describe(
                &thread,
                if tool == "thread_launch" {
                    json!({})
                } else {
                    json!({"runId":run,"delivery":"already_sent"})
                },
            );
        }
        if tool == "thread_send" && request_key.is_some() {
            let prefix = format!("thread_send\u{0}{caller_run}\u{0}{thread_arg}\u{0}");
            if self
                .broker
                .thread_requests
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .count()
                >= 64
            {
                return Err("This thread has reached its saved update limit. Start a new thread for further orchestrated work.".into());
            }
        }
        let provider_of = |name: &str| match name {
            "chatgpt" => Provider::Chatgpt,
            "claude" => Provider::Claude,
            _ => Provider::Compatible,
        };
        let message = args["message"].as_str().unwrap_or_default();
        let now = agent_host::now_ms();
        // What the request asks for, against the thread's current settings.
        let (thread, steer, reconfigure) = {
            let mut state = agent.lock().unwrap();
            let base = if tool == "thread_send" {
                thread_arg.clone()
            } else {
                state.panel.active_id().to_owned()
            };
            let session = state.panel.session(&base).map_err(|_| missing())?;
            let provider = target["providerInstanceId"]
                .as_str()
                .map_or(session.settings.provider, provider_of);
            if !link.available(provider) {
                let name = match provider {
                    Provider::Chatgpt => "ChatGPT",
                    Provider::Claude => "Claude",
                    Provider::Compatible => "your compatible provider",
                };
                return Err(format!(
                    "Sign in to {name} in Settings › Agents before starting a run."
                ));
            }
            if tool == "thread_launch" {
                let id = pm_kernel::new_id("agent-thread");
                state.panel.launch_background(&id, now).map_err(failed)?;
                (id, None, false)
            } else {
                let busy = session.run.as_ref().map(|r| r.id.clone());
                if busy.is_some()
                    && (session.settings.access == Access::Computer
                        || caller == "editor" && session.settings.access != Access::Editor)
                {
                    return Err("This thread has broader permissions than the caller.".into());
                }
                if session.applying {
                    return Err("This thread is applying its current change. Wait before sending new direction.".into());
                }
                let changes = target["providerInstanceId"]
                    .as_str()
                    .is_some_and(|p| provider_of(p) != session.settings.provider)
                    || target["model"]
                        .as_str()
                        .is_some_and(|m| m != session.settings.model)
                    || target["options"]["reasoningEffort"]
                        .as_str()
                        .is_some_and(|e| e != session.settings.effort);
                (
                    thread_arg.clone(),
                    busy.clone().filter(|_| !changes),
                    busy.is_some() && changes,
                )
            }
        };
        // A reconfigured run stops first; the new turn carries the request on.
        if reconfigure {
            let effect = agent.lock().unwrap().panel.stop(&thread).map_err(failed)?;
            run_effect(&effect)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while agent
                .lock()
                .unwrap()
                .panel
                .session(&thread)
                .is_ok_and(|s| s.run.is_some())
            {
                if std::time::Instant::now() > deadline {
                    return Err("The running turn did not stop in time. Its work continues; try again shortly.".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        let effect = {
            let mut state = agent.lock().unwrap();
            if steer.is_none() {
                // A new turn runs at the caller's access and the target settings.
                if let Some(effect) = state.panel.set_access(&thread, caller).map_err(failed)? {
                    drop(state);
                    run_effect(&effect)?;
                    state = agent.lock().unwrap();
                }
                if let Some(provider) = target["providerInstanceId"].as_str().map(provider_of) {
                    let model = target["model"].as_str().unwrap_or_default().to_owned();
                    state
                        .panel
                        .set_provider(&thread, provider, model, link.catalog(provider))
                        .map_err(failed)?;
                }
                let current = state
                    .panel
                    .session(&thread)
                    .map_err(|_| missing())?
                    .settings
                    .clone();
                let model = target["model"]
                    .as_str()
                    .map_or(current.model.clone(), str::to_owned);
                let effort = target["options"]["reasoningEffort"]
                    .as_str()
                    .map_or(current.effort.clone(), str::to_owned);
                if model != current.model || effort != current.effort {
                    state
                        .panel
                        .set_model(&thread, model, effort)
                        .map_err(failed)?;
                }
                agent_host::admit_send(&state, &thread)?;
            }
            let run = steer.clone().unwrap_or_else(|| pm_kernel::new_id("run"));
            let effect = state
                .panel
                .send_message(
                    &thread,
                    message,
                    &[],
                    &run,
                    &pm_kernel::new_id("request"),
                    now,
                )
                .map_err(failed)?;
            if tool == "thread_launch"
                && let Some(title) = args["title"].as_str()
            {
                let _ = state.panel.generated_title(&thread, message, title, now);
            }
            save(&mut state)?;
            effect
        };
        let delivery = match (&effect.kind, steer.is_some()) {
            (pm_panel_agentpanel::model::EffectKind::Answer { .. }, _) => "answered",
            (_, true) => "live",
            _ => "started",
        };
        run_effect(&effect)?;
        link.touch();
        let result = describe(
            &thread,
            if tool == "thread_send" {
                json!({"delivery":delivery})
            } else {
                json!({})
            },
        )?;
        if let Some(key) = request_key {
            self.broker.thread_requests.lock().unwrap().insert(
                key,
                (
                    fingerprint,
                    thread,
                    result["runId"].as_str().map(str::to_owned),
                ),
            );
        }
        Ok(result)
    }
    /// Main's orchestrator tools for the run named by `agentRunId` (its MCP
    /// session's `--agent-run`): subagent tasks and run watches, through this
    /// window's Agent panel runs. Call it off the main thread.
    pub fn agent_orchestrate(
        &self,
        tool: &str,
        args: &Value,
    ) -> std::result::Result<Value, String> {
        let (agent, link) = {
            let backend = self.backend("agent").map_err(|_| {
                "Open the Agent panel in this project's window before delegating.".to_owned()
            })?;
            let backend = backend.lock().unwrap();
            let agent = backend
                .native
                .agent
                .clone()
                .ok_or("This agent run is no longer active.")?;
            (agent, backend.native.agent_link.clone())
        };
        let result = agent_tasks::call(&agent, &link, tool, args);
        agent_host::ensure_worker(&agent, &link, &self.broker.kernel, self.broker.application);
        link.touch();
        link.publish_now();
        result
    }
    /// publish_visual_reply for the run named by `agentRunId`: validated and
    /// normalized as main does, saved on the run's thread before it answers.
    pub fn agent_visual_reply(&self, args: &Value) -> std::result::Result<Value, String> {
        const GONE: &str = "The visualization’s agent run is no longer active.";
        let (agent, link) = {
            let backend = self.backend("agent").map_err(|_| GONE.to_owned())?;
            let backend = backend.lock().unwrap();
            let agent = backend.native.agent.clone().ok_or(GONE)?;
            (agent, backend.native.agent_link.clone())
        };
        let run = args["agentRunId"].as_str().ok_or(GONE)?;
        let mut input = args.clone();
        if let Some(object) = input.as_object_mut() {
            for key in ["agentRunId", "callerAccess", "callerRunId"] {
                object.remove(key);
            }
        }
        let prepared = pm_agents::visual::prepare(&input)?;
        let id = format!("visual-{}", pm_kernel::new_id("reply"));
        let thread = {
            let mut state = agent.lock().unwrap();
            let thread = state.runs.live(run).ok_or(GONE)?.thread.clone();
            let artifact = json!({"version":1,"id":id,"runId":run,"title":prepared.title,
                "html":prepared.html,"height":prepared.height});
            state
                .panel
                .add_visual_reply(
                    &mut models::KernelStore::new(
                        self.broker.kernel.clone(),
                        self.broker.application,
                    ),
                    &thread,
                    artifact,
                    agent_host::now_ms(),
                )
                .map_err(|e| match e {
                    pm_panel_agentpanel::Error::Invalid("four visual replies") => {
                        "This run already published four visual replies. Continue in a follow-up message.".to_owned()
                    }
                    pm_panel_agentpanel::Error::UnknownId(_) | pm_panel_agentpanel::Error::Stale => {
                        "The visualization’s thread was removed.".to_owned()
                    }
                    _ => "The visual reply could not be saved. Free storage and try again.".to_owned(),
                })?;
            thread
        };
        link.touch();
        link.publish_now();
        Ok(json!({"id":id,"title":prepared.title,"saved":true,"threadId":thread}))
    }
    /// Files the person chose with Add Attachments, as attachment records
    /// (`{id, name, type, size, dataUrl | content | dataBase64}`), into the
    /// visible thread's message. At most six per message, as on main.
    pub fn agent_attach(&self, records: Vec<Value>) -> std::result::Result<(), String> {
        let (agent, link) = {
            let backend = self
                .backend("agent")
                .map_err(|_| "The Agent panel closed.")?;
            let backend = backend.lock().unwrap();
            (
                backend
                    .native
                    .agent
                    .clone()
                    .ok_or("The Agent panel closed.")?,
                backend.native.agent_link.clone(),
            )
        };
        let files = records
            .into_iter()
            .map(pm_panel_agentpanel::composer::Attachment::from_record)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| format!("{e:?}"))?;
        {
            let mut state = agent.lock().unwrap();
            let thread = state.panel.active_id().to_owned();
            state
                .panel
                .add_attachments(&thread, files, agent_host::now_ms())
                .map_err(|e| match e {
                    pm_panel_agentpanel::Error::Invalid("six attachments per message") => {
                        "A message can carry six attachments.".to_owned()
                    }
                    e => format!("{e:?}"),
                })?;
            // Called off the main thread: the archive write happens here.
            state
                .panel
                .save(
                    &mut models::KernelStore::new(
                        self.broker.kernel.clone(),
                        self.broker.application,
                    ),
                    agent_host::now_ms(),
                )
                .map_err(|e| format!("{e:?}"))?;
        }
        link.touch();
        link.publish_now();
        Ok(())
    }
    /// What `thread_read` adds from this window's live Agent panel: the
    /// running trace as text, open questions, and the run. `None` when the
    /// panel holds no session for the thread.
    pub fn agent_thread_live(&self, thread: &str) -> Option<Value> {
        let shared = {
            let backend = self.backend("agent").ok()?;
            let state = backend.lock().unwrap().native.agent.clone();
            state.or_else(|| self.broker.carried.lock().unwrap().clone())?
        };
        let state = shared.lock().unwrap();
        let session = state.panel.session(thread).ok()?;
        let busy = session.run.is_some();
        let steps: Vec<Value> = session.trace.rows.values().cloned().collect();
        let mut activity = if busy {
            pm_agents::conversation::message_text(&json!({"role":"trace","steps":steps}))
        } else {
            String::new()
        };
        // Main keeps the last 20,000 UTF-16 units.
        let units: Vec<u16> = activity.encode_utf16().collect();
        if units.len() > 20_000 {
            activity = String::from_utf16_lossy(&units[units.len() - 20_000..]);
        }
        let questions: Vec<Value> = session
            .trace
            .rows
            .page(0..session.trace.rows.len())
            .iter()
            .filter(|row| row.value["kind"] == "question" && row.value["status"] == "open")
            .map(|row| {
                json!({"itemId":row.id,"blocking":row.value["blocking"] != false,
                    "questions":row.value["questions"].clone()})
            })
            .collect();
        Some(
            json!({"busy":busy,"runId":session.run.as_ref().map(|run| run.id.clone()),
            "activity":activity,"questions":questions}),
        )
    }
    fn create(
        kernel: KernelHandle,
        project: &str,
        application: bool,
        wake: impl Fn() + Send + Sync + 'static,
        carry: Option<AgentCarry>,
    ) -> Result<Self> {
        let (agent, carried) = match carry.map(AgentCarry::adopt) {
            Some((link, state)) => (link, Some(state)),
            None => Default::default(),
        };
        let broker = Arc::new(Broker {
            extensions: Default::default(),
            project: project.into(),
            application,
            kernel,
            entries: Default::default(),
            publications: Default::default(),
            platform: Default::default(),
            errors: Default::default(),
            facts: Mutex::new(json!({})),
            platform_replies: Default::default(),
            timeline_requests: Default::default(),
            platform_ids: std::sync::atomic::AtomicU64::new(1),
            wake: Arc::new(wake),
            agent,
            carried: Mutex::new(carried),
            agent_released: Default::default(),
            thread_requests: Default::default(),
            commands: Default::default(),
            themes: Default::default(),
            overlays_cleared: Default::default(),
            parked: Default::default(),
            theme_request: Default::default(),
            painters: Default::default(),
            chrome: Default::default(),
            closing: Default::default(),
        });
        let pool = RuntimePool::new(
            format!("window:{project}"),
            Config {
                limits: Limits {
                    // A mounted built-in holds about 10 MiB.
                    initial_heap_bytes: 32 * 1024 * 1024,
                    ..Limits::default()
                },
                watchdog: WatchdogPolicy::default(),
                kernel: broker.clone(),
                host: broker.clone(),
                watchdog_hooks: broker.clone(),
                engine: Some(broker.clone()),
                cache: CodeCache::new(4 * 1024 * 1024),
                // The process's one cage account: each isolate (the nine
                // built-ins and the user's v4 extensions, §3.4/§3.7) starts
                // at a 32 MiB slice and grows toward its 64 MiB ceiling only
                // while every window's isolates together leave room. Closed
                // panels park to give theirs back.
                admission: Admission::cage(),
            },
        )?;
        Ok(Self {
            pool,
            broker,
            realms: Mutex::default(),
            extension_realms: Mutex::default(),
            forks: Mutex::default(),
        })
    }
    /// Ticket is awaited off the UI handler. First publication is delivered only
    /// after Rust accepts the complete atomic surface commit.
    pub fn mount(&mut self, panel: &str) -> Result<Ticket<Evaluation>> {
        self.mount_builtin(panel)
    }
    /// Parks a closed built-in panel: its isolate goes and its reservation
    /// returns to the cage account; `unpark` starts it again from its first
    /// commit. Waits (bounded) for the isolate: call off the main thread.
    pub fn park(&self, panel: &str) -> bool {
        if !builtin_panel(panel).is_some_and(|p| p.parks)
            || !self.realms.lock().unwrap().contains_key(panel)
        {
            return false;
        }
        self.retire(panel);
        self.broker.parked.lock().unwrap().insert(panel.into());
        true
    }
    /// A parked panel opened again, as the fork that replaced it if any.
    pub fn unpark(&self, panel: &str) -> Option<Result<Ticket<Evaluation>>> {
        if !self.broker.parked.lock().unwrap().remove(panel) {
            return None;
        }
        let fork = self.forks.lock().unwrap().get(panel).cloned();
        Some(match fork {
            Some((fork, graph)) => self.start(panel, fork_source(&fork, &graph, panel)),
            None => self.mount_builtin(panel),
        })
    }
    pub fn is_parked(&self, panel: &str) -> bool {
        self.broker.parked.lock().unwrap().contains(panel)
    }
    fn mount_builtin(&self, panel: &str) -> Result<Ticket<Evaluation>> {
        let Some(builtin) = builtin_panel(panel) else {
            return Err(Error::Invalid("unregistered/already mounted built-in"));
        };
        if self.realms.lock().unwrap().contains_key(panel)
            || (self.broker.application && !builtin.application)
        {
            return Err(Error::Invalid("unregistered/already mounted built-in"));
        }
        self.start(
            panel,
            format!("{BUNDLE}\n__pmBuiltin.mount({})", json!(panel)),
        )
    }
    fn start(&self, panel: &str, source: String) -> Result<Ticket<Evaluation>> {
        let realm = self.pool.realm(RealmKind::V4Extension(panel.into()))?;
        if matches!(panel, "agent" | "settings") {
            self.broker.agent.add_realm(realm.clone());
        }
        let ticket = realm.eval(source)?;
        self.realms.lock().unwrap().insert(panel.into(), realm);
        Ok(ticket)
    }
    /// Retire a built-in panel's realm, its writer and its surface, so the next
    /// mount of that id starts from its own first commit.
    /// Waits (bounded) for the old isolate to go, so its heap reservation is
    /// back before the next one starts: call off the main thread.
    fn retire(&self, panel: &str) {
        let realm = self.realms.lock().unwrap().remove(panel);
        if let Some(realm) = realm {
            realm.close();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !realm.is_disposed() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        self.broker.entries.lock().unwrap().remove(panel);
        self.broker.publications.lock().unwrap().remove(panel);
        self.broker.forget_commands(panel);
        self.broker.forget_themes(panel);
        self.broker.forget_overlay(panel);
        self.broker.forget_timeline(panel, None);
        self.broker.forget_painter(panel);
    }
    /// A user fork (manifest `replaces`) draws the built-in `panel` instead of
    /// it: same realm identity, routes and host operations, the fork's code.
    /// Waits for the old isolate to close, so call it off the main thread.
    pub fn replace(&self, panel: &str, fork: &str, graph: &Value) -> Result<Ticket<Evaluation>> {
        if builtin_panel(panel).is_none_or(|p| self.broker.application && !p.application) {
            return Err(Error::Invalid("unregistered built-in"));
        }
        self.retire(panel);
        self.forks
            .lock()
            .unwrap()
            .insert(panel.into(), (fork.into(), graph.clone()));
        self.start(panel, fork_source(fork, graph, panel))
    }
    /// The fork went away: the shipped panel comes back.
    pub fn restore(&self, panel: &str) -> Result<Ticket<Evaluation>> {
        self.retire(panel);
        self.forks.lock().unwrap().remove(panel);
        self.broker.parked.lock().unwrap().remove(panel);
        self.mount_builtin(panel)
    }
    /// Realms running user code: third-party extensions and forks of
    /// built-ins. Menus, clip painters and viewer overlays come from these.
    fn user_realms(&self) -> Vec<(String, Realm)> {
        let forks = self.forks.lock().unwrap();
        let realms = self.realms.lock().unwrap();
        let mut out: Vec<(String, Realm)> = forks
            .keys()
            .filter_map(|panel| Some((panel.clone(), realms.get(panel)?.clone())))
            .collect();
        drop((realms, forks));
        out.extend(
            self.extension_realms
                .lock()
                .unwrap()
                .iter()
                .map(|(id, realm)| (id.clone(), realm.clone())),
        );
        out
    }
    /// What agent runs may use. Without this the panel reports no connection.
    pub fn set_agent_environment(
        &self,
        environment: pm_agents::launch::Environment,
        open: std::sync::Arc<pm_agents::account::Opener>,
    ) {
        self.broker.agent.set_environment(environment, open);
    }
    /// Who hears that an agent run was sent, finished or asked a question.
    pub fn set_agent_notices(&self, sink: std::sync::Arc<NoticeSink>) {
        self.broker.agent.set_notices(sink);
    }
    /// A third-party v4 extension: its own isolate, the same host and native
    /// surface path as a built-in, and a module graph from the native compiler.
    /// `hot` is what the previous version left in `api.hot.data` (`hot()`).
    pub fn mount_extension(
        &self,
        id: &str,
        graph: &Value,
        hot: Option<&str>,
    ) -> Result<Ticket<Evaluation>> {
        let mut extensions = self.extension_realms.lock().unwrap();
        if builtin_panel(id).is_some() || extensions.contains_key(id) || self.broker.application {
            return Err(Error::Invalid("extension panel id is taken"));
        }
        self.broker.extensions.lock().unwrap().insert(id.into());
        let realm = self.pool.realm(RealmKind::V4Extension(id.into()))?;
        let ticket = realm.eval(format!(
            "{BUNDLE}\n__pmBuiltin.mountExtension({},{},undefined,{})",
            json!(id),
            json!(graph.to_string()),
            hot.map_or(Value::Null, |h| json!(h)),
        ))?;
        extensions.insert(id.into(), realm);
        Ok(ticket)
    }
    /// Before a rebuilt extension reloads: its `api.hot.data` as JSON text.
    pub fn hot(&self, id: &str) -> Result<Ticket<Evaluation>> {
        self.realm_of(id)?.eval("__pmBuiltin.hot()")
    }
    /// A surface that was disabled, removed or rebuilt. Closing the realm revokes
    /// its writer on the isolate owner; the same id can mount again.
    pub fn unmount_extension(&self, id: &str) {
        if let Some(realm) = self.extension_realms.lock().unwrap().remove(id) {
            realm.close();
        }
        self.broker.extensions.lock().unwrap().remove(id);
        // Its surface and router go too; a late call from it is a stale generation.
        self.broker.entries.lock().unwrap().remove(id);
        self.broker.forget_commands(id);
        self.broker.forget_themes(id);
        self.broker.forget_overlay(id);
        self.broker.forget_timeline(id, None);
        self.broker.forget_painter(id);
    }
    /// Every extension command in this window when they changed since `seen`:
    /// `(revision, [{panel, id, title, key}])`.
    /// Themes v4 extensions offer, when they changed since `seen`: each
    /// `{panel, id, title, tokens}`.
    pub fn themes_since(&self, seen: u64) -> Option<(u64, Vec<Value>)> {
        let themes = self.broker.themes.lock().unwrap();
        (themes.0 != seen).then(|| {
            let list = themes
                .1
                .iter()
                .flat_map(|(panel, list)| {
                    list.as_array().into_iter().flatten().map(move |t| {
                        let mut t = t.clone();
                        t["panel"] = json!(panel);
                        t
                    })
                })
                .collect();
            (themes.0, list)
        })
    }
    pub fn commands_since(&self, seen: u64) -> Option<(u64, Vec<Value>)> {
        let commands = self.broker.commands.lock().unwrap();
        (commands.0 != seen).then(|| {
            let list = commands
                .1
                .iter()
                .flat_map(|(panel, list)| {
                    list.as_array().into_iter().flatten().map(move |c| {
                        json!({"panel": panel, "id": c["id"], "title": c["title"], "key": c["key"]})
                    })
                })
                .collect();
            (commands.0, list)
        })
    }
    /// Runs a command an extension registered, in that extension's realm.
    /// True while any user realm runs (context menus ask them).
    pub fn has_extensions(&self) -> bool {
        !self.forks.lock().unwrap().is_empty() || !self.extension_realms.lock().unwrap().is_empty()
    }
    /// The latest `themes.activate` in this window: `(number, panel, id)`,
    /// with `id` `None` for the built-in look. Numbers only grow.
    pub fn theme_request(&self) -> Option<ThemeRequest> {
        self.broker.theme_request.lock().unwrap().1.clone()
    }
    /// Every v4 extension's items for a menu opening at `location` (each
    /// realm waits at most 100 ms on its own contributions).
    pub fn gather_menus(
        &self,
        location: &str,
        ctx: &Value,
    ) -> Vec<(String, Result<Ticket<Evaluation>>)> {
        let script = format!("__pmBuiltin.gatherMenu({},{})", json!(location), ctx);
        self.user_realms()
            .into_iter()
            .map(|(id, realm)| (id, realm.eval(script.clone())))
            .collect()
    }
    /// The person chose `key` from the items `panel` gave the last gather.
    pub fn run_menu(&self, panel: &str, key: &str) -> Result<Ticket<Evaluation>> {
        self.realm_of(panel)?
            .eval(format!("__pmBuiltin.runMenu({})", json!(key)))
    }
    /// Clip decorations for the timeline's visible clips from each realm
    /// with a painter (`timeline.paint`); each answers JSON text.
    pub fn paint_clips(&self, clips: &Value) -> Vec<(String, Result<Ticket<Evaluation>>)> {
        let script = format!("__pmBuiltin.paintClips({clips})");
        let painters = self.broker.painters.lock().unwrap().1.clone();
        self.user_realms()
            .into_iter()
            .filter(|(id, _)| painters.contains(id))
            .map(|(id, realm)| (id, realm.eval(script.clone())))
            .collect()
    }
    /// 0 with no clip painter; otherwise moves whenever one arrives or leaves.
    pub fn painters_revision(&self) -> u64 {
        let painters = self.broker.painters.lock().unwrap();
        if painters.1.is_empty() { 0 } else { painters.0 }
    }
    /// Viewer overlays whose extension took them down since the last look.
    pub fn take_cleared_overlays(&self) -> Vec<String> {
        std::mem::take(&mut *self.broker.overlays_cleared.lock().unwrap())
    }
    /// Where the composition sits in the viewer, for extension overlays:
    /// `{x, y, width, height, scale}` in overlay points.
    pub fn viewer_frame(&self, frame: &Value) {
        let script = format!("__pmBuiltin.viewerFrame({frame})");
        for (_, realm) in self.user_realms() {
            let _ = realm.eval(script.clone());
        }
    }
    pub fn run_command(&self, panel: &str, id: &str, args: &[Value]) -> Result<Ticket<Evaluation>> {
        self.realm_of(panel)?.eval(format!(
            "__pmBuiltin.runCommand({},{})",
            json!(id),
            json!(args)
        ))
    }
    fn realm_of(&self, panel: &str) -> Result<Realm> {
        if let Some(realm) = self.realms.lock().unwrap().get(panel) {
            return Ok(realm.clone());
        }
        self.extension_realms
            .lock()
            .unwrap()
            .get(panel)
            .cloned()
            .ok_or(Error::Closed)
    }
    pub fn publish(&self) -> Vec<(String, Result<Ticket<Evaluation>>)> {
        let extensions = self.extension_realms.lock().unwrap();
        let realms = self.realms.lock().unwrap();
        realms
            .iter()
            .chain(extensions.iter())
            .map(|(id, realm)| (id.clone(), realm.eval("__pmBuiltin.publish()")))
            .collect()
    }
    pub fn take_publications(&self) -> Vec<Publication> {
        std::mem::take(&mut *self.broker.publications.lock().unwrap())
            .into_values()
            .collect()
    }
    pub fn update_facts(&self, facts: Value) {
        *self.broker.facts.lock().unwrap() = facts.clone();
        for entry in self.broker.all_entries() {
            entry.lock().unwrap().backend.lock().unwrap().native.facts = facts.clone();
        }
    }
    /// Worker-only native observations. Never expose filesystem paths to JSX.
    pub fn update_media(
        &self,
        originals: BTreeMap<String, std::path::PathBuf>,
        observed: pm_panel_media::readiness::ObservedRuntime,
    ) {
        for entry in self.broker.all_entries() {
            let backend = entry.lock().unwrap().backend.clone();
            let mut backend = backend.lock().unwrap();
            backend.native.originals = originals.clone();
            if backend.native.media_runtime != observed {
                backend.native.media_runtime = observed.clone();
                backend.native.media_sequence += 1;
            }
        }
    }
    /// The Project panel lists only missing files from its next request; the
    /// media sequence moves so it asks again at the next publication. Takes
    /// every panel's locks: call it on a worker, as `update_media` is.
    pub fn show_missing_media(&self) {
        for entry in self.broker.all_entries() {
            let backend = entry.lock().unwrap().backend.clone();
            let mut backend = backend.lock().unwrap();
            backend.native.show_missing = true;
            backend.native.media_sequence += 1;
        }
    }
    /// Admit worker-owned originals, then await the actor's durable outcome.
    /// A metadata-only StoredAsset is never sufficient for live admission.
    pub fn admit_media(
        &self,
        owner: &pm_panel_media::Stamp,
        items: Vec<RetainedAsset>,
        folder: Option<String>,
        replace: Option<String>,
        place: Vec<pm_kernel::Command>,
        caller: pm_kernel::Caller,
    ) -> std::result::Result<
        pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>,
        String,
    > {
        if self.broker.application || owner.project != self.broker.project {
            return Err("Media completion belongs to a different project".into());
        }
        let published = self
            .broker
            .kernel
            .published(&owner.project)
            .ok_or("Project closed")?;
        let catalog = pm_panel_media::Catalog::published(&published)
            .map_err(|_| "The project's media could not be read")?;
        let mut stored = Vec::new();
        let mut bindings = Vec::new();
        for item in items {
            let id = replace.as_ref().unwrap_or(&item.id).clone();
            stored.push(pm_panel_media::StoredAsset {
                id: item.id,
                metadata: item.metadata,
                original_version: item.original.version().into(),
            });
            let expected = published
                .snapshot
                .root()
                .and_then(|r| r.get("assets"))
                .and_then(|a| a.get(&id))
                .map(pm_model::Node::to_value);
            bindings.push(pm_kernel::MediaBinding {
                asset: id,
                expected,
                original: item.original,
                previous: item.previous,
            });
        }
        let mut edit = catalog
            .prepare_import(owner, stored, folder, replace)
            .map_err(|error| match error {
                pm_panel_media::Error::MissingItem => "That folder is no longer in the project",
                _ => "The import no longer matches the project",
            })?;
        if !place.is_empty() {
            edit.label = match place.len() {
                1 => "Import file".to_owned(),
                n => format!("Import {n} files"),
            };
        }
        Ok(self.broker.kernel.call(
            &owner.project,
            edit.request_with_media(caller, bindings, place),
        ))
    }

    pub fn platform_reply(&self, id: u64, result: std::result::Result<Value, String>) {
        if id == 0 {
            return;
        }
        self.broker.platform_replies.lock().unwrap().insert(
            id,
            match result {
                Ok(value) => json!({"accepted":true,"value":value}),
                Err(error) => json!({"accepted":false,"error":error}),
            },
        );
    }
    pub fn observe_export(&self, run: Value) -> Result<Ticket<Evaluation>> {
        self.realm_of("export")?.eval(format!(
            "runtime.engine('export.observe',{});__pmBuiltin.publish()",
            run
        ))
    }
    pub fn show(&self, panel: &str) -> Result<Ticket<Evaluation>> {
        self.realm_of(panel)?.eval("__pmBuiltin.show()")
    }
    /// A shell-admitted mask reveal uses the same model and persistence as M in the timeline.
    pub fn reveal_masks(&self) -> Result<Ticket<Evaluation>> {
        static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let millis = CLOCK
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64;
        self.realm_of("timeline")?.eval(format!(
            "__pmBuiltin.command('panel.timeline',{})",
            json!({"type":"keyDown","millis":millis,"event":{"key":"m","repeat":false}})
        ))
    }
    pub fn timeline_key(
        &self,
        layer: &str,
        channel: &str,
        key: &str,
        additive: bool,
    ) -> Result<Ticket<Evaluation>> {
        self.timeline_action(json!({"type":"selectKeys","layer":layer,"path":channel,"ids":[key],"additive":additive}))
    }
    /// Enqueue on the existing v4 worker. The ticket covers actor receipts and
    /// platform acknowledgements; the main thread only polls it. Drivers can
    /// include decimal document/revision stamps from the inspected frame.
    pub fn timeline_action(&self, intent: Value) -> Result<Ticket<Evaluation>> {
        self.realm_of("timeline")?
            .eval(format!("__pmBuiltin.command('panel.timeline',{})", intent))
    }

    /// Inspect exact library source and CAS revision on the panel worker.
    pub fn timeline_easing_library(&self) -> Result<Ticket<Evaluation>> {
        self.realm_of("timeline")?
            .eval("JSON.stringify(runtime.engine('timeline.easing.library',{}))")
    }
    pub fn write(&self, panel: &str, handle: &str, value: Value) -> Result<Ticket<Evaluation>> {
        self.realm_of(panel)?
            .eval(format!("__pmBuiltin.write({},{})", json!(handle), value))
    }
    pub fn take_platform_requests(&self) -> Vec<PlatformRequest> {
        std::mem::take(&mut *self.broker.platform.lock().unwrap())
    }
    pub fn status(&self) -> Vec<Value> {
        let entries = self.broker.entries.lock().unwrap().clone();
        entries.into_iter().map(|(id,e)|{let e=e.lock().unwrap();json!({"panel":id,"generation":e.generation,"surfaceRevision":e.surface.revision(),"subscriptions":e.router.subscription_count(),"kernelRevision":e.backend.lock().unwrap().service_revision})}).collect()
    }
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.broker.errors.lock().unwrap())
    }
    /// Validate the exact tree revision before enqueueing. An old click must not
    /// activate a newly selected layer after a publication or project switch.
    pub fn event(
        &self,
        panel: &str,
        generation: u64,
        revision: u64,
        target: u64,
        event: &str,
        detail: EventDetail,
    ) -> Result<Ticket<Evaluation>> {
        self.dispatch(panel, generation, revision, target, event, detail, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        panel: &str,
        generation: u64,
        revision: u64,
        target: u64,
        event: &str,
        detail: EventDetail,
        semantic: Option<String>,
    ) -> Result<Ticket<Evaluation>> {
        let entry = self.broker.entry(panel).ok_or(Error::Closed)?;
        let mut entry = entry.lock().unwrap();
        let entry = &mut *entry;
        let mut event = entry
            .interaction
            .drive(
                &mut entry.surface,
                generation,
                entry.generation,
                revision,
                target,
                event,
                detail,
            )
            .map_err(Error::Host)?;
        event.plan.semantic = semantic;
        let wire = serde_json::to_string(&event).map_err(|e| Error::Host(e.to_string()))?;
        self.realm_of(panel)?
            .eval(format!("__pmBuiltin.event({wire})"))
    }
    /// A press may republish before its click lands. Activation still reaches
    /// a node only while it carries the same semantic identity as the one hit.
    #[allow(clippy::too_many_arguments)]
    pub fn activate(
        &self,
        panel: &str,
        generation: u64,
        revision: u64,
        target: u64,
        semantic: Option<&str>,
        event: &str,
        detail: EventDetail,
    ) -> Result<Ticket<Evaluation>> {
        let (revision, semantic) = {
            let entry = self.broker.entry(panel).ok_or(Error::Closed)?;
            let entry = entry.lock().unwrap();
            let surface = &entry.surface;
            let same = semantic.filter(|id| {
                surface
                    .node(target)
                    .and_then(|n| n.props.get("id"))
                    .and_then(Value::as_str)
                    == Some(*id)
            });
            match same {
                Some(id) => (surface.revision(), Some(id.to_owned())),
                None => (revision, None),
            }
        };
        self.dispatch(panel, generation, revision, target, event, detail, semantic)
    }
    /// Pointer stream events address the current tree. A publication landing
    /// mid-gesture must not drop a release.
    pub fn pointer_event(
        &self,
        panel: &str,
        generation: u64,
        target: u64,
        event: &str,
        detail: EventDetail,
    ) -> Result<Ticket<Evaluation>> {
        let revision = self
            .broker
            .entry(panel)
            .ok_or(Error::Closed)?
            .lock()
            .unwrap()
            .surface
            .revision();
        self.event(panel, generation, revision, target, event, detail)
    }
    /// One kernel gesture for a dragged number field, so a scrub is one undo step.
    /// Runs off the main thread; no sample passes through V8.
    pub fn scrub_begin(
        &self,
        panel: &str,
    ) -> Result<pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>>
    {
        let caller = self.backend(panel)?.lock().unwrap().caller;
        Ok(self.broker.kernel.call(
            &self.broker.project,
            pm_kernel::Request::new(
                caller,
                pm_kernel::GestureCall::Begin {
                    label: "Change Property".into(),
                },
            ),
        ))
    }
    pub fn scrub_step(
        &self,
        panel: &str,
        handle: &str,
        gesture: u64,
        value: Value,
    ) -> Result<pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>>
    {
        let backend = self.backend(panel)?;
        let (caller, command) = {
            let mut backend = backend.lock().unwrap();
            let command = backend
                .native_command("panel.binding", &json!({"handle":handle,"value":value}))
                .map_err(Error::Host)?;
            (backend.caller, command)
        };
        let models::NativeCommand::Kernel(request) = command else {
            return Err(Error::Invalid("binding is not a document property"));
        };
        let pm_kernel::Call::Transact { body, .. } = request.call else {
            return Err(Error::Invalid("binding is not a scrubbable edit"));
        };
        Ok(self.broker.kernel.call(
            &self.broker.project,
            pm_kernel::Request::new(
                caller,
                pm_kernel::GestureCall::Step {
                    gesture,
                    body,
                    replace: true,
                },
            ),
        ))
    }
    pub fn scrub_finish(
        &self,
        panel: &str,
        gesture: u64,
        commit: bool,
    ) -> Result<pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>>
    {
        let caller = self.backend(panel)?.lock().unwrap().caller;
        let call = if commit {
            pm_kernel::GestureCall::Commit { gesture }
        } else {
            pm_kernel::GestureCall::Cancel { gesture }
        };
        Ok(self
            .broker
            .kernel
            .call(&self.broker.project, pm_kernel::Request::new(caller, call)))
    }
    fn backend(&self, panel: &str) -> Result<Arc<Mutex<Backend>>> {
        self.broker
            .entry(panel)
            .map(|e| e.lock().unwrap().backend.clone())
            .ok_or(Error::Closed)
    }
    /// Caller drops the owning window without waiting on JS. Runtime disposal
    /// revokes its writer on the isolate owner, including after a failed mount.
    pub fn close_panel(&self, id: &str) {
        if let Ok(realm) = self.realm_of(id) {
            realm.close();
        }
    }
    pub fn close(&self) {
        self.broker
            .closing
            .store(true, std::sync::atomic::Ordering::Release);
        if !self
            .broker
            .agent_released
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.broker.agent.close();
        }
        let extensions = self.extension_realms.lock().unwrap();
        for realm in self
            .realms
            .lock()
            .unwrap()
            .values()
            .chain(extensions.values())
        {
            realm.close();
        }
    }
}
impl Drop for Builtins {
    fn drop(&mut self) {
        self.close();
    }
}
impl Broker {
    fn realm_id(generation: &Generation) -> Option<&str> {
        generation
            .realm
            .rsplit_once("V4Extension(\"")
            .and_then(|(_, s)| s.strip_suffix("\")"))
    }
    fn panel<'a>(&self, generation: &'a Generation) -> Result<&'a str> {
        let id = Self::realm_id(generation).ok_or(Error::Invalid("not a v4 built-in realm"))?;
        if builtin_panel(id).is_none() && !self.extensions.lock().unwrap().contains(id) {
            return Err(Error::Invalid("unknown panel"));
        }
        Ok(id)
    }
    fn report(&self, message: String) {
        let mut errors = self.errors.lock().unwrap();
        if errors.len() < 64 {
            errors.push(message);
        }
        drop(errors);
        (self.wake)();
    }
}
impl EngineProxy for Broker {
    /// Request queues are bounded here (256 host, 128 platform), so a
    /// built-in panel awaiting one reply per property row stays within its
    /// task quota. Extensions mounted through this broker stay metered.
    fn metered(&self, generation: &Generation, operation: &str) -> bool {
        let queue = matches!(
            operation,
            "host.enqueue" | "host.poll" | "platform.enqueue" | "platform.poll"
        );
        !(queue && Self::realm_id(generation).is_some_and(|id| builtin_panel(id).is_some()))
    }
    fn call(
        &self,
        generation: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        let panel = self.panel(generation)?;
        if operation == "identity" {
            let mut entries = self.entries.lock().unwrap();
            if entries.contains_key(panel) {
                return Err(Error::Invalid("duplicate panel identity"));
            }
            let identity = Identity {
                realm: generation.realm.clone(),
                generation: generation.number,
                project: self.project.clone(),
            };
            let backend = Arc::new(Mutex::new(
                Backend::new(
                    self.kernel.clone(),
                    identity.clone(),
                    panel,
                    self.application,
                )
                .map_err(Error::Host)?,
            ));
            backend.lock().unwrap().native.agent_link = self.agent.clone();
            if panel == "agent"
                && let Some(state) = self.carried.lock().unwrap().take()
            {
                backend.lock().unwrap().native.agent = Some(state);
            }
            let facts = self.facts.lock().unwrap().clone();
            if !facts.is_null() && facts.as_object().is_some_and(|m| !m.is_empty()) {
                backend.lock().unwrap().native.facts = facts;
            }
            let router = HostRouter::new(
                identity.clone(),
                HostDataService::new(identity.clone(), data::Data(backend.clone()))
                    .map_err(Error::Host)?,
            )
            .map_err(Error::Host)?;
            entries.insert(
                panel.into(),
                Arc::new(Mutex::new(Entry {
                    generation: generation.number,
                    backend,
                    router,
                    surface: Surface::default(),
                    overlay: Surface::default(),
                    interaction: Interaction::default(),
                })),
            );
            return Ok(serde_json::to_value(identity).unwrap());
        }
        let entry = self.entry(panel).ok_or(Error::StaleGeneration)?;
        let mut entry = entry.lock().unwrap();
        if entry.generation != generation.number {
            return Err(Error::StaleGeneration);
        }
        let entry = &mut *entry;
        match operation {
            "tokens.row" => Ok(json!(
                pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default())
                    .metrics
                    .row
            )),
            "host.enqueue" => {
                entry
                    .router
                    .enqueue(
                        serde_json::from_value(input.clone())
                            .map_err(|e| Error::Host(e.to_string()))?,
                    )
                    .map_err(Error::Host)?;
                Ok(Value::Null)
            }
            "host.poll" => {
                {
                    let mut backend = entry.backend.lock().unwrap();
                    // Drain effect receipts on the panel worker. Main only
                    // deposits replies; it never locks a worker-owned backend.
                    {
                        let mut pending = self.timeline_requests.lock().unwrap();
                        let mut replies = self.platform_replies.lock().unwrap();
                        let ids = pending
                            .iter()
                            .filter(|(_, owner)| owner.0 == panel && owner.1 == generation.number)
                            .map(|(id, _)| *id)
                            .collect::<Vec<_>>();
                        for id in ids {
                            if let Some(reply) = replies.remove(&id) {
                                pending.remove(&id);
                                backend.native.timeline_effect_pending =
                                    backend.native.timeline_effect_pending.saturating_sub(1);
                                if reply["accepted"] != true {
                                    backend.native.timeline_effect_result =
                                        Some(Err(reply["error"]
                                            .as_str()
                                            .unwrap_or("Native timeline effect rejected")
                                            .into()));
                                }
                            }
                        }
                    }
                    backend.tick().map_err(Error::Host)?;
                    let requests = std::mem::take(&mut backend.native.platform);
                    if !requests.is_empty() {
                        let mut queue = self.platform.lock().unwrap();
                        for (operation, input) in requests {
                            let id = if operation == "timeline.effect" {
                                let id = self
                                    .platform_ids
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                self.timeline_requests
                                    .lock()
                                    .unwrap()
                                    .insert(id, (panel.into(), generation.number));
                                id
                            } else {
                                0
                            };
                            queue.push(PlatformRequest {
                                id,
                                panel: panel.into(),
                                generation: generation.number,
                                operation,
                                input,
                            });
                        }
                        (self.wake)();
                    }
                }
                serde_json::to_value(
                    entry
                        .router
                        .poll(input.as_u64().ok_or(Error::Invalid("request id"))?)
                        .map_err(Error::Host)?,
                )
                .map_err(|e| Error::Host(e.to_string()))
            }
            "host.publications" => {
                entry.backend.lock().unwrap().tick().map_err(Error::Host)?;
                serde_json::to_value(entry.router.publications().map_err(Error::Host)?)
                    .map_err(|e| Error::Host(e.to_string()))
            }
            "host.retire" => {
                entry.router.retire().map_err(Error::Host)?;
                Ok(Value::Null)
            }
            "themes.set" => {
                let valid = input.as_array().is_some_and(|list| {
                    list.len() <= 32
                        && list.iter().all(|t| {
                            t["id"]
                                .as_str()
                                .is_some_and(|s| (1..=128).contains(&s.len()))
                                && t["title"]
                                    .as_str()
                                    .is_some_and(|s| (1..=128).contains(&s.len()))
                                && t["tokens"].as_object().is_some_and(|m| m.len() <= 256)
                        })
                });
                if !valid {
                    return Err(Error::Invalid("themes need an id, a title and tokens"));
                }
                let mut themes = self.themes.lock().unwrap();
                themes.0 += 1;
                themes.1.insert(panel.into(), input.clone());
                drop(themes);
                (self.wake)();
                Ok(Value::Null)
            }
            // `themes.activate(id)`: the latest request in the window wins.
            "themes.activate" => {
                let id = match input {
                    Value::Null => None,
                    Value::String(id) if (1..=128).contains(&id.len()) => Some(id.clone()),
                    _ => return Err(Error::Invalid("a theme id or null")),
                };
                let mut request = self.theme_request.lock().unwrap();
                request.0 += 1;
                request.1 = Some((request.0, panel.into(), id));
                drop(request);
                self.themes.lock().unwrap().0 += 1;
                (self.wake)();
                Ok(Value::Null)
            }
            "commands.set" => {
                let valid = input.as_array().is_some_and(|list| {
                    list.len() <= 64
                        && list.iter().all(|c| {
                            c["id"]
                                .as_str()
                                .is_some_and(|s| (1..=128).contains(&s.len()))
                                && c["title"]
                                    .as_str()
                                    .is_some_and(|s| (1..=128).contains(&s.len()))
                                && (c["key"].is_null()
                                    || c["key"].as_str().is_some_and(|k| k.len() <= 64))
                        })
                });
                if !valid {
                    return Err(Error::Invalid("commands need an id and a title"));
                }
                let mut commands = self.commands.lock().unwrap();
                commands.0 += 1;
                commands.1.insert(panel.into(), input.clone());
                drop(commands);
                (self.wake)();
                Ok(Value::Null)
            }
            "surface.commit" => {
                let commit = serde_json::from_value(input.clone())
                    .map_err(|e| Error::Host(e.to_string()))?;
                match entry.surface.apply(commit) {
                    Ok(_) => {
                        entry.interaction.reconcile(&entry.surface);
                        self.publications.lock().unwrap().insert(
                            panel.into(),
                            Publication {
                                panel: panel.into(),
                                generation: generation.number,
                                surface: Arc::new(entry.surface.clone()),
                            },
                        );
                        (self.wake)();
                        Ok(json!({"accepted":true,"revision":entry.surface.revision()}))
                    }
                    Err(error) => Ok(
                        json!({"accepted":false,"revision":entry.surface.revision(),"error":error.to_string()}),
                    ),
                }
            }
            "overlay.commit" => {
                let commit = serde_json::from_value(input.clone())
                    .map_err(|e| Error::Host(e.to_string()))?;
                match entry.overlay.apply(commit) {
                    Ok(_) => {
                        self.publications.lock().unwrap().insert(
                            format!("{panel}{VIEWER_OVERLAY}"),
                            Publication {
                                panel: format!("{panel}{VIEWER_OVERLAY}"),
                                generation: generation.number,
                                surface: Arc::new(entry.overlay.clone()),
                            },
                        );
                        (self.wake)();
                        Ok(json!({"accepted":true,"revision":entry.overlay.revision()}))
                    }
                    Err(error) => Ok(
                        json!({"accepted":false,"revision":entry.overlay.revision(),"error":error.to_string()}),
                    ),
                }
            }
            "timeline.chrome" => {
                let patch = timeline_patch(input);
                if patch.as_object().is_none_or(|p| p.is_empty()) {
                    return Err(Error::Invalid(
                        "timeline chrome needs at least one known field",
                    ));
                }
                self.chrome_edit(chrome::ChromeEdit::Set {
                    panel: panel.into(),
                    generation: generation.number,
                    patch,
                });
                Ok(Value::Null)
            }
            "timeline.painted" => {
                // A painter came (`true`) or went: the timeline asks again.
                let mut painters = self.painters.lock().unwrap();
                painters.0 += 1;
                if input == &json!(true) {
                    painters.1.insert(panel.into());
                } else {
                    painters.1.remove(panel);
                }
                drop(painters);
                (self.wake)();
                Ok(Value::Null)
            }
            "overlay.clear" => {
                // A publication not yet drawn goes too, so it can't come back.
                entry.overlay = Surface::default();
                self.forget_overlay(panel);
                (self.wake)();
                Ok(Value::Null)
            }
            "export.observe" => {
                let mut backend = entry.backend.lock().unwrap();
                backend.native.export["run"] = input.clone();
                backend.native.export["sequence"] =
                    json!(backend.native.export["sequence"].as_u64().unwrap_or(0) + 1);
                Ok(Value::Null)
            }
            "fields.register" => entry
                .backend
                .lock()
                .unwrap()
                .fields(input)
                .map_err(Error::Host),
            "inspector.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .inspector_snapshot()
                .map_err(Error::Host),
            "timeline.easing.library" => Ok(timeline_easing::inspect(&self.kernel)),
            "timeline.frame" => entry
                .backend
                .lock()
                .unwrap()
                .timeline_frame(input)
                .map_err(Error::Host),
            "agent.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .agent_snapshot()
                .map_err(Error::Host),
            "settings.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .settings_snapshot()
                .map_err(Error::Host),
            "project.layout" => Ok(
                json!({"columns":1,"rowHeight":pm_design::Tokens::new(pm_design::Scheme::Dark,Default::default()).metrics.row}),
            ),
            "platform.enqueue" => {
                let mut requests = self.platform.lock().unwrap();
                if requests.len() >= 128 {
                    return Err(Error::Quota("platform queue"));
                }
                let id = self
                    .platform_ids
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (operation, platform_input) = inspector_platform(panel, input)?;
                requests.push(PlatformRequest {
                    id,
                    panel: panel.into(),
                    generation: generation.number,
                    operation: operation.into(),
                    input: platform_input,
                });
                (self.wake)();
                Ok(json!(id))
            }
            "platform.poll" => Ok(self
                .platform_replies
                .lock()
                .unwrap()
                .remove(&input.as_u64().ok_or(Error::Invalid("platform request"))?)
                .unwrap_or(Value::Null)),
            "effects.bindings" => {
                entry.backend.lock().unwrap().local_fields();
                Ok(json!({"search":"effects.search", "scope":"effects.scope"}))
            }
            "effects.registry" => Ok(entry.backend.lock().unwrap().registry()),
            "effects.target" => entry.backend.lock().unwrap().target().map_err(Error::Host),
            "report" => {
                self.report(format!("{panel}: {input}"));
                Ok(Value::Null)
            }
            "platform.activatePanel" | "platform.focus" | "platform.announce" => {
                let mut requests = self.platform.lock().unwrap();
                if requests.len() >= 128 {
                    return Err(Error::Quota("platform queue"));
                }
                requests.push(PlatformRequest {
                    id: 0,
                    panel: panel.into(),
                    generation: generation.number,
                    operation: operation.into(),
                    input: input.clone(),
                });
                (self.wake)();
                Ok(Value::Null)
            }
            // Menus require an asynchronous reply from AppKit. Refuse them until
            // installed; a cancelled menu must never be reported as a choice.
            "platform.effectMenu" | "platform.contextMenu" => {
                Err(Error::Host("native panel menu adapter unavailable".into()))
            }
            _ => Err(Error::Invalid("unknown built-in host operation")),
        }
    }
}
/// Normalize owned panel requests to the existing asynchronous AppKit routes.
fn inspector_platform<'a>(panel: &str, request: &'a Value) -> Result<(&'a str, Value)> {
    let operation = request["operation"]
        .as_str()
        .ok_or(Error::Invalid("platform operation"))?;
    let input = &request["input"];
    if panel == "inspector" && operation == "inspector" {
        match input["type"].as_str() {
            Some("add-effect-menu") => return Ok(("effectMenu", input.clone())),
            Some("font-menu") => return Ok(("fontMenu", input.clone())),
            Some("animator-menu") => return Ok(("contextMenu", input["items"].clone())),
            Some("row-menu") => {
                let items = input["items"]
                    .as_array()
                    .ok_or(Error::Invalid("row menu"))?;
                let mut options = Vec::new();
                for item in items {
                    if item["separatorBefore"] == true {
                        options.push(Value::Null);
                    }
                    options.push(json!({"id":item["action"],"label":item["label"],
                        "disabled":item["enabled"] != true,"checked":item["checked"]}));
                }
                return Ok(("contextMenu", json!(options)));
            }
            _ => {}
        }
    }
    Ok((operation, input.clone()))
}

impl KernelHooks for Broker {
    fn load_storage(&self, _: &str) -> Result<Value> {
        Ok(json!({}))
    }
    fn commit_activation(
        &self,
        previous: Option<&Generation>,
        next: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        if previous.is_some_and(|old| old != next) || !ops.is_empty() || storage != &json!({}) {
            return Err(Error::Invalid(
                "built-in activation cannot publish untyped edits",
            ));
        }
        Ok(())
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        Err(Error::StaleGeneration)
    }
    fn revoke_generation(&self, generation: &Generation) {
        let Ok(panel) = self.panel(generation) else {
            return;
        };
        let Some(entry) = self.entry(panel) else {
            return;
        };
        let mut entry = entry.lock().unwrap();
        if entry.generation != generation.number {
            return;
        }
        let _ = entry.router.retire();
        // A generation that failed (a quota, a crash) takes down what it put
        // up; a closing window's realms just end.
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        entry.overlay = Surface::default();
        drop(entry);
        let panel = panel.to_owned();
        self.forget_overlay(&panel);
        self.forget_timeline(&panel, Some(generation.number));
        self.forget_painter(&panel);
        self.forget_themes(&panel);
        self.forget_commands(&panel);
        (self.wake)();
    }
    fn seal_open_gestures(&self, _: &Generation) {}
}
impl Broker {
    fn entry(&self, panel: &str) -> Option<Arc<Mutex<Entry>>> {
        self.entries.lock().unwrap().get(panel).cloned()
    }
    fn all_entries(&self) -> Vec<Arc<Mutex<Entry>>> {
        self.entries.lock().unwrap().values().cloned().collect()
    }
    /// Queues a `timeline.chrome` edit for this window's chrome writer.
    fn chrome_edit(&self, edit: chrome::ChromeEdit) {
        let mut writer = self.chrome.lock().unwrap();
        let tx = writer
            .get_or_insert_with(|| chrome::spawn_writer(self.kernel.clone(), self.project.clone()));
        let _ = tx.send(edit);
    }
    /// An extension (or one failed generation of it) leaving puts back the
    /// timeline chrome it set.
    fn forget_timeline(&self, panel: &str, generation: Option<u64>) {
        if self.chrome.lock().unwrap().is_some() {
            self.chrome_edit(chrome::ChromeEdit::Forget {
                panel: panel.into(),
                generation,
            });
        }
    }
    fn forget_painter(&self, panel: &str) {
        let mut painters = self.painters.lock().unwrap();
        if painters.1.remove(panel) {
            painters.0 += 1;
        }
    }
    fn forget_overlay(&self, panel: &str) {
        let id = format!("{panel}{VIEWER_OVERLAY}");
        self.publications.lock().unwrap().remove(&id);
        self.overlays_cleared.lock().unwrap().push(id);
    }
    fn forget_themes(&self, panel: &str) {
        let mut themes = self.themes.lock().unwrap();
        if themes.1.remove(panel).is_some() {
            themes.0 += 1;
        }
    }
    fn forget_commands(&self, panel: &str) {
        let mut commands = self.commands.lock().unwrap();
        if commands.1.remove(panel).is_some() {
            commands.0 += 1;
        }
    }
}
impl HostProxy for Broker {
    fn fetch(
        &self,
        _: &Generation,
        _: FetchRequest,
        _: Cancellation,
    ) -> Result<Box<dyn pm_runtime_v8::HostRequest>> {
        Err(Error::Host(
            "built-in network access requires service permission".into(),
        ))
    }
    fn console(&self, _: &Generation, level: &str, message: &str) {
        self.report(format!("{level}: {message}"));
    }
}
impl WatchdogHooks for Broker {
    fn notice(&self, _: &Generation, _: u64) {
        self.report("Built-in panel is taking longer than expected".into());
    }
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        self.report("Built-in panel is still running; waiting for its owner".into());
        WatchdogDecision::Wait
    }
}

#[cfg(test)]
mod tests;

/// A fork's mount script: the fork's module graph drawing built-in `panel`.
fn fork_source(fork: &str, graph: &Value, panel: &str) -> String {
    format!(
        "{BUNDLE}\n__pmBuiltin.mountExtension({},{},{})",
        json!(fork),
        json!(graph.to_string()),
        json!(panel)
    )
}

/// v1–3's `sanitizeInterfaceEdit` for the timeline: known fields, v1's ranges.
fn timeline_patch(input: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let number = |key: &str, low: f64, high: f64, round: bool| {
        input[key]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| if round { v.round() } else { v }.clamp(low, high))
    };
    for (key, low, high, round) in [
        ("rowHeight", 22., 48., true),
        ("gutterWidth", 160., 360., true),
        ("rulerHeight", 20., 42., true),
        ("clipRadius", 0., 12., false),
        ("keyframeSize", 4., 12., false),
    ] {
        if let Some(v) = number(key, low, high, round) {
            out.insert(key.into(), json!(v));
        }
    }
    for key in ["showLayerNumbers", "showTypeBadges"] {
        if let Some(v) = input[key].as_bool() {
            out.insert(key.into(), json!(v));
        }
    }
    for (key, allowed) in [
        ("toolbarDensity", ["compact", "normal"]),
        ("surfaceOrder", ["normal", "reversed"]),
    ] {
        if let Some(v) = input[key].as_str().filter(|v| allowed.contains(v)) {
            out.insert(key.into(), json!(v));
        }
    }
    Value::Object(out)
}
