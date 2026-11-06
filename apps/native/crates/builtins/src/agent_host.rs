//! The Agent panel's process path. Admission precedes Panel mutation, so a
//! missing runtime never consumes a draft or starts a fictitious run. Runs start
//! on the panel worker and a dedicated worker delivers their events durably.
use crate::models::KernelStore;
use pm_agents::{
    Provider,
    account::{Accounts, Opener, State},
    launch::{self, Environment, Request},
    runtime::Runtime,
};
use pm_kernel::KernelHandle;
use pm_panel_agentpanel::{
    Error,
    input::Catalog,
    model::{Effect, EffectKind, Panel},
    runtime::ProviderRuns,
};
use pm_runtime_v8::Realm;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Everything the panel shares with its window, without locking a Backend.
pub(crate) struct AgentLink {
    environment: Mutex<Option<Environment>>,
    accounts: Mutex<Option<Arc<Accounts>>>,
    realms: Mutex<Vec<Realm>>,
    closing: AtomicBool,
    sequence: AtomicU64,
}
impl Default for AgentLink {
    fn default() -> Self {
        Self {
            environment: Mutex::default(),
            accounts: Mutex::default(),
            realms: Mutex::default(),
            closing: AtomicBool::new(false),
            sequence: AtomicU64::new(1),
        }
    }
}
impl AgentLink {
    /// Reads who is signed in on workers; the panel updates when they answer.
    pub fn set_environment(self: &Arc<Self>, environment: Environment, open: Arc<Opener>) {
        let weak = Arc::downgrade(self);
        let accounts = Accounts::new(
            environment.clone(),
            open,
            Arc::new(move || {
                if let Some(link) = weak.upgrade() {
                    link.touch();
                    link.publish();
                }
            }),
        );
        *self.environment.lock().unwrap() = Some(environment);
        *self.accounts.lock().unwrap() = Some(accounts.clone());
        accounts.refresh(Provider::Chatgpt);
        accounts.refresh(Provider::Claude);
    }
    pub fn add_realm(&self, realm: Realm) {
        self.realms.lock().unwrap().push(realm);
    }
    pub fn close(&self) {
        self.realms.lock().unwrap().clear();
        self.closing.store(true, Ordering::Release);
        if let Some(accounts) = self.accounts() {
            accounts.close();
        }
    }
    pub fn accounts(&self) -> Option<Arc<Accounts>> {
        self.accounts.lock().unwrap().clone()
    }
    /// Changes when a sign-in does, so Settings republishes.
    pub fn accounts_revision(&self) -> u64 {
        self.accounts().map_or(0, |accounts| accounts.revision())
    }
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }
    pub fn touch(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
    }
    /// A run needs its runtime and a signed-in account.
    pub fn available(&self, provider: Provider) -> bool {
        self.connection(provider).0 == "connected"
    }
    /// The panel's connection state and what to tell the user about it.
    pub fn connection(&self, provider: Provider) -> (&'static str, String) {
        let name = match provider {
            Provider::Chatgpt => "ChatGPT",
            Provider::Claude => "Claude",
            Provider::Compatible => {
                return (
                    "unavailable",
                    "Connect an API, Ollama, or LM Studio in Settings.".into(),
                );
            }
        };
        let installed = self
            .environment
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|environment| environment.available(provider));
        let accounts = self.accounts().filter(|_| installed);
        let Some(accounts) = accounts else {
            let reason = format!("Powermove's built-in {name} runtime is missing.");
            return ("unavailable", reason);
        };
        let status = accounts.status(provider);
        let signed_out = || format!("Sign in to {name} in Settings.");
        match status.state {
            State::Checking => ("checking", String::new()),
            State::Connecting => ("waiting", String::new()),
            State::Connected => ("connected", String::new()),
            State::Disconnected => ("unavailable", status.detail.unwrap_or_else(signed_out)),
            State::Unavailable => ("unavailable", status.detail.unwrap_or_else(signed_out)),
        }
    }
    /// Providers this install can run, and the choices of the given one.
    pub fn catalog(&self, provider: Provider) -> Catalog {
        let environment = self.environment.lock().unwrap();
        let mut providers = vec![];
        for (candidate, label) in [(Provider::Chatgpt, "ChatGPT"), (Provider::Claude, "Claude")] {
            let runtime = match candidate {
                Provider::Claude => Runtime::Claude,
                _ => Runtime::Codex,
            };
            let installed = environment.as_ref().and_then(|e| e.binary(runtime));
            if installed.is_some() || candidate == provider {
                providers.push((candidate.name().to_owned(), label.to_owned()));
            }
        }
        drop(environment);
        let listed = self
            .accounts()
            .map_or_else(Vec::new, |accounts| accounts.models(provider));
        let mut models = vec![(String::new(), "Default".to_owned())];
        let mut efforts = vec![String::new()];
        for model in listed {
            models.push((model.id, model.label));
            for effort in model.efforts {
                if !efforts.contains(&effort) {
                    efforts.push(effort);
                }
            }
        }
        Catalog {
            models,
            providers,
            efforts,
            claude: provider == Provider::Claude,
        }
    }
    fn publish(&self) {
        for realm in self.realms.lock().unwrap().iter() {
            let _ = realm.eval("__pmBuiltin.publish()");
        }
    }
}
/// The application Agent panel's live runs, kept while a project is open. A run
/// that opens or creates a project keeps going, and the application panel shows
/// it again on return. Dropping this cancels whatever is still running.
pub struct AgentCarry(pub(crate) Option<(Arc<AgentLink>, SharedAgent)>);
impl AgentCarry {
    /// The window giving these up stops publishing to its realms.
    pub(crate) fn take(link: &Arc<AgentLink>, state: &SharedAgent) -> Option<Self> {
        if state.lock().unwrap().runs.active() == 0 {
            return None;
        }
        link.realms.lock().unwrap().clear();
        Some(Self(Some((link.clone(), state.clone()))))
    }
    /// Hand the runs to a new application window; nothing is cancelled.
    pub(crate) fn adopt(mut self) -> (Arc<AgentLink>, SharedAgent) {
        self.0.take().expect("carried once")
    }
}
impl Drop for AgentCarry {
    fn drop(&mut self) {
        if let Some((link, _)) = self.0.take() {
            link.close();
        }
    }
}
pub(crate) struct AgentState {
    pub panel: Panel,
    pub runs: ProviderRuns,
    worker: bool,
}
impl AgentState {
    pub fn new(panel: Panel) -> Self {
        Self {
            panel,
            runs: ProviderRuns::default(),
            worker: false,
        }
    }
}
pub(crate) type SharedAgent = Arc<Mutex<AgentState>>;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
pub fn now_ms() -> u64 {
    now()
}
const UNAVAILABLE: &str = "Agent provider or native action service is unavailable";
/// `live`: the thread has a run, which outlives its account's status.
pub fn admit(action: &Value, connected: bool, live: bool) -> Result<(), String> {
    let allowed = match action["type"].as_str() {
        Some(
            "draft" | "focus" | "newThread" | "switchThread" | "disclose" | "removeAttachment"
            | "setProvider" | "setModel" | "chooseAnswer" | "typeAnswer" | "keepResult"
            | "openSettings",
        ) => true,
        Some("stop") => true,
        Some("answer") => connected || live,
        Some("submit") => connected,
        Some("setAccess") => matches!(
            action["access"].as_str(),
            Some("editor" | "supervised" | "auto")
        ),
        Some("shell") => matches!(action["action"].as_str(), Some("open" | "close")),
        Some("key") => {
            connected
                || action["key"]["key"] != "Enter"
                || action["key"]["composing"] == true
                || action["key"]["shiftKey"] == true
        }
        Some("chooseSlash") => action["id"] == "settings",
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(UNAVAILABLE.into())
    }
}
pub fn sends(action: &Value) -> bool {
    action["type"] == "submit"
        || action["type"] == "key"
            && action["key"]["key"] == "Enter"
            && action["key"]["shiftKey"] != true
            && action["key"]["composing"] != true
}
/// Refuse a send the panel could not hand to a provider, before it is consumed.
pub fn admit_send(state: &AgentState, thread: &str) -> Result<(), String> {
    let session = state.panel.session(thread).map_err(|e| format!("{e:?}"))?;
    if !session.draft.sendable() {
        return Ok(());
    }
    if session.draft.attachments().next().is_some() {
        return Err("Attachments can't reach the agent in this build. Remove them to send.".into());
    }
    match &session.run {
        None if state.runs.active() >= 8 => {
            Err("Eight agents are already running. Wait for one to finish.".into())
        }
        Some(run) if !state.runs.steerable(&run.id) && !session.needs_answer() => {
            Err("This run can't take new messages. Stop it first.".into())
        }
        _ => Ok(()),
    }
}
fn workspace(profile: &std::path::Path, project: &str, thread: &str) -> std::io::Result<PathBuf> {
    use pm_agents::workspace::safe_component;
    let path = profile
        .join("Agent State")
        .join(safe_component(project))
        .join("threads")
        .join(safe_component(thread))
        .join("workspace");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
/// A run carries bounded thread history, so it needs no provider session.
fn prompt(panel: &Panel, thread: &str, text: &str) -> Result<String, Error> {
    let session = panel.session(thread)?;
    let mut messages = session.conversation.values().cloned().collect::<Vec<_>>();
    messages.pop();
    Ok(if messages.is_empty() {
        text.to_owned()
    } else {
        format!(
            "THREAD CONTEXT\n{}\n\nUSER REQUEST\n{text}",
            pm_agents::conversation::context(&messages, thread)
        )
    })
}
/// Run a panel effect that reaches a provider. Anything that keeps it from the
/// provider settles in the conversation: a start fails its run, a Stop or an
/// answer is acknowledged as refused. Only an unsettled refusal is an error.
pub fn run_effect(
    shared: &SharedAgent,
    link: &Arc<AgentLink>,
    kernel: &KernelHandle,
    application: bool,
    effect: &Effect,
) -> Result<(), String> {
    let mut store = KernelStore::new(kernel.clone(), application);
    let mut guard = shared.lock().unwrap();
    let state = &mut *guard;
    let result = match &effect.kind {
        EffectKind::Send {
            text,
            settings,
            steer: false,
            ..
        } => {
            let environment = link.environment.lock().unwrap().clone();
            launch_request(state, environment, effect, text, settings).and_then(
                |(environment, request)| {
                    state
                        .runs
                        .start(&mut state.panel, &mut store, effect, now(), || {
                            launch::launch(&environment, request).map_err(Error::Host)
                        })
                },
            )
        }
        _ => state
            .runs
            .dispatch(&mut state.panel, &mut store, effect, now()),
    };
    let refused = result.err().and_then(|error| {
        state
            .runs
            .refuse(&mut state.panel, &mut store, effect, now(), &error)
            .err()
            .map(|_| error.to_string())
    });
    drop(guard);
    ensure_worker(shared, link, kernel, application);
    refused.map_or(Ok(()), Err)
}
fn launch_request(
    state: &AgentState,
    environment: Option<Environment>,
    effect: &Effect,
    text: &str,
    settings: &pm_panel_agentpanel::model::Settings,
) -> Result<(Environment, Request), Error> {
    let environment = environment.ok_or_else(|| Error::Host(UNAVAILABLE.into()))?;
    let thread = &effect.target.thread;
    let directory = workspace(&environment.profile, &effect.target.project, thread)
        .map_err(|e| Error::Host(format!("The agent's workspace could not be created: {e}")))?;
    let request = Request {
        run: effect.target.run.clone().ok_or(Error::Stale)?,
        prompt: prompt(&state.panel, thread, text)?,
        provider: settings.provider,
        model: Some(settings.model.clone()).filter(|m| !m.is_empty()),
        effort: Some(settings.effort.clone()).filter(|e| !e.is_empty()),
        access: settings.access,
        approval: settings.approval,
        workspace: directory,
    };
    Ok((environment, request))
}
/// Start the delivery worker if runs or deferred saves need one.
pub(crate) fn ensure_worker(
    shared: &SharedAgent,
    link: &Arc<AgentLink>,
    kernel: &KernelHandle,
    application: bool,
) {
    let mut state = shared.lock().unwrap();
    if state.worker || state.runs.is_empty() && !state.panel.unsaved() {
        return;
    }
    let (worker, link, kernel) = (shared.clone(), link.clone(), kernel.clone());
    state.worker = std::thread::Builder::new()
        .name("pm-agent-runs".into())
        .spawn(move || deliver(worker, link, kernel, application))
        .is_ok();
}
/// Durable delivery, one bounded batch at a time. A failed publication keeps the
/// exact event for retry. Streamed text and typing are saved once they are a
/// checkpoint old. Window close cancels what is left, saves, then gives up.
fn deliver(shared: SharedAgent, link: Arc<AgentLink>, kernel: KernelHandle, application: bool) {
    let mut store = KernelStore::new(kernel, application);
    let mut closed: Option<Instant> = None;
    loop {
        let (changed, failed, done) = {
            let mut guard = shared.lock().unwrap();
            let state = &mut *guard;
            if link.closing.load(Ordering::Acquire) && closed.is_none() {
                closed = Some(Instant::now());
                state.runs.cancel_all();
            }
            if closed.is_some_and(|at| at.elapsed() > Duration::from_secs(5)) {
                state.runs = ProviderRuns::default();
            }
            let saved = state.panel.save_error.clone();
            let polled = state.runs.poll(&mut state.panel, &mut store, now(), 64);
            let mut failed = polled.is_err();
            if state.runs.is_empty() && state.panel.unsaved() && closed.is_some() {
                failed |= state.panel.save(&mut store, now()).is_err();
            }
            let done = state.runs.is_empty() && !state.panel.unsaved();
            if done {
                state.worker = false;
            }
            let changed = *polled.as_ref().unwrap_or(&0) > 0
                || failed
                || saved != state.panel.save_error;
            (changed, failed, done)
        };
        if changed {
            link.touch();
            link.publish();
        }
        if done {
            return;
        }
        std::thread::sleep(Duration::from_millis(if failed { 250 } else { 16 }));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn absent_provider_rejects_submission_but_still_reaches_a_live_run() {
        for action in [
            json!({"type":"submit"}),
            json!({"type":"answer"}),
            json!({"type":"setAccess","access":"full"}),
            json!({"type":"key","key":{"key":"Enter"}}),
        ] {
            assert!(admit(&action, false, false).is_err());
        }
        for action in [
            json!({"type":"stop"}),
            json!({"type":"draft"}),
            json!({"type":"openSettings"}),
            json!({"type":"key","key":{"key":"Enter","composing":true}}),
        ] {
            assert!(admit(&action, false, false).is_ok());
        }
    }
    #[test]
    fn a_connected_provider_admits_runs_but_never_computer_access() {
        for action in [
            json!({"type":"submit"}),
            json!({"type":"answer"}),
            json!({"type":"stop"}),
            json!({"type":"key","key":{"key":"Enter"}}),
        ] {
            assert!(admit(&action, true, false).is_ok());
        }
        assert!(admit(&json!({"type":"setAccess","access":"full"}), true, false).is_err());
        // Signed out mid-run: the run still takes its answers.
        assert!(admit(&json!({"type":"answer"}), false, true).is_ok());
    }
}
