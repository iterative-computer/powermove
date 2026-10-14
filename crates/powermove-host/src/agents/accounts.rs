//! ChatGPT (Codex) and Claude connection state, sign-in and model lists
//! (port of `codex/app-server-account.ts` and `claude/account.ts`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;

use super::app_server::{AppServer, Incoming};
use super::runtime::{self, Runtimes};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MODEL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DETAIL_CHARS: usize = 500;
const CODEX_EFFORTS: [&str; 7] = ["none", "low", "medium", "high", "xhigh", "max", "ultra"];
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub type Publish = Arc<dyn Fn(Value) + Send + Sync>;

pub fn status(state: &str, email: Option<&str>, plan: Option<&str>, detail: Option<&str>) -> Value {
    json!({ "state": state, "email": email, "planType": plan, "detail": detail.map(|text| super::trace::truncate(text, MAX_DETAIL_CHARS)) })
}

fn disconnected(detail: Option<&str>) -> Value {
    status("disconnected", None, None, detail)
}

fn unavailable(detail: &str) -> Value {
    status("unavailable", None, None, Some(detail))
}

/// `MODEL_ID` patterns from the account clients.
fn model_id(value: &str, brackets: bool) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(byte) || (brackets && b"[]".contains(byte)))
}

/* ── ChatGPT through the Codex App Server ───────────────────── */

pub fn chatgpt_status_from(value: &Value) -> Value {
    let account = &value["account"];
    if account["type"] != "chatgpt" {
        return disconnected(None);
    }
    status("connected", account["email"].as_str(), account["planType"].as_str(), None)
}

pub fn codex_models_from(value: &Value) -> Vec<Value> {
    let mut models: Vec<Value> = Vec::new();
    for entry in value["data"].as_array().into_iter().flatten() {
        if entry["hidden"] == true {
            continue;
        }
        let Some(id) = entry["model"].as_str().or_else(|| entry["id"].as_str()) else { continue };
        if !model_id(id, false) || models.iter().any(|model| model["id"] == id) {
            continue;
        }
        let mut efforts: Vec<&str> = Vec::new();
        for effort in entry["supportedReasoningEfforts"].as_array().into_iter().flatten().filter_map(|item| item["reasoningEffort"].as_str()) {
            if CODEX_EFFORTS.contains(&effort) && !efforts.contains(&effort) {
                efforts.push(effort);
            }
        }
        let label = entry["displayName"].as_str().filter(|label| label.len() <= 120).unwrap_or(id);
        models.push(json!({ "id": id, "label": label, "reasoningEfforts": efforts }));
    }
    models
}

fn allowed_auth_url(raw: &str) -> Result<String> {
    let Some(rest) = raw.strip_prefix("https://") else { bail!("Codex returned an unsafe ChatGPT authentication URL.") };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    let host = host.rsplit_once('@').map_or(host.as_str(), |(_, host)| host).split(':').next().unwrap_or_default().to_owned();
    let trusted = host == "chatgpt.com" || host.ends_with(".chatgpt.com") || host == "openai.com" || host.ends_with(".openai.com");
    if !trusted || raw.chars().any(char::is_whitespace) {
        bail!("Codex returned an unsafe ChatGPT authentication URL.");
    }
    Ok(raw.to_owned())
}

pub struct ChatGpt {
    runtimes: Runtimes,
    server: tokio::sync::Mutex<Option<Arc<AppServer>>>,
    current: Mutex<Value>,
    login: Mutex<Option<String>>,
    publish: Publish,
    open_external: Arc<dyn Fn(&str) -> Result<()> + Send + Sync>,
}

impl ChatGpt {
    pub fn new(runtimes: Runtimes, publish: Publish, open_external: Arc<dyn Fn(&str) -> Result<()> + Send + Sync>) -> Arc<Self> {
        Arc::new(Self { runtimes, server: tokio::sync::Mutex::default(), current: Mutex::new(status("checking", None, None, None)), login: Mutex::default(), publish, open_external })
    }

