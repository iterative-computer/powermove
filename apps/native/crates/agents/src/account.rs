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
}
impl Status {
    fn of(state: State, detail: Option<String>) -> Self {
        Self {
            state,
            email: None,
            plan: None,
            detail: detail.map(|d| d.chars().take(DETAIL_LIMIT).collect()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
}
struct Slot {
    status: Status,
    models: Vec<Model>,
    worker: bool,
    /// What runs when the current worker finishes. A sign-out outranks a sign-in.
    next: Option<Job>,
    cancel: Arc<AtomicBool>,
}
pub type Opener = dyn Fn(&str) -> Result<(), String> + Send + Sync;
/// The browser hop, a parameter because it is the one step a test must not take.
pub fn open_in_browser(url: &str) -> Result<(), String> {
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
}
#[derive(Clone, Copy, PartialEq)]
enum Job {
    Refresh,
    Connect,
    Disconnect,
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
    pub fn refresh(self: &Arc<Self>, provider: Provider) {
        self.start(provider, Job::Refresh);
    }
    pub fn connect(self: &Arc<Self>, provider: Provider) {
        self.start(provider, Job::Connect);
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
        if matches!(job, Job::Connect) {
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
                    slot.worker = false;
                    slot.next.take()
                };
                this.publish(provider, status);
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
            Provider::Claude => self.claude(job, cancel),
            Provider::Compatible => Ok(Status::of(State::Disconnected, None)),
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
        }
        self.bump();
    }
    fn binary(&self, runtime: Runtime) -> Result<PathBuf, String> {
        self.environment.binary(runtime).ok_or_else(|| {
            let name = match runtime {
                Runtime::Codex => "ChatGPT",
                Runtime::Claude => "Claude",
            };
            format!("Powermove's built-in {name} runtime is missing. Reinstall Powermove.")
        })
    }
    fn chatgpt(&self, job: Job, cancel: &AtomicBool) -> Result<Status, String> {
        let mut peer = Peer::start(
            &self.binary(Runtime::Codex)?,
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
                self.set_models(Provider::Chatgpt, vec![]);
                return Ok(Status::of(State::Disconnected, None));
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
                let url = trusted_auth_url(url)?;
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
        let status = chatgpt_status(&mut peer)?;
        if status.state == State::Connected {
            // A model list is a nicety. The provider's default still runs without it.
            let models = chatgpt_models(&mut peer).unwrap_or_default();
            self.set_models(Provider::Chatgpt, models);
        }
        Ok(status)
    }
    fn claude(&self, job: Job, cancel: &AtomicBool) -> Result<Status, String> {
        let binary = self.binary(Runtime::Claude)?;
        let vars = self.environment.claude_vars()?;
        match job {
            Job::Refresh => {}
            Job::Disconnect => {
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
            Job::Connect => {
                let status = claude_status(&binary, &vars, cancel)?;
                if status.state != State::Disconnected {
                    return Ok(status);
                }
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
            }
        }
        let status = claude_status(&binary, &vars, cancel)?;
        if status.state == State::Connected {
            // A model list is a nicety. The provider's default still runs without it.
            self.set_models(
                Provider::Claude,
                claude_models(&binary, vars).unwrap_or_default(),
            );
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
    if account["type"] != "chatgpt" {
        return Ok(Status::of(State::Disconnected, None));
    }
    Ok(Status {
        state: State::Connected,
        email: text(&account["email"], 320),
        plan: text(&account["planType"], 80),
        detail: None,
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
        for entry in page["data"].as_array().into_iter().flatten() {
            let id = entry["model"].as_str().or(entry["id"].as_str());
            let Some(id) = id.filter(|id| is_valid_model_id(id)) else {
                continue;
            };
            if entry["hidden"] == true || models.iter().any(|m| m.id == id) {
                continue;
            }
            models.push(Model {
                id: id.into(),
                label: text(&entry["displayName"], 120).unwrap_or_else(|| id.into()),
                efforts: entry["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| text(&e["reasoningEffort"], 16))
                    .collect(),
            });
        }
        cursor = text(&page["nextCursor"], 500).filter(|c| !seen.contains(c));
        let Some(next) = cursor.clone() else {
            break;
        };
        seen.push(next);
    }
    Ok(models)
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
fn claude_status(
    binary: &PathBuf,
    vars: &BTreeMap<String, String>,
    cancel: &AtomicBool,
) -> Result<Status, String> {
    let (code, output) = run(binary, &["auth", "status"], vars, REQUEST, cancel)?;
    if matches!(code, Some(126 | 127)) {
        return Err("Claude could not be started. Check its installation and try again.".into());
    }
    let value = serde_json::from_str::<Value>(output.trim()).unwrap_or_default();
    if value["loggedIn"] != true {
        return Ok(Status::of(State::Disconnected, None));
    }
    Ok(Status {
        state: State::Connected,
        email: text(&value["email"], 320),
        plan: text(&value["subscriptionType"], 80).or_else(|| text(&value["authMethod"], 80)),
        detail: None,
    })
}
/// Claude Code answers an initialize control request without a user prompt.
fn claude_models(binary: &PathBuf, vars: BTreeMap<String, String>) -> Result<Vec<Model>, String> {
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
    let deadline = Instant::now() + MODELS;
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        let line = peer
            .lines
            .recv_timeout(wait)
            .map_err(|_| "Claude model discovery timed out.".to_owned())?;
        let message = serde_json::from_str::<Value>(&line).unwrap_or_default();
        if message["type"] != "control_response" || message["response"]["request_id"] != "models" {
            continue;
        }
        let mut models = Vec::<Model>::new();
        let list = message["response"]["response"]["models"].as_array();
        for entry in list.into_iter().flatten() {
            let efforts = entry["supportedEffortLevels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| text(e, 16))
                .collect::<Vec<_>>();
            for key in ["value", "resolvedModel"] {
                let id = entry[key].as_str().filter(|id| is_valid_model_id(id));
                let Some(id) = id.filter(|id| !models.iter().any(|m| m.id == *id)) else {
                    continue;
                };
                models.push(Model {
                    id: id.into(),
                    label: if key == "value" {
                        text(&entry["displayName"], 120).unwrap_or_else(|| id.into())
                    } else {
                        id.into()
                    },
                    efforts: efforts.clone(),
                });
            }
        }
        return Ok(models);
    }
}
/// A short-lived newline-JSON peer. Dropping it ends the process.
struct Peer {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next: u64,
}
impl Peer {
    fn start(
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
            stdin,
            lines,
            next: 1,
        })
    }
    fn write(&mut self, message: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{message}")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "The agent runtime stopped responding.".to_owned())
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"method":method,"params":params}))
    }
    fn request(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
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
    use super::*;
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
    fn an_existing_login_is_copied_once_and_a_sign_out_stays_signed_out() {
        let root = std::env::temp_dir().join(format!("pm-account-{}", std::process::id()));
        let shared = root.join("home/.codex");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("auth.json"), "{}").unwrap();
        let environment = Environment {
            profile: root.join("profile"),
            vars: BTreeMap::from([("HOME".into(), root.join("home").display().to_string())]),
            ..Default::default()
        };
        let vars = environment.codex_vars().unwrap();
        let home = PathBuf::from(&vars["CODEX_HOME"]);
        assert!(home.join("auth.json").exists());
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("cli_auth_credentials_store = \"file\""));
        std::fs::remove_file(home.join("auth.json")).unwrap();
        environment.codex_vars().unwrap();
        assert!(!home.join("auth.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
