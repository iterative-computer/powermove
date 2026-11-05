//! External MCP client registration plans. No startup auto-registration exists.
//! Apply on a worker through a configuration CAS/archive host. Claude Desktop
//! always requires a single-use consent bound to the exact reviewed plan (S070).
use crate::error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};
pub const MCP_FLAG: &str = "--powermove-mcp";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    ClaudeCode,
    Codex,
    ClaudeDesktop,
    GeminiCli,
    Cursor,
    Vscode,
    Windsurf,
    LmStudio,
    Opencode,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Disconnect,
}
impl Client {
    pub const ALL: [Self; 9] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::ClaudeDesktop,
        Self::GeminiCli,
        Self::Cursor,
        Self::Vscode,
        Self::Windsurf,
        Self::LmStudio,
        Self::Opencode,
    ];
    pub fn path(self) -> PathBuf {
        PathBuf::from(match self {
            Self::ClaudeCode => ".claude.json",
            Self::Codex => ".codex/config.toml",
            Self::ClaudeDesktop => "Library/Application Support/Claude/claude_desktop_config.json",
            Self::GeminiCli => ".gemini/settings.json",
            Self::Cursor => ".cursor/mcp.json",
            Self::Vscode => "Library/Application Support/Code/User/mcp.json",
            Self::Windsurf => ".codeium/windsurf/mcp_config.json",
            Self::LmStudio => ".lmstudio/mcp.json",
            Self::Opencode => ".config/opencode/opencode.json",
        })
    }
    fn key(self) -> &'static str {
        match self {
            Self::Vscode => "servers",
            Self::Opencode => "mcp",
            _ => "mcpServers",
        }
    }
}
fn command_owned(command: &Value) -> bool {
    command.as_str().is_some_and(|v| {
        v.ends_with(".app/Contents/MacOS/Powermove")
            && v.rsplit('/').nth(3).is_some_and(|app| {
                app == "Powermove.app" || (app.starts_with("Powermove ") && app.ends_with(".app"))
            })
    })
}
fn owned(client: Client, entry: &Value) -> bool {
    let Some(object) = entry.as_object() else {
        return false;
    };
    if client == Client::Opencode {
        // Preserve all unrecognized fields rather than silently clobber them.
        return object
            .keys()
            .all(|k| ["type", "command", "enabled"].contains(&k.as_str()))
            && entry["command"]
                .as_array()
                .is_some_and(|v| v.len() == 2 && command_owned(&v[0]) && v[1] == MCP_FLAG);
    }
    let allowed = if matches!(client, Client::Vscode | Client::ClaudeCode) {
        &["type", "command", "args", "env"][..]
    } else {
        &["command", "args"][..]
    };
    command_owned(&entry["command"])
        && entry["args"] == json!([MCP_FLAG])
        && object.keys().all(|k| allowed.contains(&k.as_str()))
        && entry
            .get("env")
            .is_none_or(|v| v.as_object().is_some_and(|v| v.is_empty()))
}
fn entry(client: Client, exe: &str) -> Value {
    match client {
        Client::Vscode => json!({"type":"stdio","command":exe,"args":[MCP_FLAG]}),
        Client::Opencode => json!({"type":"local","command":[exe,MCP_FLAG],"enabled":true}),
        _ => json!({"command":exe,"args":[MCP_FLAG]}),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Available,
    Connected { custom: bool },
    Unreadable,
}
fn parse(bytes: Option<&[u8]>) -> io::Result<Value> {
    let text = std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|_| error("Unsupported configuration"))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|_| error("Unsupported configuration; use the manual setup entry"))?;
    if !value.is_object() {
        return Err(error("Unsupported configuration"));
    }
    Ok(value)
}
fn codex_table(text: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let clean = line.split('#').next().unwrap_or("").trim();
        let header = matches!(
            clean,
            "[mcp_servers.powermove]" | "[mcp_servers.\"powermove\"]" | "[mcp_servers.'powermove']"
        );
        if let Some(start) = start
            && clean.starts_with('[')
        {
            return Some((start, offset));
        }
        if header {
            start = Some(offset);
        }
        offset += line.len();
    }
    start.map(|start| (start, text.len()))
}
fn codex_owned(body: &str) -> bool {
    let mut command = None;
    let mut args = None;
    for line in body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.starts_with('#'))
    {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        match key.trim() {
            "command" => {
                if command.is_some() {
                    return false;
                }
                command = serde_json::from_str::<Value>(value.trim()).ok();
            }
            "args" => {
                if args.is_some() {
                    return false;
                }
                args = serde_json::from_str::<Value>(value.trim().replace(",]", "]").as_str()).ok();
            }
            "startup_timeout_sec" => {}
            _ => return false,
        }
    }
    command.as_ref().is_some_and(command_owned) && args == Some(json!([MCP_FLAG]))
}
fn codex_mentioned(text: &str) -> bool {
    codex_table(text).is_some()
        || text.lines().any(|line| {
            let line = line.trim();
            !line.starts_with('#')
                && line.contains("powermove")
                && (line.contains("mcp_servers") || text.contains("[mcp_servers]"))
        })
}
pub fn state(client: Client, bytes: Option<&[u8]>) -> State {
    if bytes.is_some_and(|v| v.len() > 16 * 1024 * 1024) {
        return State::Unreadable;
    }
    if client == Client::Codex {
        let Ok(text) = std::str::from_utf8(bytes.unwrap_or_default()) else {
            return State::Unreadable;
        };
        return if codex_mentioned(text) {
            State::Connected {
                custom: codex_table(text).is_none_or(|(a, b)| !codex_owned(&text[a..b])),
            }
        } else {
            State::Available
        };
    }
    let Ok(value) = parse(bytes) else {
        return State::Unreadable;
    };
    let Some(servers) = value.get(client.key()) else {
        return State::Available;
    };
    let Some(servers) = servers.as_object() else {
        return State::Unreadable;
    };
    match servers.get("powermove") {
        None => State::Available,
        Some(entry) => State::Connected {
            custom: !owned(client, entry),
        },
    }
}
pub struct Plan {
    client: Client,
    action: Action,
    path: PathBuf,
    original: Option<Vec<u8>>,
    replacement: Option<Vec<u8>>,
    cli: Vec<Vec<String>>,
    fingerprint: String,
}
impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrationPlan")
            .field("client", &self.client)
            .field("action", &self.action)
            .field("path", &self.path)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}