    fn publish(&self, value: Value) -> Value {
        if value["state"] == "connected" {
            *self.login.lock() = None;
        }
        *self.current.lock() = value.clone();
        (self.publish)(value.clone());
        value
    }

    async fn server(self: &Arc<Self>) -> Result<Arc<AppServer>> {
        let mut slot = self.server.lock().await;
        if let Some(server) = slot.as_ref().filter(|server| server.is_alive()) {
            return Ok(server.clone());
        }
        let binary = self.runtimes.codex()?;
        let env = self.runtimes.codex_env(&binary).await?;
        let (server, events) = AppServer::start(&binary, &env, "powermove", false, REQUEST_TIMEOUT).await?;
        *slot = Some(server.clone());
        tokio::spawn(Self::watch(Arc::downgrade(self), events));
        Ok(server)
    }

    async fn watch(this: std::sync::Weak<Self>, mut events: mpsc::UnboundedReceiver<Incoming>) {
        while let Some(event) = events.recv().await {
            let Some(this) = this.upgrade() else { return };
            match event {
                Incoming::Notification { method, params } if method == "account/updated" || method == "account/login/completed" => {
                    if method == "account/login/completed" {
                        {
                            let mut login = this.login.lock();
                            if login.as_deref().is_some_and(|id| params["loginId"] == id) {
                                *login = None;
                            }
                        }
                        if params["success"] == false {
                            let detail = params["error"].as_str().unwrap_or("ChatGPT sign-in was not completed.").to_owned();
                            this.publish(disconnected(Some(&detail)));
                            continue;
                        }
                    }
                    // The notifying server is live; reusing it keeps this task from spawning another watcher.
                    let server = this.server.lock().await.clone();
                    let refreshed = match server {
                        Some(server) => server.request("account/read", json!({ "refreshToken": false })).await.map(|value| chatgpt_status_from(&value)),
                        None => Err(anyhow!("Codex App Server stopped.")),
                    };
                    this.publish(refreshed.unwrap_or_else(|error| unavailable(&format!("{error:#}"))));
                }
                Incoming::Request { id, method, .. } => {
                    if let Some(server) = this.server.lock().await.as_ref() {
                        server.respond_error(&id, &format!("Powermove does not handle {method}."));
                    }
                }
                Incoming::Exited(message) => {
                    *this.login.lock() = None;
                    this.publish(unavailable(&message));
                    return;
                }
                Incoming::Notification { .. } => {}
            }
        }
    }

    pub async fn status(self: &Arc<Self>) -> Value {
        let current = self.current.lock().clone();
        if current["state"] == "connecting" {
            return current;
        }
        let result = async { self.server().await?.request("account/read", json!({ "refreshToken": false })).await }.await;
        self.publish(match result {
            Ok(value) => chatgpt_status_from(&value),
            Err(error) => unavailable(&format!("{error:#}")),
        })
    }

    pub async fn models(self: &Arc<Self>) -> Result<Vec<Value>> {
        let server = self.server().await?;
        let mut models: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = Vec::new();
        for _ in 0..5 {
            let mut params = json!({ "limit": 100, "includeHidden": false });
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let result = server.request("model/list", params).await?;
            for model in codex_models_from(&result) {
                if !models.iter().any(|item| item["id"] == model["id"]) {
                    models.push(model);
                }
            }
            match result["nextCursor"].as_str() {
                Some(next) if !seen.iter().any(|item| item == next) => {
                    seen.push(next.to_owned());
                    cursor = Some(next.to_owned());
                }
                _ => break,
            }
        }
        Ok(models)
    }

