//! Provider sign-in. Each runtime owns its credentials inside its isolated home;
//! Powermove only starts its login and reads who is connected. Every process runs
//! on an account worker, so reading a status never waits.
use crate::{
    Provider,
    launch::{Environment, is_valid_model_id},
    runtime::Runtime,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

const REQUEST: Duration = Duration::from_secs(15);
const MODELS: Duration = Duration::from_secs(30);
const LOGIN: Duration = Duration::from_secs(600);
const DETAIL_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Checking,
    /// Waiting for the user to finish in their browser.
    Connecting,
    Connected,
    Disconnected,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub detail: Option<String>,
    /// Codex signed in with an OpenAI API key (its `account/read` says
    /// `apiKey`). Codex never hands out the key, so none is shown.
    pub api_key: bool,
}
impl Status {
    fn of(state: State, detail: Option<String>) -> Self {
        Self {
            state,
            email: None,
            plan: None,
            detail: detail.map(|d| d.chars().take(DETAIL_LIMIT).collect()),
            api_key: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    /// The model the provider runs when none is chosen.
    pub default: bool,
    /// Other ids the provider accepts for this model (Claude's resolved
    /// `claude-opus-5-5` for `opus`), so a saved one still reads as its name.
    pub aliases: Vec<String>,
    /// The reasoning effort the provider runs this model at when none is
    /// chosen, when it says (Codex does; Claude Code does not).
    pub default_effort: Option<String>,
}
struct Slot {
    status: Status,
    models: Vec<Model>,
    finding_models: bool,
    worker: bool,
    /// What runs when the current worker finishes. A sign-out outranks a sign-in.
    next: Option<Job>,
    cancel: Arc<AtomicBool>,
}
pub type Opener = dyn Fn(&str) -> Result<(), String> + Send + Sync;
/// The browser hop, a parameter because it is the one step a test must not take.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    // A development or lane build never opens the person's browser (a
    // provider's sign-in page included): it says what it would have opened.
    if cfg!(debug_assertions) || std::env::var_os("PM_NATIVE_NO_ORIGINAL").is_some() {
        eprintln!("Powermove (development build) would open: {url}");
        return Ok(());
    }
    Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| "The browser could not be opened.".to_owned())
}
pub struct Accounts {
    environment: Environment,
    open: Arc<Opener>,
    changed: Arc<dyn Fn() + Send + Sync>,
    slots: Mutex<BTreeMap<&'static str, Slot>>,
    revision: AtomicU64,
    closed: AtomicBool,
    keys: Mutex<Keys>,
    /// Aero on the Powermove account, and the refresh that last asked.
    aero: Mutex<(crate::inference::Standing, u64)>,
}
/// API keys from Settings › Providers, for Claude Code and Codex.
#[derive(Default)]
struct Keys {
    anthropic: KeyState,
    /// A key waiting for Codex to save it (`connect_openai_key`).
    pending_openai: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    /// The saved key, masked.
    pub masked: Option<String>,
    pub models: Vec<Model>,
    pub detail: Option<String>,
    pub checking: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Job {
    Refresh,
    Connect,
    Disconnect,
    /// Sign Codex in with the pending OpenAI API key.
    ConnectKey,
}
impl Accounts {
    pub fn new(
        environment: Environment,
        open: Arc<Opener>,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        let slot = || Slot {
            status: Status::of(State::Checking, None),
            models: vec![],
            finding_models: false,
            worker: false,
            next: None,
            cancel: Arc::default(),
        };
        Arc::new(Self {
            environment,
            open,
            changed,
            slots: Mutex::new(BTreeMap::from([("chatgpt", slot()), ("claude", slot())])),
            revision: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            keys: Mutex::default(),
            aero: Mutex::default(),
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn status(&self, provider: Provider) -> Status {
        self.slots.lock().unwrap().get(provider.name()).map_or_else(
            || Status::of(State::Disconnected, None),
            |s| s.status.clone(),
        )
    }
    pub fn models(&self, provider: Provider) -> Vec<Model> {
        self.slots
            .lock()
            .unwrap()
            .get(provider.name())
            .map_or_else(Vec::new, |s| s.models.clone())
    }
    /// True during the model request, not while waiting for sign-in.
    pub fn finding_models(&self, provider: Provider) -> bool {
        self.slots
            .lock()
            .unwrap()
            .get(provider.name())
            .is_some_and(|s| s.finding_models)
    }
    /// Reads who is signed in again (Try Again): for Aero, its plan, credits
    /// and catalogue.
    pub fn refresh(self: &Arc<Self>, provider: Provider) {
        if provider == Provider::Aero {
            return self.refresh_aero();
        }
        self.start(provider, Job::Refresh);
    }
    pub fn connect(self: &Arc<Self>, provider: Provider) {
        self.start(provider, Job::Connect);
    }
    /// OpenAI's second way in: Codex saves `key` in the shared Keychain item.
    pub fn connect_openai_key(self: &Arc<Self>, key: String) {
        self.keys.lock().unwrap().pending_openai = Some(key);
        self.start(Provider::Chatgpt, Job::ConnectKey);
    }
    pub fn anthropic(&self) -> KeyState {
        self.keys.lock().unwrap().anthropic.clone()
    }
    /// Saves the Anthropic API key in the Keychain and lists its models.
    pub fn set_anthropic_key(self: &Arc<Self>, key: String) -> Result<(), String> {
        let key = key.trim().to_owned();
        if !crate::keys::valid_key(&key) {
            return Err("That doesn’t look like an API key.".into());
        }
        let secrets = self
            .environment
            .secrets
            .clone()
            .ok_or("The Keychain is unavailable.")?;
        secrets.write(
            &crate::keys::anthropic_service(&self.environment),
            "api-key",
            key.as_bytes(),
        )?;
        self.refresh_anthropic();
        Ok(())
    }
    /// Removes the key on a worker; the row follows when it is gone.
    pub fn remove_anthropic_key(self: &Arc<Self>) -> Result<(), String> {
        let secrets = self.environment.secrets.clone().ok_or("The Keychain is unavailable.")?;
        let this = self.clone();
        std::thread::spawn(move || {
            let service = crate::keys::anthropic_service(&this.environment);
            match secrets.delete(&service, "api-key") {
                Ok(()) => this.keys.lock().unwrap().anthropic = KeyState::default(),
                Err(error) => this.keys.lock().unwrap().anthropic.detail = Some(error),
            }
            this.bump();
        });
        Ok(())
    }
    /// Reads the saved Anthropic key and lists its models, off this thread.
    /// Switch off: nothing is read. The Keychain is read on a worker, never
    /// on the calling thread.
    pub fn refresh_anthropic(self: &Arc<Self>) {
        if !self.environment.providers() {
            return;
        }
        let (Some(secrets), Some(net)) = (self.environment.secrets.clone(), self.environment.net.clone()) else {
            return;
        };
        let this = self.clone();
        std::thread::spawn(move || {
            let Some(key) = crate::keys::anthropic_key(secrets.as_ref(), &this.environment) else {
                this.keys.lock().unwrap().anthropic = KeyState::default();
                return this.bump();
            };
            {
                let mut keys = this.keys.lock().unwrap();
                keys.anthropic.masked = Some(crate::keys::mask(&key));
                keys.anthropic.checking = true;
            }
            this.bump();
            let listed = crate::keys::anthropic_models(net.as_ref(), &key);
            {
                let mut keys = this.keys.lock().unwrap();
                keys.anthropic.checking = false;
                match listed {
                    Ok(models) => {
                        keys.anthropic.models = models;
                        keys.anthropic.detail = None;
                    }
                    Err(detail) => keys.anthropic.detail = Some(detail),
                }
            }
            this.bump();
        });
    }
    /// Where Aero stands: signed out, on Free, or its models and credits.
    pub fn aero(&self) -> crate::inference::Standing {
        self.aero.lock().unwrap().0.clone()
    }
    /// Reads Aero's plan, credits and models from the Powermove account on a
    /// worker (the account changed, a turn spent credits). Switch off:
    /// nothing is read. The newest refresh's answer wins.
    pub fn refresh_aero(self: &Arc<Self>) {
        if !self.environment.providers() {
            return;
        }
        let asked = {
            let mut aero = self.aero.lock().unwrap();
            aero.1 += 1;
            aero.1
        };
        let this = self.clone();
        std::thread::spawn(move || {
            let standing = crate::inference::standing(&this.environment);
            let mut aero = this.aero.lock().unwrap();
            if aero.1 == asked && aero.0 != standing {
                aero.0 = standing;
                drop(aero);
                this.bump();
            }
        });
    }
    /// Sign-out wins over a login still waiting on the browser.
    pub fn disconnect(self: &Arc<Self>, provider: Provider) {
        if let Some(slot) = self.slots.lock().unwrap().get(provider.name()) {
            slot.cancel.store(true, Ordering::Release);
        }
        self.start(provider, Job::Disconnect);
    }
    /// Stops waiting logins. Workers exit on their own.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        for slot in self.slots.lock().unwrap().values() {
            slot.cancel.store(true, Ordering::Release);
        }
    }
    fn start(self: &Arc<Self>, provider: Provider, job: Job) {
        let name = provider.name();
        let cancel = {
            let mut slots = self.slots.lock().unwrap();
            let Some(slot) = slots.get_mut(name) else {
                return;
            };
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            if slot.worker {
                // Runs once the current worker finishes; a refresh then is redundant.
                if job != Job::Refresh && slot.next != Some(Job::Disconnect) {
                    slot.next = Some(job);
                }
                return;
            }
            slot.worker = true;
            slot.cancel = Arc::default();
            slot.cancel.clone()
        };
        if matches!(job, Job::Connect | Job::ConnectKey) {
            self.publish(provider, Status::of(State::Checking, None));
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("pm-agent-account".into())
            .spawn(move || {
                let status = this.run(provider, job, &cancel);
                let next = {
                    let mut slots = this.slots.lock().unwrap();
                    let slot = slots.get_mut(provider.name()).unwrap();
                    slot.status = status;
                    slot.finding_models = false;
                    slot.worker = false;
                    slot.next.take()
                };
                this.bump();
                if let Some(next) = next {
                    this.start(provider, next);
                }
            });
        if let Err(error) = spawned {
            self.slots.lock().unwrap().get_mut(name).unwrap().worker = false;
            self.publish(
                provider,
                Status::of(State::Unavailable, Some(error.to_string())),
            );
        }
    }
    fn run(&self, provider: Provider, job: Job, cancel: &AtomicBool) -> Status {
        let result = match provider {
            Provider::Chatgpt => self.chatgpt(job, cancel),
            Provider::Claude => self.claude(job, cancel, MODELS),
            Provider::Compatible | Provider::Aero => Ok(Status::of(State::Disconnected, None)),
        };
        result.unwrap_or_else(|detail| Status::of(State::Unavailable, Some(detail)))
    }
    fn publish(&self, provider: Provider, status: Status) {
        if let Some(slot) = self.slots.lock().unwrap().get_mut(provider.name()) {
            slot.status = status;
        }
        self.bump();
    }
    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        (self.changed)();
    }
    fn set_models(&self, provider: Provider, models: Vec<Model>) {
        if let Some(slot) = self.slots.lock().unwrap().get_mut(provider.name()) {
            slot.models = models;
            slot.finding_models = false;
        }
        self.bump();
    }
    fn ensure_runtime(
        &self,
        provider: Provider,
        runtime: Runtime,
        job: Job,
        cancel: &AtomicBool,
    ) -> Result<PathBuf, Status> {
        if let Some(binary) = self.environment.binary(runtime) {
            return Ok(binary);
        }
        match job {
            Job::Refresh => return Err(Status::of(State::Disconnected, None)),
            Job::Disconnect => {
                self.set_models(provider, vec![]);
                return Err(Status::of(State::Disconnected, None));
            }
            Job::Connect | Job::ConnectKey => {}
        }
        let label = match runtime {
            Runtime::Codex => "ChatGPT",
            Runtime::Claude => "Claude",
        };
        if cancel.load(Ordering::Acquire) {
            return Err(Status::of(
                State::Disconnected,
                Some("Sign-in was cancelled.".into()),
            ));
        }
        self.publish(
            provider,
            Status::of(
                State::Connecting,
                Some(format!("Downloading {label} runtime…")),
            ),
        );
        let root = self.environment.runtimes_root();
        let pin = crate::runtime::bundled_pin(runtime, crate::runtime::current_arch());
        let mut registry = crate::runtime::NpmRegistry;
        let mut probe = crate::runtime::NativeProbe;
        match crate::runtime::install(
            &root,
            &self.environment.app_version,
            &pin,
            &mut registry,
            &mut probe,
        ) {
            Ok(binary) => {
                if !crate::launch::grants::any_live() {
                    let _ = crate::runtime::prune_inactive(&root, runtime);
                }
                if cancel.load(Ordering::Acquire) {
                    return Err(Status::of(
                        State::Disconnected,
                        Some("Sign-in was cancelled.".into()),
                    ));
                }
                Ok(binary)
            }
            Err(error) => Err(Status::of(
                State::Disconnected,
                Some(format!(
                    "Couldn't download the {label} runtime ({error}). Check your connection and try again."
                )),
            )),
        }
    }
    fn chatgpt(&self, job: Job, cancel: &AtomicBool) -> Result<Status, String> {
        let binary = match self.ensure_runtime(Provider::Chatgpt, Runtime::Codex, job, cancel) {
            Ok(binary) => binary,
            Err(status) => return Ok(status),
        };
        let mut peer = Peer::start(
            &binary,
            &["app-server"],
            self.environment.codex_vars()?,
        )?;
        peer.request(
            "initialize",
            json!({"clientInfo":{"name":"powermove","title":"Powermove","version":self.environment.app_version},
                "capabilities":{"experimentalApi":true}}),
            REQUEST,
        )?;
        peer.notify("initialized", json!({}))?;
        match job {
            Job::Refresh => {}
            Job::Disconnect => {
                peer.request("account/logout", json!({}), REQUEST)?;
                // Signed out only once Codex itself says so (P1-4): live runs
                // on the old sign-in stop when the identity generation moves.
                let mut status = chatgpt_status(&mut peer)?;
                if status.state != State::Disconnected {
                    status.detail =
                        Some("Codex couldn’t remove the sign-in. Try Sign Out again.".into());
                    return Ok(status);
                }
                if self.environment.providers() {
                    crate::codex_auth::identity_changed();
                }
                self.set_models(Provider::Chatgpt, vec![]);
                return Ok(Status::of(State::Disconnected, None));
            }
            Job::ConnectKey => {
                let Some(key) = self.keys.lock().unwrap().pending_openai.take() else {
                    return chatgpt_status(&mut peer);
                };
                if !crate::keys::valid_key(key.trim()) {
                    return Ok(Status::of(State::Disconnected, Some("That doesn’t look like an API key.".into())));
                }
                peer.request("account/login/start", json!({"type":"apiKey","apiKey":key.trim()}), REQUEST)?;
            }
            Job::Connect => {
                let status = chatgpt_status(&mut peer)?;
                if status.state != State::Disconnected {
                    return Ok(status);
                }
                let started = peer.request(
                    "account/login/start",
                    json!({"type":"chatgpt","useHostedLoginSuccessPage":true,"appBrand":"chatgpt"}),
                    REQUEST,
                )?;
                let (Some(login), Some(url)) =
                    (started["loginId"].as_str(), started["authUrl"].as_str())
                else {
                    return Ok(Status::of(
                        State::Disconnected,
                        Some("Codex did not return a ChatGPT sign-in URL.".into()),
                    ));
                };
                // A development build against the local fakes signs in on a
                // loopback issuer; everything else only on OpenAI's own.
                let fake = cfg!(debug_assertions)
                    && self.environment.vars.get("PM_AGENT_FAKE_BROWSER").is_some_and(|v| v == "1")
                    && url.starts_with("http://127.0.0.1:");
                let url = if fake { url.to_owned() } else { trusted_auth_url(url)? };
                (self.open)(&url)?;
                self.publish(
                    Provider::Chatgpt,
                    Status::of(
                        State::Connecting,
                        Some("Finish signing in in your browser.".into()),
                    ),
                );
                if let Some(failure) = wait_for_login(&mut peer, login, cancel)? {
                    return Ok(Status::of(State::Disconnected, Some(failure)));
                }
            }
        }
        let mut status = chatgpt_status(&mut peer)?;
        // A new sign-in or key is a new identity: turns on the old one end.
        if matches!(job, Job::Connect | Job::ConnectKey)
            && status.state == State::Connected
            && self.environment.providers()
        {
            crate::codex_auth::identity_changed();
        }
        if status.state == State::Connected {
            self.slots.lock().unwrap().get_mut(Provider::Chatgpt.name()).unwrap().finding_models = true;
            self.bump();
            match chatgpt_models(&mut peer) {
                Ok(models) => self.set_models(Provider::Chatgpt, models),
                Err(_) => {
                    status.detail = Some(if self.models(Provider::Chatgpt).is_empty() {
                        "Couldn't load ChatGPT models. The provider's default is still available.".into()
                    } else {
                        "Couldn't refresh ChatGPT models. Showing the last known models.".into()
                    });
                }
            }
        }
        Ok(status)
    }
    fn claude(
        &self,
        job: Job,
        cancel: &AtomicBool,
        discovery_timeout: Duration,
    ) -> Result<Status, String> {
        let binary = match self.ensure_runtime(Provider::Claude, Runtime::Claude, job, cancel) {
            Ok(binary) => binary,
            Err(status) => return Ok(status),
        };
        let vars = self.environment.claude_vars()?;
        if job == Job::Disconnect {
            let (code, output) = run(&binary, &["auth", "logout"], &vars, REQUEST, cancel)?;
            if code != Some(0) {
                // Still signed in: say so rather than show a sign-out that didn't happen.
                let mut status = claude_status(&binary, &vars, cancel)?;
                let output = output.trim();
                status.detail = Some(if output.is_empty() {
                    "Claude Code couldn't sign out.".to_owned()
                } else {
                    format!("Claude Code couldn't sign out: {output}")
                });
                return Ok(status);
            }
            self.set_models(Provider::Claude, vec![]);
            return Ok(Status::of(State::Disconnected, None));
        }
        let mut status = claude_status(&binary, &vars, cancel)?;
        if job == Job::Connect && status.state == State::Disconnected {
            self.publish(
                Provider::Claude,
                Status::of(
                    State::Connecting,
                    Some("Finish signing in in the browser opened by Claude Code.".into()),
                ),
            );
            let login = ["auth", "login", "--claudeai"];
            let (code, output) = match run(&binary, &login, &vars, LOGIN, cancel) {
                Ok(finished) => finished,
                Err(detail) => return Ok(Status::of(State::Disconnected, Some(detail))),
            };
            if code != Some(0) {
                let detail = output.trim();
                return Ok(Status::of(
                    State::Disconnected,
                    Some(if detail.is_empty() {
                        "Claude sign-in was not completed.".to_owned()
                    } else {
                        detail.to_owned()
                    }),
                ));
            }
            status = claude_status(&binary, &vars, cancel)?;
        }
        if status.state == State::Connected {
            self.slots
                .lock()
                .unwrap()
                .get_mut(Provider::Claude.name())
                .unwrap()
                .finding_models = true;
            self.bump();
            match claude_models(&binary, vars, discovery_timeout) {
                Ok(models) => self.set_models(Provider::Claude, models),
                Err(detail) => {
                    let slots = self.slots.lock().unwrap();
                    status.detail = Some(if slots[Provider::Claude.name()].models.is_empty() {
                        detail
                    } else {
                        format!("{detail} Showing the last known models.")
                    });
                }
            }
        }
        Ok(status)
    }
}
fn text(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= limit)
        .map(str::to_owned)
}
fn trusted_auth_url(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|e| e.to_string())?;
    let host = url.host_str().unwrap_or_default().to_lowercase();
    let trusted = ["chatgpt.com", "openai.com"]
        .iter()
        .any(|root| host == *root || host.ends_with(&format!(".{root}")));
    if url.scheme() == "https" && trusted && url.username().is_empty() && url.password().is_none() {
        Ok(url.to_string())
    } else {
        Err("Codex returned an unsafe ChatGPT sign-in URL.".into())
    }
}
fn chatgpt_status(peer: &mut Peer) -> Result<Status, String> {
    let read = peer.request("account/read", json!({"refreshToken":false}), REQUEST)?;
    let account = &read["account"];
    if account["type"] == "apiKey" {
        // Codex signed in with an OpenAI API key (the second way in under Settings › Providers).
        return Ok(Status {
            api_key: true,
            ..Status::of(State::Connected, None)
        });
    }
    if account["type"] != "chatgpt" {
        return Ok(Status::of(State::Disconnected, None));
    }
    Ok(Status {
        state: State::Connected,
        email: text(&account["email"], 320),
        plan: text(&account["planType"], 80).map(|p| plan_name(&p)),
        detail: None,
        api_key: false,
    })
}
fn chatgpt_models(peer: &mut Peer) -> Result<Vec<Model>, String> {
    let mut models = Vec::<Model>::new();
    let mut seen = vec![];
    let mut cursor = None::<String>;
    for _ in 0..5 {
        let mut params = json!({"limit":100,"includeHidden":false});
        if let Some(cursor) = &cursor {
            params["cursor"] = json!(cursor);
        }
        let page = peer.request("model/list", params, REQUEST)?;
        let entries = page["data"].as_array().ok_or("Codex returned an invalid model list.")?;
        for entry in entries {
            let id = entry["model"].as_str().or(entry["id"].as_str());
            let Some(id) = id.filter(|id| is_valid_model_id(id)) else {
                continue;
            };
            if entry["hidden"] == true || models.iter().any(|m| m.id == id) {
                continue;
            }
            let name = text(&entry["displayName"], 120);
            models.push(Model {
                id: id.into(),
                label: codex_model_label(name.as_deref().unwrap_or(id)),
                efforts: entry["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| text(&e["reasoningEffort"], 16))
                    .collect(),
                default: entry["isDefault"] == true,
                aliases: vec![],
                default_effort: text(&entry["defaultReasoningEffort"], 16),
            });
        }
        if page["nextCursor"].is_null() {
            return if models.is_empty() { Err("Codex returned no usable models.".into()) } else { Ok(models) };
        }
        let next = text(&page["nextCursor"], 500)
            .filter(|c| !seen.contains(c))
            .ok_or("Codex returned an invalid model list cursor.")?;
        cursor = Some(next.clone());
        seen.push(next);
    }
    Err("Codex returned too many model list pages.".into())
}

/// A display name for GPT model identifiers; preserves other model names.
pub fn codex_model_label(label: &str) -> String {
    let Some((_, rest)) = label.split_once('-').filter(|(family, _)| family.eq_ignore_ascii_case("gpt")) else {
        return label.to_owned();
    };
    let (version, tier) = rest.split_once(['-', ' ']).unwrap_or((rest, ""));
    if !version.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) {
        return label.to_owned();
    }
    let tier = tier.trim_start();
    if let Some(first) = tier.chars().next() {
        format!("GPT-{version} {}{}", first.to_ascii_uppercase(), &tier[first.len_utf8()..])
    } else {
        format!("GPT-{version}")
    }
}
/// `None` once the login finished; otherwise why it didn't.
fn wait_for_login(
    peer: &mut Peer,
    login: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>, String> {
    let deadline = Instant::now() + LOGIN;
    let outcome = loop {
        if cancel.load(Ordering::Acquire) {
            break Some("Sign-in was cancelled.".to_owned());
        }
        if Instant::now() > deadline {
            break Some("ChatGPT sign-in timed out. Try connecting again.".to_owned());
        }
        match peer.lines.recv_timeout(Duration::from_millis(100)) {
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("The ChatGPT runtime stopped during sign-in.".into());
            }
            Ok(line) => {
                let message = serde_json::from_str::<Value>(&line).unwrap_or_default();
                if message["method"] == "account/updated" {
                    break None;
                }
                if message["method"] == "account/login/completed"
                    && message["params"]["loginId"] == login
                {
                    break (message["params"]["success"] == false).then(|| {
                        text(&message["params"]["error"], DETAIL_LIMIT)
                            .unwrap_or_else(|| "ChatGPT sign-in was not completed.".to_owned())
                    });
                }
            }
        }
    };
    if outcome.is_some() {
        let _ = peer.request("account/login/cancel", json!({"loginId":login}), REQUEST);
    }
    Ok(outcome)
}
/// A provider's plan id as Settings shows it: "plus" → "Plus", "max" →
/// "Max", "team_pro" → "Team Pro".
pub fn plan_name(id: &str) -> String {
    id.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
/// Claude Code's own report of its sign-in.
pub(crate) fn claude_status_of(binary: &PathBuf, vars: &BTreeMap<String, String>) -> Result<Status, String> {
    claude_status(binary, vars, &AtomicBool::new(false))
}
/// Claude Code may run on Settings › Providers' Anthropic key only when its
/// own status says plainly that it is signed out: a timeout, an error, an
/// unreadable answer or "checking" never moves a signed-in person onto a
/// metered key. The launch and the picker use this one condition.
pub fn claude_key_fallback(status: &Result<Status, String>) -> bool {
    matches!(status, Ok(status) if status.state == State::Disconnected)
}
fn claude_status(
    binary: &PathBuf,
    vars: &BTreeMap<String, String>,
    cancel: &AtomicBool,
) -> Result<Status, String> {
    let (code, output) = run(binary, &["auth", "status"], vars, REQUEST, cancel)?;
    if matches!(code, Some(126 | 127)) {
        return Err("Claude could not be started. Check its installation and try again.".into());
    }
    let value = serde_json::from_str::<Value>(output.trim())
        .map_err(|_| "Claude Code did not return an account status.")?;
    match value["loggedIn"].as_bool() {
        Some(false) => return Ok(Status::of(State::Disconnected, None)),
        Some(true) => {}
        None => return Err("Claude Code did not return an account status.".into()),
    }
    Ok(Status {
        state: State::Connected,
        email: text(&value["email"], 320),
        // The plan in words; how it signed in (`authMethod`, e.g. a token) is
        // no plan and never shown.
        plan: text(&value["subscriptionType"], 80).map(|p| plan_name(&p)),
        detail: None,
        api_key: false,
    })
}
/// Claude Code answers an initialize control request without a user prompt.
fn claude_models(
    binary: &PathBuf,
    vars: BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Vec<Model>, String> {
    let mut peer = Peer::start(
        binary,
        &[
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--setting-sources",
            "",
        ],
        vars,
    )?;
    peer.write(&json!({"type":"control_request","request_id":"models",
        "request":{"subtype":"initialize"}}))?;
    let timed_out = "Claude model discovery timed out.";
    let deadline = Instant::now() + timeout;
    loop {
        let wait = deadline
            .checked_duration_since(Instant::now())
            .ok_or(timed_out)?;
        let line = peer.lines.recv_timeout(wait).map_err(|error| match error {
            RecvTimeoutError::Timeout => timed_out,
            RecvTimeoutError::Disconnected => "Claude Code exited before returning models.",
        })?;
        let message = serde_json::from_str::<Value>(&line).unwrap_or_default();
        if message["type"] != "control_response" || message["response"]["request_id"] != "models" {
            continue;
        }
        let reply = &message["response"];
        if reply["subtype"] != "success" {
            return Err("Claude Code rejected the model discovery request.".into());
        }
        let list = reply["response"]["models"]
            .as_array()
            .ok_or("Claude Code's model discovery reply is missing the model list.")?;
        let models = claude_listing(list.iter());
        if models.is_empty() {
            return Err("Claude Code returned no usable models.".into());
        }
        return Ok(models);
    }
}
/// The family and version a Claude model id names: `claude-opus-5-5` →
/// ("opus", "5.5"), `claude-opus-5-5[1m]` the same, `claude-3-5-sonnet-20241022`
/// → ("sonnet", "3.5"). A dated snapshot suffix is no part of the version.
fn claude_version(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("claude-")?;
    let rest = rest.split('[').next().unwrap_or(rest);
    let parts: Vec<&str> = rest.split('-').collect();
    let family = parts
        .iter()
        .find(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic()))?;
    let version: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| (1..=2).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()))
        .collect();
    (!version.is_empty()).then(|| (family.to_ascii_lowercase(), version.join(".")))
}
/// A display name with its version after the family word ("Opus (1M context)"
/// → "Opus 5.5 (1M context)"); a name that is not the family's, or already
/// carries the version, stays as given.
fn versioned(name: &str, family: &str, version: &str) -> String {
    if let Some((first, suffix)) = name.split_once('-')
        && first.eq_ignore_ascii_case(family)
        && let Some(tail) = suffix.strip_prefix(version)
        && (tail.is_empty() || tail.starts_with([' ', '(']))
    {
        return format!("{first} {version}{tail}");
    }
    if name.contains(version) {
        return name.to_owned();
    }
    let first = name.split_whitespace().next().unwrap_or_default();
    if !first.eq_ignore_ascii_case(family) {
        return name.to_owned();
    }
    format!("{first} {version}{}", &name[first.len()..])
}
/// Claude Code's model list as the pickers show it: one row per model under
/// its own name. Claude's own "Default (recommended)" row is no model; the
/// model it resolves to is marked as the default instead.
fn claude_listing<'a>(entries: impl Iterator<Item = &'a Value>) -> Vec<Model> {
    let entries: Vec<&Value> = entries.collect();
    let valid = |v: &Value| {
        v.as_str()
            .filter(|id| is_valid_model_id(id))
            .map(str::to_owned)
    };
    let default = entries
        .iter()
        .find(|e| e["value"] == "default")
        .and_then(|e| valid(&e["resolvedModel"]));
    let mut models = Vec::<Model>::new();
    for entry in entries {
        let Some(id) = valid(&entry["value"]) else {
            continue;
        };
        let resolved = valid(&entry["resolvedModel"]).filter(|r| *r != id);
        if id == "default" {
            continue;
        }
        if models.iter().any(|m| m.id == id || m.aliases.contains(&id)) {
            continue;
        }
        let efforts = entry["supportedEffortLevels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| text(e, 16))
            .collect::<Vec<_>>();
        // Two names for one resolved model: the first listed keeps it.
        let resolved = resolved.filter(|r| !models.iter().any(|m| m.aliases.contains(r)));
        let is_default = default
            .as_ref()
            .is_some_and(|d| *d == id || resolved.as_ref() == Some(d))
            && !models.iter().any(|m| m.default);
        // Claude Code names its aliases without a version ("Opus"); the
        // picker shows the one it runs, read from the id the alias resolves
        // to (`claude-opus-5-5` → "Opus 5.5"). The alias stays the value.
        let named = text(&entry["displayName"], 120).unwrap_or_else(|| id.clone());
        let label = resolved
            .as_deref()
            .or(Some(id.as_str()))
            .and_then(claude_version)
            .map_or_else(
                || named.clone(),
                |(family, version)| versioned(&named, &family, &version),
            );
        models.push(Model {
            label,
            id,
            efforts,
            default: is_default,
            aliases: resolved.into_iter().collect(),
            default_effort: None,
        });
    }
    // A default no listed row names still shows, under its own id.
    if let Some(d) = default.filter(|d| !models.iter().any(|m| m.default || m.id == *d)) {
        let efforts = models
            .first()
            .map(|m| m.efforts.clone())
            .unwrap_or_default();
        models.insert(
            0,
            Model {
                id: d.clone(),
                label: claude_version(&d).map_or_else(
                    || d.clone(),
                    |(mut family, version)| {
                        family[..1].make_ascii_uppercase();
                        format!("{family} {version}")
                    },
                ),
                efforts,
                default: true,
                aliases: vec![],
                default_effort: None,
            },
        );
    }
    models
}
/// A short-lived newline-JSON peer. Dropping it ends the process.
pub(crate) struct Peer {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
}
impl Peer {
    pub(crate) fn start(
        binary: &PathBuf,
        args: &[&str],
        vars: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let mut child = Command::new(binary)
            .args(args)
            .env_clear()
            .envs(vars)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("The agent runtime could not be started: {e}"))?;
        let stdin = child.stdin.take().ok_or("Runtime stdin is unavailable")?;
        let stdout = child.stdout.take().ok_or("Runtime stdout is unavailable")?;
        let (sender, lines) = mpsc::channel();
        std::thread::Builder::new()
            .name("pm-agent-account-read".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            next: 1,
        })
    }
    fn write(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("The agent runtime stopped responding.")?;
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|_| "The agent runtime stopped responding.".to_owned())
    }
    pub(crate) fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"method":method,"params":params}))
    }
    pub(crate) fn request(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        self.write(&json!({"method":method,"id":id,"params":params}))?;
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .map_err(|_| "The agent runtime stopped responding.".to_owned())?;
            let message = serde_json::from_str::<Value>(&line).unwrap_or_default();
            if message["id"] != id {
                continue;
            }
            return match message["error"]["message"].as_str() {
                Some(error) => Err(error.chars().take(DETAIL_LIMIT).collect()),
                None => Ok(message["result"].clone()),
            };
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
/// Runs one CLI step, bounded and cancellable. Returns its code and combined output.
fn run(
    binary: &PathBuf,
    args: &[&str],
    vars: &BTreeMap<String, String>,
    wait: Duration,
    cancel: &AtomicBool,
) -> Result<(Option<i32>, String), String> {
    let mut child = Command::new(binary)
        .args(args)
        .env_clear()
        .envs(vars)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("The agent runtime could not be started: {e}"))?;
    let collect = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    };
    let out = collect(Box::new(child.stdout.take().ok_or("Runtime stdout")?));
    let err = collect(Box::new(child.stderr.take().ok_or("Runtime stderr")?));
    let deadline = Instant::now() + wait;
    let code = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status.code();
        }
        if cancel.load(Ordering::Acquire) || Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Sign-in was cancelled or timed out. Try again.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = out.join().unwrap_or_default();
    if output.trim().is_empty() {
        output = err.join().unwrap_or_default();
    }
    Ok((code, output))
}
#[cfg(test)]
mod tests {
    /// Item 6: Claude Code moves to the Anthropic key only on an explicit
    /// signed-out status, never on a timeout, an error or "checking".
    #[test]
    fn claude_falls_back_to_the_key_only_on_an_explicit_signed_out_status() {
        let of = |state| Ok(Status::of(state, None));
        assert!(claude_key_fallback(&of(State::Disconnected)));
        for state in [State::Connected, State::Unavailable, State::Checking, State::Connecting] {
            assert!(!claude_key_fallback(&of(state)), "{state:?}");
        }
        assert!(!claude_key_fallback(&Err("Claude Code did not return an account status.".into())));
    }
    use super::*;
    #[test]
    fn a_plan_reads_as_words() {
        assert_eq!(plan_name("plus"), "Plus");
        assert_eq!(plan_name("max"), "Max");
        assert_eq!(plan_name("team_pro"), "Team Pro");
    }
    #[test]
    fn sign_in_urls_stay_on_the_providers_own_https_hosts() {
        assert!(trusted_auth_url("https://auth.openai.com/login").is_ok());
        for bad in [
            "http://chatgpt.com/",
            "https://chatgpt.com.example.invalid/",
            "https://notopenai.com/",
            "https://user@openai.com/",
        ] {
            assert!(trusted_auth_url(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn claudes_default_row_is_no_model_and_marks_the_model_it_resolves_to() {
        // Claude Code 2.1's own initialize answer, trimmed to the read fields.
        let e = |value: &str, resolved: &str, name: &str| {
            json!({"value":value,"resolvedModel":resolved,"displayName":name,
                "supportedEffortLevels":["low","high"]})
        };
        let listed = [
            e("default", "claude-opus-5-5", "Default (recommended)"),
            e("opus", "claude-opus-5-5", "Opus"),
            e("fable", "claude-fable-5-2", "Fable"),
            e("sonnet", "claude-sonnet-5-5", "Sonnet"),
        ];
        let models = claude_listing(listed.iter());
        let rows: Vec<_> = models
            .iter()
            .map(|m| (m.id.as_str(), m.label.as_str(), m.default))
            .collect();
        assert_eq!(
            rows,
            [
                ("opus", "Opus 5.5", true),
                ("fable", "Fable 5.2", false),
                ("sonnet", "Sonnet 5.5", false)
            ]
        );
        assert_eq!(models[0].aliases, ["claude-opus-5-5"]);
        assert_eq!(models[0].efforts, ["low", "high"]);
        // A default no row names is still listed, under its id.
        // A 1M-context variant keeps its note after the version; a name
        // that is not the family's is left as given.
        let wide =
            claude_listing([e("opus[1m]", "claude-opus-5-5[1m]", "Opus (1M context)")].iter());
        assert_eq!(wide[0].label, "Opus 5.5 (1M context)");
        assert_eq!(wide[0].id, "opus[1m]");
        assert_eq!(versioned("Opus-5.5 (1M context)", "opus", "5.5"), "Opus 5.5 (1M context)");
        assert_eq!(versioned("Best", "opus", "5.5"), "Best");
        assert_eq!(
            claude_version("claude-3-5-sonnet-20241022"),
            Some(("sonnet".into(), "3.5".into()))
        );
        let only = claude_listing([e("default", "claude-next", "Default (recommended)")].iter());
        assert_eq!(
            (only[0].id.as_str(), only[0].default),
            ("claude-next", true)
        );
        let only = claude_listing([e("default", "claude-haiku-4-5", "Default (recommended)")].iter());
        assert_eq!((only[0].id.as_str(), only[0].label.as_str()), ("claude-haiku-4-5", "Haiku 4.5"));
    }

    struct FakeClaude(PathBuf);
    impl FakeClaude {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;

            let root = std::env::temp_dir().join(format!("pm-claude-{}", crate::runtime::unique()));
            std::fs::create_dir(&root).unwrap();
            let binary = root.join("claude");
            std::fs::write(
                &binary,
                r#"#!/bin/sh
if [ "$*" = "auth status" ]; then
    printf '%s\n' '{"loggedIn":true,"subscriptionType":"pro"}'
    exit
fi
IFS= read -r request
case "$request" in *'"request_id":"models"'*'"subtype":"initialize"'*) ;; *) exit 2 ;; esac
case "$DISCOVERY" in
    slow) exec /bin/sleep 10 ;;
    exit) exit 1 ;;
    *) printf '%s\n' "$DISCOVERY" ;;
esac
"#,
            )
            .unwrap();
            std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(root)
        }
        fn binary(&self) -> PathBuf {
            self.0.join("claude")
        }
        fn vars(&self, answer: &str) -> BTreeMap<String, String> {
            [("DISCOVERY".into(), answer.into())].into()
        }
    }
    impl Drop for FakeClaude {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // The public model fields from Claude Code 2.1.325's initialize reply.
    fn claude_reply() -> String {
        let models: Vec<_> = [
            ("default", "claude-opus-5-5", "Default (recommended)"),
            ("opus", "claude-opus-5-5", "Opus 5.5"),
            ("fable", "claude-fable-5-2", "Fable 5.2"),
            ("sonnet", "claude-sonnet-5-5", "Sonnet 5.5"),
            ("haiku", "claude-haiku-5-5", "Haiku 5.5"),
            ("claude-fable-5-1", "claude-fable-5-1", "Fable 5.1"),
            ("claude-haiku-4-5-20251001", "claude-haiku-4-5-20251001", "Haiku 4.5"),
            ("claude-sonnet-5", "claude-sonnet-5", "Sonnet 5"),
            ("claude-opus-5", "claude-opus-5", "Opus 5"),
            ("claude-fable-5", "claude-fable-5", "Fable 5"),
            ("claude-opus-4-8", "claude-opus-4-8", "Opus 4.8"),
            ("claude-opus-4-7", "claude-opus-4-7", "Opus 4.7"),
            ("claude-opus-4-6", "claude-opus-4-6", "Opus 4.6"),
            ("claude-sonnet-4-6", "claude-sonnet-4-6", "Sonnet 4.6"),
        ]
        .map(|(value, resolved, name)| {
            json!({"value":value,"resolvedModel":resolved,"displayName":name})
        })
        .into();
        json!({"type":"control_response","response":{
            "request_id":"models","subtype":"success","response":{"models":models}
        }})
        .to_string()
    }

    #[test]
    fn claude_discovery_reads_all_real_models_and_marks_the_default() {
        let fake = FakeClaude::new();
        let models = claude_models(&fake.binary(), fake.vars(&claude_reply()), MODELS).unwrap();
        assert_eq!(models.len(), 13);
        assert_eq!(
            (models[0].id.as_str(), models[0].label.as_str()),
            ("opus", "Opus 5.5")
        );
        assert!(models[0].default);
        assert!(!models[1..].iter().any(|model| model.default));
        assert_eq!(models[1].label, "Fable 5.2");
        assert_eq!(models[2].label, "Sonnet 5.5");
        assert_eq!(models[3].label, "Haiku 5.5");
        assert_eq!(models[12].id, "claude-sonnet-4-6");
    }

    #[test]
    fn claude_discovery_distinguishes_a_timeout_from_an_exited_process() {
        let fake = FakeClaude::new();
        let binary = fake.binary();
        let slow = claude_models(&binary, fake.vars("slow"), Duration::from_millis(150));
        assert!(slow.unwrap_err().contains("timed out"));
        let exited = claude_models(&binary, fake.vars("exit"), MODELS);
        assert!(exited.unwrap_err().contains("exited"));
    }

    #[test]
    fn claude_publishes_discovery_progress_until_the_failure_status_arrives() {
        let fake = FakeClaude::new();
        let source = Arc::new(Mutex::new(std::sync::Weak::<Accounts>::new()));
        let read = source.clone();
        let (send, changes) = mpsc::channel();
        let provider = Provider::Claude;
        let accounts = Accounts::new(
            Environment {
                profile: fake.0.join("profile"),
                vars: [
                    ("CLAUDE_BINARY".into(), fake.binary().display().to_string()),
                    ("DISCOVERY".into(), "exit".into()),
                ]
                .into(),
                ..Default::default()
            },
            Arc::new(|_| panic!("a refresh never opens a browser")),
            Arc::new(move || {
                if let Some(accounts) = read.lock().unwrap().upgrade() {
                    let _ = send.send((
                        accounts.finding_models(provider),
                        accounts.status(provider).detail,
                    ));
                }
            }),
        );
        *source.lock().unwrap() = Arc::downgrade(&accounts);
        accounts.refresh(provider);
        let wait = Duration::from_secs(5);
        assert_eq!(changes.recv_timeout(wait).unwrap(), (true, None));
        let (finding, detail) = changes.recv_timeout(wait).unwrap();
        assert!(!finding);
        assert!(detail.unwrap().contains("exited"));
    }

    #[test]
    fn claude_failed_refresh_keeps_the_last_good_catalog_and_reports_why() {
        let fake = FakeClaude::new();
        let mut accounts = Accounts::new(
            Environment {
                profile: fake.0.join("profile"),
                vars: [
                    ("CLAUDE_BINARY".into(), fake.binary().display().to_string()),
                    ("DISCOVERY".into(), claude_reply()),
                ]
                .into(),
                ..Default::default()
            },
            Arc::new(|_| panic!("a refresh never opens a browser")),
            Arc::new(|| {}),
        );
        let cancel = AtomicBool::default();
        let provider = Provider::Claude;
        assert_eq!(
            accounts.run(provider, Job::Refresh, &cancel).state,
            State::Connected
        );
        let good = accounts.models(provider);
        assert_eq!(good.len(), 13);
        for (answer, reason) in [
            ("exit", "exited"),
            ("slow", "timed out"),
            (
                r#"{"type":"control_response","response":{"request_id":"models","subtype":"error"}}"#,
                "rejected",
            ),
            (
                r#"{"type":"control_response","response":{"request_id":"models","subtype":"success","response":{}}}"#,
                "missing",
            ),
            (
                r#"{"type":"control_response","response":{"request_id":"models","subtype":"success","response":{"models":[]}}}"#,
                "no usable models",
            ),
        ] {
            Arc::get_mut(&mut accounts)
                .unwrap()
                .environment
                .vars
                .insert("DISCOVERY".into(), answer.into());
            let timeout = if answer == "slow" {
                Duration::from_millis(150)
            } else {
                MODELS
            };
            let status = accounts.claude(Job::Refresh, &cancel, timeout).unwrap();
            assert_eq!(accounts.models(provider), good, "{answer}");
            assert_eq!(status.state, State::Connected);
            let detail = status.detail.unwrap();
            assert!(detail.contains(reason), "{detail}");
            assert!(detail.contains("last known"), "{detail}");
        }
    }
    /// App Server that answers slowly, has no model list, and signs in at once.
    const SLOW_CODEX: &str = r#"#!/usr/bin/python3
import json, os, sys, time
flag = os.path.join(os.environ['CODEX_HOME'], 'in')
for line in sys.stdin:
    r = json.loads(line)
    m, i = r.get('method'), r.get('id')
    if m == 'account/read':
        time.sleep(0.3)
        a = {'type': 'chatgpt', 'email': 'user@example.invalid'} if os.path.exists(flag) else None
        print(json.dumps({'id': i, 'result': {'account': a}}), flush=True)
    elif m == 'account/login/start':
        open(flag, 'w').close()
        print(json.dumps({'id': i, 'result': {'loginId': 'l', 'authUrl': 'https://chatgpt.com/auth'}}), flush=True)
        print(json.dumps({'method': 'account/login/completed', 'params': {'loginId': 'l', 'success': True}}), flush=True)
    elif m == 'model/list':
        print(json.dumps({'id': i, 'error': {'code': -32601, 'message': 'no models'}}), flush=True)
    elif i is not None:
        print(json.dumps({'id': i, 'result': {}}), flush=True)
"#;
    #[test]
    fn chatgpt_discovery_keeps_the_installed_codex_catalog_across_failure() {
        use std::os::unix::fs::PermissionsExt;
        let fake = FakeClaude::new();
        let codex = fake.0.join("codex");
        let script = r#"#!/usr/bin/python3
import json, os, sys
for line in sys.stdin:
    r = json.loads(line)
    m, i = r.get('method'), r.get('id')
    if i is None:
        continue
    answer = {'id': i, 'result': {}}
    if m == 'account/read':
        answer['result'] = {'account': {'type': 'chatgpt'}}
    elif m == 'model/list':
        mode = os.environ.get('DISCOVERY')
        if mode == 'error':
            del answer['result']
            answer['error'] = {'code': -32601, 'message': 'no models'}
        elif mode != 'malformed':
            answer['result'] = {'data': [] if mode == 'empty' else [
                {'model': 'gpt-6.2-sol', 'displayName': 'GPT-6.2-Sol', 'isDefault': True,
                 'supportedReasoningEfforts': [{'reasoningEffort': 'low'}],
                 'defaultReasoningEffort': 'low'},
                {'model': 'gpt-6.2-astra', 'displayName': 'GPT-6.2-Astra', 'isDefault': False},
                {'model': 'gpt-6.1-terra', 'displayName': 'GPT-6.1 Terra', 'isDefault': False},
                {'model': 'gpt-6.1-sol', 'displayName': 'GPT-6.1-Sol'},
                {'model': 'gpt-6-astra', 'displayName': 'GPT-6-Astra'},
                {'model': 'gpt-5.6-terra'}
            ]}
    print(json.dumps(answer), flush=True)
"#;
        std::fs::write(&codex, script).unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut accounts = Accounts::new(
            Environment {
                profile: fake.0.join("profile"),
                vars: [("CODEX_BINARY".into(), codex.display().to_string())].into(),
                ..Default::default()
            },
            Arc::new(|_| panic!("a refresh never opens a browser")),
            Arc::new(|| {}),
        );
        let provider = Provider::Chatgpt;
        let cancel = AtomicBool::default();
        assert_eq!(accounts.run(provider, Job::Refresh, &cancel).state, State::Connected);
        let good = accounts.models(provider);
        assert_eq!(
            good.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-6.2-sol", "gpt-6.2-astra", "gpt-6.1-terra", "gpt-6.1-sol", "gpt-6-astra", "gpt-5.6-terra"]
        );
        assert!(good[0].default);
        assert_eq!(
            good.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
            ["GPT-6.2 Sol", "GPT-6.2 Astra", "GPT-6.1 Terra", "GPT-6.1 Sol", "GPT-6 Astra", "GPT-5.6 Terra"],
        );
        assert_eq!(good[0].efforts, ["low"]);
        for mode in ["error", "empty", "malformed"] {
            Arc::get_mut(&mut accounts)
                .unwrap()
                .environment
                .vars
                .insert("DISCOVERY".into(), mode.into());
            assert_eq!(accounts.run(provider, Job::Refresh, &cancel).state, State::Connected);
            assert_eq!(accounts.models(provider), good, "{mode}");
        }
    }
    #[test]
    fn a_sign_in_during_a_refresh_runs_after_it_and_a_missing_model_list_still_connects() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pm-account-slow-{}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        let codex = root.join("codex");
        std::fs::write(&codex, SLOW_CODEX).unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let environment = Environment {
            profile: root.join("profile"),
            vars: BTreeMap::from([
                ("HOME".into(), root.join("home").display().to_string()),
                ("PATH".into(), std::env::var("PATH").unwrap()),
                ("CODEX_BINARY".into(), codex.display().to_string()),
            ]),
            ..Default::default()
        };
        let opened = Arc::new(Mutex::new(vec![]));
        let record = opened.clone();
        let accounts = Accounts::new(
            environment,
            Arc::new(move |url: &str| {
                record.lock().unwrap().push(url.to_owned());
                Ok(())
            }),
            Arc::new(|| {}),
        );
        accounts.refresh(Provider::Chatgpt);
        accounts.connect(Provider::Chatgpt);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while accounts.status(Provider::Chatgpt).state != State::Connected {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                accounts.status(Provider::Chatgpt)
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            ["https://chatgpt.com/auth"]
        );
        assert!(accounts.models(Provider::Chatgpt).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn codex_keeps_its_own_file_login_and_never_reads_the_persons() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(format!("pm-account-{}", std::process::id()));
        let shared = root.join("home/.codex");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("auth.json"), "{}").unwrap();
        let environment = Environment {
            profile: root.join("profile"),
            vars: BTreeMap::from([
                ("HOME".into(), root.join("home").display().to_string()),
                ("CODEX_HOME".into(), shared.display().to_string()),
            ]),
            ..Default::default()
        };
        let vars = environment.codex_vars().unwrap();
        let home = PathBuf::from(&vars["CODEX_HOME"]);
        assert_eq!(home, root.join("profile/codex-runtime"));
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // The person's own login is never copied in: they sign in inside Powermove.
        assert!(!home.join("auth.json").exists());
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("cli_auth_credentials_store = \"file\""));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refresh_without_bundled_runtime_reports_disconnected_ready_for_on_demand_download() {
        let root = std::env::temp_dir().join(format!(
            "pm-account-missing-{}",
            crate::runtime::unique()
        ));
        let accounts = Accounts::new(
            Environment {
                profile: root.join("profile"),
                resources: None,
                ..Default::default()
            },
            Arc::new(|_| panic!("refresh never opens a browser")),
            Arc::new(|| {}),
        );
        let cancel = AtomicBool::default();
        for provider in [Provider::Chatgpt, Provider::Claude] {
            let status = accounts.run(provider, Job::Refresh, &cancel);
            assert_eq!(status.state, State::Disconnected, "{provider:?}");
            assert_eq!(status.detail, None, "{provider:?}");
        }
    }
}