impl Plan {
    pub fn client(&self) -> Client {
        self.client
    }
    pub fn action(&self) -> Action {
        self.action
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn original(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }
    pub fn replacement(&self) -> Option<&[u8]> {
        self.replacement.as_deref()
    }
    pub fn cli(&self) -> &[Vec<String>] {
        &self.cli
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn changes(&self) -> bool {
        self.replacement.is_some() || !self.cli.is_empty()
    }
}
pub fn prepare(
    client: Client,
    action: Action,
    original: Option<&[u8]>,
    exe: &str,
) -> io::Result<Plan> {
    if !Path::new(exe).is_absolute() || exe.contains('\0') {
        return Err(error(
            "Powermove executable must be an absolute native path",
        ));
    }
    let current = state(client, original);
    let mut replacement = None;
    let mut cli = vec![];
    match current {
        State::Unreadable => {
            return Err(error(
                "Configuration cannot safely be edited; preserve it and use manual setup",
            ));
        }
        State::Connected { custom: true } => {
            if action == Action::Disconnect {
                return Err(error("This connection was set up by hand."));
            }
        }
        State::Available if action == Action::Disconnect => {}
        _ if client == Client::ClaudeCode => {
            if matches!(current, State::Connected { .. }) {
                cli.push(vec![
                    "mcp".into(),
                    "remove".into(),
                    "--scope".into(),
                    "user".into(),
                    "powermove".into(),
                ]);
            }
            if action == Action::Connect {
                cli.push(vec![
                    "mcp".into(),
                    "add".into(),
                    "--scope".into(),
                    "user".into(),
                    "powermove".into(),
                    "--".into(),
                    exe.into(),
                    MCP_FLAG.into(),
                ]);
            }
        }
        _ if client == Client::Codex => {
            let text = std::str::from_utf8(original.unwrap_or_default())
                .map_err(|_| error("Unsupported configuration"))?;
            let without = if let Some((a, b)) = codex_table(text) {
                format!("{}{}", &text[..a], &text[b..])
            } else {
                text.into()
            };
            let result = if action == Action::Connect {
                let base = without.trim_end();
                format!(
                    "{base}{}[mcp_servers.powermove]\ncommand = {}\nargs = [\"{MCP_FLAG}\"]\n# The hidden editor takes a while to start the first time.\nstartup_timeout_sec = 60\n",
                    if base.is_empty() { "" } else { "\n\n" },
                    serde_json::to_string(exe)?
                )
            } else {
                let mut text = without;
                while text.contains("\n\n\n") {
                    text = text.replace("\n\n\n", "\n\n");
                }
                format!("{}\n", text.trim_end())
            };
            replacement = Some(result.into_bytes());
        }
        _ => {
            let mut value = parse(original)?;
            let key = client.key();
            if value.get(key).is_none() {
                value[key] = json!({});
            }
            let servers = value[key]
                .as_object_mut()
                .ok_or_else(|| error("Unsupported configuration"))?;
            if action == Action::Connect {
                servers.insert("powermove".into(), entry(client, exe));
            } else {
                servers.remove("powermove");
            }
            replacement = Some(format!("{}\n", serde_json::to_string_pretty(&value)?).into_bytes());
        }
    }
    if replacement.as_deref() == original {
        replacement = None;
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"client":client,"connect":action==Action::Connect,"original":original,"replacement":replacement,"cli":cli,"executable":exe})
        )?)
    );
    Ok(Plan {
        client,
        action,
        path: client.path(),
        original: original.map(Vec::from),
        replacement,
        cli,
        fingerprint,
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentRequest {
    pub id: u64,
    pub plan: String,
}
pub struct Permit {
    id: u64,
    plan: String,
}
#[derive(Default)]
pub struct ConsentGate {
    next: u64,
    pending: BTreeMap<u64, String>,
    approved: BTreeMap<u64, String>,
}
impl ConsentGate {
    pub fn request(&mut self, plan: &Plan) -> io::Result<ConsentRequest> {
        if self.pending.len() + self.approved.len() >= 64 {
            return Err(error("Too many pending registration consent requests"));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("Consent identity exhausted"))?;
        self.pending.insert(self.next, plan.fingerprint.clone());
        Ok(ConsentRequest {
            id: self.next,
            plan: plan.fingerprint.clone(),
        })
    }
    /// Only the native user-consent handler calls this with an explicit answer.
    /// No default, persisted startup flag, agent tool or timeout grants consent.
    pub fn decide(
        &mut self,
        request: &ConsentRequest,
        accepted: bool,
    ) -> io::Result<Option<Permit>> {
        let expected = self
            .pending
            .remove(&request.id)
            .ok_or_else(|| error("Consent request expired"))?;
        if expected != request.plan {
            return Err(error("Consent plan changed"));
        }
        if !accepted {
            return Ok(None);
        }
        self.approved.insert(request.id, expected.clone());
        Ok(Some(Permit {
            id: request.id,
            plan: expected,
        }))
    }
    fn consume(&mut self, plan: &Plan, permit: Option<Permit>) -> io::Result<()> {
        let permit = permit
            .ok_or_else(|| error("Explicit Claude desktop registration consent is required"))?;
        let approved = self
            .approved
            .remove(&permit.id)
            .ok_or_else(|| error("Consent is no longer valid"))?;
        if approved != plan.fingerprint || permit.plan != plan.fingerprint {
            return Err(error(
                "Consent does not authorize this configuration change",
            ));
        }
        Ok(())
    }
}
/// S070's durable answer suppresses a repeated alert; it never authorizes a
/// future write. Each actual Connect still requires a fresh plan-bound permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    Added,
    Declined,
    Skipped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    Absent,
    Original,
    Current,
    IntentionallyRemoved,
    Custom,
    Unreadable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchConsent {
    Wait,
    Done,
    Skip,
    Ask,
    AskSwitch,
}
/// `ready` means S070's first-launch/onboarding, key-window, sheet, playback
/// and text-editing gates all permit presentation. Hidden hosts never ask.
pub fn launch_consent(
    recorded: Option<Answer>,
    found: Found,
    installed: bool,
    ready: bool,
    hidden: bool,
) -> LaunchConsent {
    if recorded.is_some() {
        return LaunchConsent::Done;
    }
    if hidden || !installed || found == Found::Unreadable {
        return LaunchConsent::Wait;
    }
    if matches!(
        found,
        Found::Current | Found::IntentionallyRemoved | Found::Custom
    ) {
        return LaunchConsent::Skip;
    }
    if !ready {
        return LaunchConsent::Wait;
    }
    if found == Found::Original {
        LaunchConsent::AskSwitch
    } else {
        LaunchConsent::Ask
    }
}
/// Persist an explicit alert/button answer before allowing the settings write.
/// Failure returns no permit. Decline changes only the profile answer.
pub fn answer(
    host: &mut impl Host,
    gate: &mut ConsentGate,
    plan: &Plan,
    request: &ConsentRequest,
    accepted: bool,
) -> io::Result<Option<Permit>> {
    if plan.client != Client::ClaudeDesktop
        || plan.action != Action::Connect
        || request.plan != plan.fingerprint
    {
        return Err(error("Consent plan changed"));
    }
    let permit = gate.decide(request, accepted)?;
    if let Err(err) = host.record_answer(if accepted {
        Answer::Added
    } else {
        Answer::Declined
    }) {
        if let Some(permit) = permit {
            gate.approved.remove(&permit.id);
        }
        return Err(err);
    }
    Ok(permit)
}
pub trait Host {
    /// Durably publish `externalClients.claudeDesktop` before any settings write.
    /// Archive the previous profile value; this record is not a reusable grant.
    fn record_answer(&mut self, answer: Answer) -> io::Result<()>;
    /// Compare reviewed bytes, archive the complete prior config/receipt, publish
    /// atomically with no symlink traversal, fsync, and re-read the result.
    /// Claude Code must use the native CLI, never rewrite its live config file.
    /// For a Claude Desktop disconnect, include the intentional-disconnect
    /// receipt in this recovery transaction; no startup auto-registration.
    fn commit(&mut self, plan: &Plan) -> io::Result<()>;
}
pub fn apply(
    host: &mut impl Host,
    gate: &mut ConsentGate,
    plan: &Plan,
    permit: Option<Permit>,
) -> io::Result<()> {
    if !plan.changes() {
        return Ok(());
    }
    if plan.client == Client::ClaudeDesktop && plan.action == Action::Connect {
        gate.consume(plan, permit)?;
        host.record_answer(Answer::Added)?;
    }
    if plan.client == Client::ClaudeDesktop && plan.action == Action::Disconnect {
        host.record_answer(Answer::Declined)?;
    }
    host.commit(plan)?;
    Ok(())
}

pub fn specs() -> &'static [Value] {
    static SPECS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../data/client-specs-main.json"))
            .expect("generated public client metadata")
    });
    &SPECS
}
pub trait Probe {
    /// Fixed paths from the public catalog only. The adapter resolves relative
    /// paths under the explicit home and system app paths on macOS.
    fn present(&mut self, path: &Path) -> io::Result<bool>;
    /// Bounded, regular-file, no-follow reads. No secrets are returned in status.
    fn configuration(&mut self, relative: &Path) -> io::Result<Option<Vec<u8>>>;
}
pub fn list_status(probe: &mut impl Probe, home: &Path) -> Vec<Value> {
    specs().iter().map(|spec|{
        let client:Client=serde_json::from_value(spec["id"].clone()).expect("known client metadata");
        let mut installed=false;let mut denied=false;
        for path in spec["installed"].as_array().unwrap().iter().filter_map(Value::as_str) {
            match probe.present(Path::new(path)){Ok(true)=>{installed=true;break;},Ok(false)=>{},Err(_)=>denied=true}
        }
        let status=if installed {probe.configuration(&client.path()).map(|bytes|state(client,bytes.as_deref())).unwrap_or(State::Unreadable)}else{State::Available};
        let mut value=json!({"id":spec["id"],"name":spec["name"],"detail":spec["detail"],"configPath":home.join(client.path()).to_string_lossy(),"custom":false});
        value["state"]=json!(if !installed {if denied{"unreadable"}else{"not-installed"}}else{match status{State::Available=>"available",State::Connected{custom}=>{value["custom"]=json!(custom);"connected"},State::Unreadable=>"unreadable"}});
        value
    }).collect()
}