    pub async fn connect(self: &Arc<Self>) -> Value {
        if self.current.lock()["state"] == "connecting" && self.login.lock().is_some() {
            return self.current.lock().clone();
        }
        let existing = self.status().await;
        if existing["state"] != "disconnected" {
            return existing;
        }
        let result: Result<Value> = async {
            let server = self.server().await?;
            let started = server.request("account/login/start", json!({ "type": "chatgpt", "useHostedLoginSuccessPage": true, "appBrand": "chatgpt" })).await?;
            let (Some(login_id), Some(url)) = (started["loginId"].as_str(), started["authUrl"].as_str()) else {
                bail!("Codex did not return a ChatGPT sign-in URL.");
            };
            let login_id = login_id.to_owned();
            let url = match allowed_auth_url(url) {
                Ok(url) => url,
                Err(error) => {
                    let _ = server.request("account/login/cancel", json!({ "loginId": login_id })).await;
                    return Err(error);
                }
            };
            *self.login.lock() = Some(login_id.clone());
            let this = Arc::downgrade(self);
            tokio::spawn(async move {
                tokio::time::sleep(LOGIN_TIMEOUT).await;
                let Some(this) = this.upgrade() else { return };
                if this.login.lock().as_deref() != Some(login_id.as_str()) {
                    return;
                }
                this.cancel_login(&login_id).await;
                this.publish(disconnected(Some("ChatGPT sign-in timed out. Try connecting again.")));
            });
            (self.open_external)(&url)?;
            Ok(status("connecting", None, None, Some("Finish signing in in your browser.")))
        }
        .await;
        match result {
            Ok(value) => self.publish(value),
            Err(error) => self.publish(disconnected(Some(&format!("{error:#}")))),
        }
    }

    async fn cancel_login(&self, login_id: &str) {
        {
            let mut login = self.login.lock();
            if login.as_deref() == Some(login_id) {
                *login = None;
            }
        }
        if let Some(server) = self.server.lock().await.clone() {
            let _ = server.request("account/login/cancel", json!({ "loginId": login_id })).await;
        }
    }

    pub async fn disconnect(self: &Arc<Self>) -> Value {
        let result: Result<()> = async {
            let server = self.server().await?;
            let login = self.login.lock().clone();
            if let Some(login) = login {
                self.cancel_login(&login).await;
            }
            server.request("account/logout", json!({})).await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => self.publish(disconnected(None)),
            Err(error) => self.publish(unavailable(&format!("{error:#}"))),
        }
    }

    pub async fn shutdown(&self) {
        *self.login.lock() = None;
        if let Some(server) = self.server.lock().await.take() {
            server.shutdown().await;
        }
    }
}

/* ── Claude Code ─────────────────────────────────────────────── */

pub fn claude_status_from(value: &Value) -> Value {
    if value["loggedIn"] != true {
        return disconnected(None);
    }
    let plan = value["subscriptionType"].as_str().or_else(|| value["authMethod"].as_str());
    status("connected", value["email"].as_str(), plan, None)
}

/// `claudeModelsFromSdk`: the CLI's `initialize` model list.
pub fn claude_models_from(models: &Value) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    for model in models.as_array().into_iter().flatten() {
        let efforts: Vec<&str> = model["supportedEffortLevels"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|effort| CLAUDE_EFFORTS.contains(effort)).collect();
        for key in ["value", "resolvedModel"] {
            let Some(id) = model[key].as_str() else { continue };
            if !model_id(id, true) || result.iter().any(|item| item["id"] == id) {
                continue;
            }
            let label = if key == "value" { model["displayName"].as_str().filter(|label| label.len() <= 120).unwrap_or(id) } else { id };
            result.push(json!({ "id": id, "label": label, "reasoningEfforts": efforts }));
        }
    }
    result
}

pub struct Claude {
    runtimes: Runtimes,
    current: Mutex<Value>,
    login: tokio::sync::Mutex<Option<Child>>,
    publish: Publish,
    /// Model id → supported efforts, from the last discovery.
    pub efforts: Mutex<HashMap<String, Vec<String>>>,
}

impl Claude {
    pub fn new(runtimes: Runtimes, publish: Publish) -> Arc<Self> {
        Arc::new(Self { runtimes, current: Mutex::new(status("checking", None, None, None)), login: tokio::sync::Mutex::default(), publish, efforts: Mutex::default() })
    }

    fn publish(&self, value: Value) -> Value {
        *self.current.lock() = value.clone();
        (self.publish)(value.clone());
        value
    }

    async fn command(&self, args: &[&str]) -> Result<std::process::Output> {
        let binary = self.runtimes.claude()?;
        let home = self.runtimes.claude_home().await?;
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        runtime::output(runtime::command(&binary, &args, None, &[("CLAUDE_CONFIG_DIR", home.into_os_string())], &["CLAUDE_CODE_SAFE_MODE"]), REQUEST_TIMEOUT).await
    }

    pub async fn status(&self) -> Value {
        let current = self.current.lock().clone();
        if current["state"] == "connecting" {
            return current;
        }
        let value = match self.command(&["auth", "status"]).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let text = if stdout.trim().is_empty() { stderr.trim().to_owned() } else { stdout.trim().to_owned() };
                match serde_json::from_str::<Value>(&text) {
                    Ok(parsed) => claude_status_from(&parsed),
                    Err(_) if output.status.success() => disconnected(None),
                    Err(_) => disconnected(Some("Sign in to Claude to use your subscription.")),
                }
            }
            Err(error) => unavailable(&format!("{error:#}")),
        };
        self.publish(value)
    }

    pub async fn models(&self) -> Result<Vec<Value>> {
        let binary = self.runtimes.claude()?;
        let home = self.runtimes.claude_home().await?;
        let args: Vec<String> = ["--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--tools", "", "--setting-sources", ""].iter().map(|arg| (*arg).to_owned()).collect();
        let mut child = runtime::command(&binary, &args, None, &[("CLAUDE_CONFIG_DIR", home.into_os_string())], &["CLAUDE_CODE_SAFE_MODE"]).spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("Claude Code has no stdin."))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Claude Code has no stdout."))?;
        // Streaming input lets the CLI answer its control request without a prompt.
        stdin.write_all(b"{\"type\":\"control_request\",\"request_id\":\"powermove-models\",\"request\":{\"subtype\":\"initialize\"}}\n").await?;
        stdin.flush().await?;
        let read = async {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await? {
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                if event["type"] == "control_response" && event["response"]["request_id"] == "powermove-models" {
                    if event["response"]["subtype"] != "success" {
                        bail!("Claude Code could not list its models.");
                    }
                    return Ok(claude_models_from(&event["response"]["response"]["models"]));
                }
            }
            bail!("Claude Code stopped before listing its models.")
        };
        let result = match tokio::time::timeout(MODEL_TIMEOUT, read).await {
            Ok(result) => result,
            Err(_) => Err(anyhow!("Claude model discovery timed out.")),
        };
        drop(stdin);
        runtime::kill_tree(&mut child).await;
        if let Ok(models) = &result {
            let mut efforts = self.efforts.lock();
            efforts.clear();
            for model in models {
                let list = model["reasoningEfforts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
                efforts.insert(model["id"].as_str().unwrap_or_default().to_owned(), list);
            }
        }
        result
    }

    pub async fn connect(self: &Arc<Self>) -> Value {
        if self.current.lock()["state"] == "connecting" && self.login.lock().await.is_some() {
            return self.current.lock().clone();
        }
        let existing = self.status().await;
        if existing["state"] != "disconnected" {
            return existing;
        }
        let spawned: Result<Child> = async {
            let binary = self.runtimes.claude()?;
            let home = self.runtimes.claude_home().await?;
            let args = vec!["auth".to_owned(), "login".to_owned(), "--claudeai".to_owned()];
            // Claude Code opens the browser itself and waits for the callback.
            Ok(runtime::command(&binary, &args, None, &[("CLAUDE_CONFIG_DIR", home.into_os_string())], &["CLAUDE_CODE_SAFE_MODE"]).spawn()?)
        }
        .await;
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return self.publish(unavailable(&format!("{error:#}"))),
        };
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        *self.login.lock().await = Some(child);
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let diagnostic = async {
                let mut text = Vec::new();
                if let Some(stdout) = stdout.as_mut() {
                    let _ = tokio::io::AsyncReadExt::read_to_end(stdout, &mut text).await;
                }
                if let Some(stderr) = stderr.as_mut() {
                    let _ = tokio::io::AsyncReadExt::read_to_end(stderr, &mut text).await;
                }
                String::from_utf8_lossy(&text).trim().to_owned()
            };
            let outcome = tokio::time::timeout(LOGIN_TIMEOUT, diagnostic).await;
            let Some(this) = this.upgrade() else { return };
            let Some(mut child) = this.login.lock().await.take() else { return };
            let finished = match outcome {
                Ok(diagnostic) => match child.wait().await {
                    Ok(code) if code.success() => Ok(()),
                    _ => Err(if diagnostic.is_empty() { "Claude sign-in was not completed.".to_owned() } else { diagnostic.lines().last().unwrap_or_default().to_owned() }),
                },
                Err(_) => {
                    runtime::kill_tree(&mut child).await;
                    Err("Claude sign-in timed out. Try connecting again.".to_owned())
                }
            };
            match finished {
                Ok(()) => {
                    *this.current.lock() = status("checking", None, None, None);
                    this.status().await;
                }
                Err(detail) => {
                    this.publish(disconnected(Some(&detail)));
                }
            }
        });
        self.publish(status("connecting", None, None, Some("Finish signing in in the browser opened by Claude Code.")))
    }

    pub async fn disconnect(&self) -> Value {
        self.stop_login().await;
        let value = match self.command(&["auth", "logout"]).await {
            Ok(output) if output.status.success() => disconnected(None),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                unavailable(if stderr.is_empty() { "Claude Code could not sign out." } else { &stderr })
            }
            Err(error) => unavailable(&format!("{error:#}")),
        };
        self.publish(value)
    }

    async fn stop_login(&self) {
        if let Some(mut child) = self.login.lock().await.take() {
            runtime::kill_tree(&mut child).await;
        }
    }

    pub async fn shutdown(&self) {
        self.stop_login().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_account_states() {
        let connected = chatgpt_status_from(&json!({ "account": { "type": "chatgpt", "email": "a@b.c", "planType": "plus" } }));
        assert_eq!(connected["state"], "connected");
        assert_eq!(connected["planType"], "plus");
        assert_eq!(chatgpt_status_from(&json!({ "account": null }))["state"], "disconnected");
        let claude = claude_status_from(&json!({ "loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "pro", "email": "a@b.c" }));
        assert_eq!(claude["planType"], "pro");
        assert_eq!(claude_status_from(&json!({ "loggedIn": false }))["state"], "disconnected");
    }

    #[test]
    fn parses_model_lists() {
        let codex = codex_models_from(&json!({ "data": [
            { "model": "gpt-6-sol", "displayName": "GPT 6 Sol", "supportedReasoningEfforts": [{ "reasoningEffort": "high" }, { "reasoningEffort": "bogus" }] },
            { "model": "hidden", "hidden": true },
            { "model": "--flag" }
        ] }));
        assert_eq!(codex, vec![json!({ "id": "gpt-6-sol", "label": "GPT 6 Sol", "reasoningEfforts": ["high"] })]);
        let claude = claude_models_from(&json!([
            { "value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus 5.5", "supportedEffortLevels": ["low", "max", "ultra"] }
        ]));
        assert_eq!(claude.len(), 2);
        assert_eq!(claude[0]["label"], "Opus 5.5");
        assert_eq!(claude[1]["id"], "claude-opus-5-5");
        assert_eq!(claude[1]["reasoningEfforts"], json!(["low", "max"]));
    }

    #[test]
    fn only_trusts_openai_sign_in_urls() {
        assert!(allowed_auth_url("https://auth.openai.com/oauth/authorize?x=1").is_ok());
        assert!(allowed_auth_url("https://chatgpt.com/login").is_ok());
        assert!(allowed_auth_url("http://auth.openai.com/").is_err());
        assert!(allowed_auth_url("https://openai.com.evil.example/").is_err());
        assert!(allowed_auth_url("https://evil.example/@openai.com").is_err());
    }
}
