//! A02/S070 external MCP client backing from main c7f46a0564, agent-clients.ts.
//! Owner 2026-11-04: ask first, including Claude Desktop; never register at launch.
//! The nine-client rows and once-only consent follow S070 at 78294c4c9.
//! Return facts and setup payloads only; native hosts own presentation and IO.
use super::{Flight, Ticket};
use crate::{Result, ServiceError, ServiceErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MCP_FLAG: &str = "--powermove-mcp";
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
pub const CLIENTS: &[Client] = &[
    Client::ClaudeCode,
    Client::Codex,
    Client::ClaudeDesktop,
    Client::GeminiCli,
    Client::Cursor,
    Client::Vscode,
    Client::Windsurf,
    Client::LmStudio,
    Client::Opencode,
];
impl Client {
    pub fn config(self) -> &'static str {
        match self {
            Self::ClaudeCode => ".claude.json",
            Self::Codex => ".codex/config.toml",
            Self::ClaudeDesktop => "Library/Application Support/Claude/claude_desktop_config.json",
            Self::GeminiCli => ".gemini/settings.json",
            Self::Cursor => ".cursor/mcp.json",
            Self::Vscode => "Library/Application Support/Code/User/mcp.json",
            Self::Windsurf => ".codeium/windsurf/mcp_config.json",
            Self::LmStudio => ".lmstudio/mcp.json",
            Self::Opencode => ".config/opencode/opencode.json",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Vscode => "servers",
            Self::Opencode => "mcp",
            _ => "mcpServers",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub client: Client,
    pub managed_entry_hash: Option<String>,
    pub deliberately_disabled: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    NotInstalled,
    NeedsConsent,
    Connected,
    Handwritten,
    Unreadable,
    IntentionallyDisabled,
    Pending,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    InvalidExecutable,
    Unreadable,
    Handwritten,
    Stale,
    WrongConsent,
    TooLarge,
}
pub struct Consent {
    client: Client,
}
impl Consent {
    pub fn from_native_response(client: Client, accepted: bool) -> Option<Self> {
        accepted.then_some(Self { client })
    }
}
pub enum Action {
    Connect(Consent),
    Disconnect,
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn entry(client: Client, executable: &Path) -> std::result::Result<Value, Problem> {
    let command = executable
        .to_str()
        .filter(|s| !s.contains('\0'))
        .filter(|_| executable.is_absolute())
        .ok_or(Problem::InvalidExecutable)?;
    Ok(match client {
        Client::Vscode => json!({"type":"stdio","command":command,"args":[MCP_FLAG]}),
        Client::Opencode => json!({"type":"local","command":[command,MCP_FLAG],"enabled":true}),
        _ => json!({"command":command,"args":[MCP_FLAG]}),
    })
}
pub fn copy_setup(client: Client, executable: &Path) -> std::result::Result<String, Problem> {
    let value = entry(client, executable)?;
    if client == Client::Codex {
        let mut document = toml_edit::DocumentMut::new();
        document["mcp_servers"]["powermove"] = codex_entry(executable)?;
        Ok(document.to_string())
    } else {
        serde_json::to_string_pretty(&json!({(client.key()):{"powermove":value}}))
            .map_err(|_| Problem::Unreadable)
    }
}
fn codex_entry(executable: &Path) -> std::result::Result<toml_edit::Item, Problem> {
    entry(Client::Codex, executable)?;
    let mut table = toml_edit::Table::new();
    table["command"] = toml_edit::value(executable.to_str().unwrap());
    let mut args = toml_edit::Array::new();
    args.push(MCP_FLAG);
    table["args"] = toml_edit::value(args);
    table["startup_timeout_sec"] = toml_edit::value(60);
    Ok(toml_edit::Item::Table(table))
}
fn json_document(text: &str) -> std::result::Result<Value, Problem> {
    if text.len() > 16 * 1024 * 1024 {
        return Err(Problem::TooLarge);
    }
    let document = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|_| Problem::Unreadable)?
    };
    if !document.is_object() {
        return Err(Problem::Unreadable);
    }
    Ok(document)
}
fn current_hash(client: Client, text: &str) -> std::result::Result<Option<String>, Problem> {
    if text.len() > 16 * 1024 * 1024 {
        return Err(Problem::TooLarge);
    }
    if client == Client::Codex {
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| Problem::Unreadable)?;
        let Some(servers) = doc.get("mcp_servers") else {
            return Ok(None);
        };
        if !servers.is_table() {
            return Err(Problem::Unreadable);
        }
        Ok(servers
            .get("powermove")
            .map(|item| digest(&item.to_string())))
    } else {
        let doc = json_document(text)?;
        let Some(servers) = doc.get(client.key()) else {
            return Ok(None);
        };
        if !servers.is_object() {
            return Err(Problem::Unreadable);
        }
        Ok(servers
            .get("powermove")
            .map(|value| digest(&value.to_string())))
    }
}
pub fn classify_claude(
    text: &str,
    original: Option<&Path>,
    native: &Path,
    legacy_added: bool,
) -> super::client_consent::Found {
    use super::client_consent::Found;
    let Ok(doc) = json_document(text) else {
        return Found::Unreadable;
    };
    if doc.get("mcpServers").is_some_and(|s| !s.is_object()) {
        return Found::Unreadable;
    }
    let Some(current) = doc.get("mcpServers").and_then(|s| s.get("powermove")) else {
        return if legacy_added {
            Found::ReceiptButRemoved
        } else {
            Found::Absent
        };
    };
    if entry(Client::ClaudeDesktop, native).is_ok_and(|e| &e == current) {
        return Found::ThisApp;
    }
    if original.is_some_and(|path| entry(Client::ClaudeDesktop, path).is_ok_and(|e| &e == current))
    {
        return Found::OriginalApp;
    }
    Found::Customized
}
/// Read-only adoption of the exact original-app entry, requiring the actual
/// persisted/native or explicit Settings consent before any replacement plan.
pub fn original_switch_receipt(
    text: &str,
    original: &Path,
    consent: &Consent,
) -> std::result::Result<Receipt, Problem> {
    if consent.client != Client::ClaudeDesktop {
        return Err(Problem::WrongConsent);
    }
    let doc = json_document(text)?;
    if doc.get("mcpServers").and_then(|s| s.get("powermove"))
        != Some(&entry(Client::ClaudeDesktop, original)?)
    {
        return Err(Problem::Handwritten);
    }
    Ok(Receipt {
        client: Client::ClaudeDesktop,
        managed_entry_hash: current_hash(Client::ClaudeDesktop, text)?,
        deliberately_disabled: false,
    })
}
pub fn inspect(client: Client, installed: bool, text: &str, receipt: Option<&Receipt>) -> State {
    if !installed {
        return State::NotInstalled;
    }
    match current_hash(client, text) {
        Err(_) => State::Unreadable,
        Ok(Some(hash)) => {
            if receipt
                .is_some_and(|r| r.client == client && r.managed_entry_hash.as_ref() == Some(&hash))
            {
                State::Connected
            } else {
                State::Handwritten
            }
        }
        Ok(None) => {
            if receipt.is_some_and(|r| r.client == client && r.deliberately_disabled) {
                State::IntentionallyDisabled
            } else {
                State::NeedsConsent
            }
        }
    }
}
pub enum Mutation {
    File {
        next: String,
    },
    /// Execute Claude Code's own native CLI without a shell, preserving its
    /// concurrent-session ownership. Host archives first, then re-reads/verifies.
    ClaudeCli {
        commands: Vec<Vec<String>>,
        expected_entry: Value,
    },
    Unchanged,
}
pub struct Change {
    pub client: Client,
    pub expected: String,
    pub mutation: Mutation,
    pub receipt: Receipt,
    /// S070 Settings button intent. Persist this native-profile answer before
    /// changing Claude's file; removal must prevent the launch alert returning.
    pub consent_answer: Option<super::client_consent::Answer>,
}
impl std::fmt::Debug for Mutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::File { .. } => "File([private configuration])",
            Self::ClaudeCli { .. } => "ClaudeCli([private configuration])",
            Self::Unchanged => "Unchanged",
        })
    }
}
impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Change")
            .field("client", &self.client)
            .field("mutation", &self.mutation)
            .field("receipt", &self.receipt)
            .finish_non_exhaustive()
    }
}
pub fn plan(
    client: Client,
    text: &str,
    receipt: Option<&Receipt>,
    action: Action,
    executable: &Path,
) -> std::result::Result<Change, Problem> {
    let current = current_hash(client, text)?;
    let managed = current.as_ref().is_some_and(|hash| {
        receipt.is_some_and(|r| r.client == client && r.managed_entry_hash.as_ref() == Some(hash))
    });
    let connect = match action {
        Action::Connect(consent) => {
            if consent.client != client {
                return Err(Problem::WrongConsent);
            }
            true
        }
        Action::Disconnect => false,
    };
    if current.is_some() && !managed {
        if connect {
            return Ok(Change {
                client,
                expected: text.into(),
                mutation: Mutation::Unchanged,
                receipt: receipt.cloned().unwrap_or(Receipt {
                    client,
                    managed_entry_hash: None,
                    deliberately_disabled: false,
                }),
                consent_answer: None,
            });
        }
        return Err(Problem::Handwritten);
    }
    let mut next_receipt = Receipt {
        client,
        managed_entry_hash: None,
        deliberately_disabled: !connect,
    };
    if client == Client::ClaudeCode {
        let mut commands = Vec::new();
        if current.is_some() {
            commands.push(vec![
                "mcp".into(),
                "remove".into(),
                "--scope".into(),
                "user".into(),
                "powermove".into(),
            ]);
        }
        let expected_entry = entry(client, executable)?;
        if connect {
            commands.push(vec![
                "mcp".into(),
                "add".into(),
                "--scope".into(),
                "user".into(),
                "powermove".into(),
                "--".into(),
                executable.to_str().unwrap().into(),
                MCP_FLAG.into(),
            ]);
            next_receipt.managed_entry_hash = Some(digest(&expected_entry.to_string()));
        }
        return Ok(Change {
            client,
            expected: text.into(),
            mutation: Mutation::ClaudeCli {
                commands,
                expected_entry,
            },
            receipt: next_receipt,
            consent_answer: None,
        });
    }
    let next = if client == Client::Codex {
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| Problem::Unreadable)?;
        if connect {
            document["mcp_servers"]["powermove"] = codex_entry(executable)?;
        } else if let Some(servers) = document
            .get_mut("mcp_servers")
            .and_then(toml_edit::Item::as_table_mut)
        {
            servers.remove("powermove");
        }
        let next = document.to_string();
        next_receipt.managed_entry_hash = current_hash(client, &next)?;
        next
    } else {
        let mut doc = json_document(text)?;
        if connect {
            if doc.get(client.key()).is_none() {
                doc[client.key()] = json!({});
            }
            doc[client.key()]["powermove"] = entry(client, executable)?;
        } else if let Some(servers) = doc.get_mut(client.key()).and_then(Value::as_object_mut) {
            servers.remove("powermove");
        }
        next_receipt.managed_entry_hash = doc
            .get(client.key())
            .and_then(|s| s.get("powermove"))
            .map(|e| digest(&e.to_string()));
        format!(
            "{}\n",
            serde_json::to_string_pretty(&doc).map_err(|_| Problem::Unreadable)?
        )
    };
    Ok(Change {
        client,
        expected: text.into(),
        mutation: Mutation::File { next },
        receipt: next_receipt,
        consent_answer: (client == Client::ClaudeDesktop).then_some(if connect {
            super::client_consent::Answer::Added
        } else {
            super::client_consent::Answer::Declined
        }),
    })
}

pub trait ClientHost: Send + 'static {
    /// Durable native-profile answer before any Claude settings write.
    fn record_claude_consent(&mut self, answer: super::client_consent::Answer) -> Result<()>;
    /// Worker capability: bound no-follow regular-file reads (16 MiB), explicit
    /// user-selected home/config root. Accept canonical root-owned system links
    /// on macOS; refuse app-owned config-path links. Never discover other homes.
    fn read(&mut self, client: Client) -> Result<String>;
    /// CAS the exact expected text; archive every original + old receipt before
    /// atomic durable publication. Claude CLI: no shell, 60 s/1 MiB cap, archive
    /// first and verify final entry after CLI success before issuing its receipt.
    /// A failure retains archives/staging and reports its actual code.
    fn apply(&mut self, change: Change) -> Result<Receipt>;
}
pub struct Work {
    pub ticket: Ticket,
    pub change: Change,
}
pub struct Reply {
    pub ticket: Ticket,
    pub client: Client,
    pub result: Result<Receipt>,
}
impl Work {
    pub fn run(self, host: &mut impl ClientHost) -> Reply {
        let client = self.change.client;
        let result = host.read(client).and_then(|text| {
            if text == self.change.expected {
                if let Some(answer) = self.change.consent_answer {
                    host.record_claude_consent(answer)?;
                }
                host.apply(self.change)
            } else {
                Err(ServiceError::message(
                    ServiceErrorCode::InvalidData,
                    "agent-client settings changed",
                ))
            }
        });
        Reply {
            ticket: self.ticket,
            client,
            result,
        }
    }
}
#[derive(Debug, Default)]
pub struct Controller {
    flight: Flight,
    client: Option<Client>,
    pub receipt: Option<Receipt>,
    pub error: Option<ServiceErrorCode>,
}
impl Controller {
    pub fn is_busy(&self) -> bool {
        self.flight.current().is_some()
    }
    pub fn start(&mut self, change: Change) -> Option<Work> {
        if self.flight.current().is_some() {
            return None;
        }
        self.client = Some(change.client);
        self.error = None;
        Some(Work {
            ticket: self.flight.start(),
            change,
        })
    }
    pub fn receive(&mut self, reply: Reply) -> bool {
        if !self.flight.finish(reply.ticket) || self.client != Some(reply.client) {
            return false;
        }
        match reply.result {
            Ok(receipt) => self.receipt = Some(receipt),
            Err(error) => self.error = Some(error.code),
        }
        true
    }
    pub fn close(&mut self) {
        self.flight.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn executable() -> &'static Path {
        Path::new("/Applications/Powermove.app/Contents/MacOS/Powermove")
    }
    fn connect(client: Client, text: &str, receipt: Option<&Receipt>) -> Change {
        plan(
            client,
            text,
            receipt,
            Action::Connect(Consent::from_native_response(client, true).unwrap()),
            executable(),
        )
        .unwrap()
    }
    fn file(change: &Change) -> &str {
        match &change.mutation {
            Mutation::File { next } => next,
            _ => panic!("expected file plan"),
        }
    }
    #[test]
    fn all_nine_client_shapes_use_native_executable_and_literal_args() {
        assert_eq!(CLIENTS.len(), 9);
        for client in CLIENTS {
            assert!(!Path::new(client.config()).is_absolute());
            let setup = copy_setup(*client, executable()).unwrap();
            assert!(setup.contains(MCP_FLAG));
            assert!(!setup.contains("node"));
        }
        let value: Value =
            serde_json::from_str(&copy_setup(Client::Vscode, executable()).unwrap()).unwrap();
        assert_eq!(value["servers"]["powermove"]["type"], "stdio");
        let value: Value =
            serde_json::from_str(&copy_setup(Client::Opencode, executable()).unwrap()).unwrap();
        assert_eq!(value["mcp"]["powermove"]["command"][1], MCP_FLAG);
    }
    #[test]
    fn explicit_consent_is_required_and_no_launch_registration_exists() {
        assert!(Consent::from_native_response(Client::ClaudeDesktop, false).is_none());
        assert_eq!(
            inspect(Client::ClaudeDesktop, true, "{}", None),
            State::NeedsConsent
        );
        let consent = Consent::from_native_response(Client::Cursor, true).unwrap();
        assert!(matches!(
            plan(
                Client::ClaudeDesktop,
                "{}",
                None,
                Action::Connect(consent),
                executable()
            ),
            Err(Problem::WrongConsent)
        ));
    }
    #[test]
    fn custom_entries_and_commented_json_are_never_rewritten() {
        let text = "{\"mcpServers\":{\"powermove\":{\"command\":\"custom\"}}}";
        assert!(matches!(
            connect(Client::Cursor, text, None).mutation,
            Mutation::Unchanged
        ));
        assert!(matches!(
            plan(Client::Cursor, text, None, Action::Disconnect, executable()),
            Err(Problem::Handwritten)
        ));
        let comments = "{\n// user note\n\"mcpServers\":{}}";
        assert_eq!(
            inspect(Client::Vscode, true, comments, None),
            State::Unreadable
        );
        assert!(matches!(
            plan(
                Client::Vscode,
                comments,
                None,
                Action::Disconnect,
                executable()
            ),
            Err(Problem::Unreadable)
        ));
    }
    #[test]
    fn json_connect_disconnect_preserves_other_entries_and_intentional_removal() {
        let text = "{\"mcpServers\":{\"mine\":{\"command\":\"keep\"}},\"future\":1}";
        let connected = connect(Client::ClaudeDesktop, text, None);
        let next = file(&connected);
        assert_eq!(
            inspect(Client::ClaudeDesktop, true, next, Some(&connected.receipt)),
            State::Connected
        );
        let value: Value = serde_json::from_str(next).unwrap();
        assert_eq!(value["future"], 1);
        assert_eq!(value["mcpServers"]["mine"]["command"], "keep");
        let removed = plan(
            Client::ClaudeDesktop,
            next,
            Some(&connected.receipt),
            Action::Disconnect,
            executable(),
        )
        .unwrap();
        assert!(removed.receipt.deliberately_disabled);
        assert_eq!(
            inspect(
                Client::ClaudeDesktop,
                true,
                file(&removed),
                Some(&removed.receipt)
            ),
            State::IntentionallyDisabled
        );
    }
    #[test]
    fn codex_edit_preserves_comments_and_other_tables_and_refuses_changed_entry() {
        let text = "# handwritten header\nmodel = \"m\"\n\n[mcp_servers.mine]\ncommand = \"keep\" # handwritten\n";
        let connected = connect(Client::Codex, text, None);
        let next = file(&connected);
        assert!(next.contains(text.trim_end()));
        let removed = plan(
            Client::Codex,
            next,
            Some(&connected.receipt),
            Action::Disconnect,
            executable(),
        )
        .unwrap();
        assert!(!file(&removed).contains("--powermove-mcp"));
        assert!(file(&removed).contains("# handwritten"));
        let changed = next.replace("startup_timeout_sec = 60", "startup_timeout_sec = 90");
        assert_eq!(
            inspect(Client::Codex, true, &changed, Some(&connected.receipt)),
            State::Handwritten
        );
    }
    #[test]
    fn claude_code_uses_its_own_cli_without_a_shell() {
        let change = connect(Client::ClaudeCode, "{}", None);
        let Mutation::ClaudeCli { commands, .. } = change.mutation else {
            panic!("expected CLI");
        };
        assert_eq!(
            commands[0],
            vec![
                "mcp",
                "add",
                "--scope",
                "user",
                "powermove",
                "--",
                executable().to_str().unwrap(),
                MCP_FLAG
            ]
        );
    }
    #[test]
    fn archive_failure_and_stale_config_never_publish_a_receipt() {
        struct Host {
            text: String,
            archive: Vec<String>,
            fail: bool,
        }
        impl ClientHost for Host {
            fn record_claude_consent(
                &mut self,
                _: super::super::client_consent::Answer,
            ) -> Result<()> {
                Ok(())
            }
            fn read(&mut self, _: Client) -> Result<String> {
                Ok(self.text.clone())
            }
            fn apply(&mut self, change: Change) -> Result<Receipt> {
                self.archive.push(self.text.clone());
                if self.fail {
                    return Err(ServiceError::message(
                        ServiceErrorCode::DiskFull,
                        "fixture archive failure",
                    ));
                }
                self.text = file(&change).into();
                Ok(change.receipt)
            }
        }
        let mut controller = Controller::default();
        let work = controller
            .start(connect(Client::Cursor, "{}", None))
            .unwrap();
        let mut host = Host {
            text: "{\"changed\":1}".into(),
            archive: vec![],
            fail: false,
        };
        assert!(controller.receive(work.run(&mut host)));
        assert!(controller.receipt.is_none());
        assert!(host.archive.is_empty());
        let work = controller
            .start(connect(Client::Cursor, "{}", None))
            .unwrap();
        host.text = "{}".into();
        host.fail = true;
        controller.receive(work.run(&mut host));
        assert_eq!(host.text, "{}");
        assert!(controller.receipt.is_none());
        assert_eq!(controller.error, Some(ServiceErrorCode::DiskFull));
    }
    #[test]
    fn original_app_switch_requires_exact_entry_and_consumes_explicit_consent() {
        use super::super::client_consent::{Answer, Found};
        let original = Path::new("/Applications/Powermove Original.app/Contents/MacOS/Powermove");
        let text = copy_setup(Client::ClaudeDesktop, original).unwrap();
        assert_eq!(
            classify_claude(&text, Some(original), executable(), false),
            Found::OriginalApp
        );
        assert_eq!(
            classify_claude("{}", Some(original), executable(), true),
            Found::ReceiptButRemoved
        );
        let permit = Consent::from_native_response(Client::ClaudeDesktop, true).unwrap();
        let receipt = original_switch_receipt(&text, original, &permit).unwrap();
        let change = plan(
            Client::ClaudeDesktop,
            &text,
            Some(&receipt),
            Action::Connect(permit),
            executable(),
        )
        .unwrap();
        assert_eq!(change.consent_answer, Some(Answer::Added));
        assert_eq!(
            classify_claude(file(&change), Some(original), executable(), false),
            Found::ThisApp
        );
        let removed = plan(
            Client::ClaudeDesktop,
            file(&change),
            Some(&change.receipt),
            Action::Disconnect,
            executable(),
        )
        .unwrap();
        assert_eq!(removed.consent_answer, Some(Answer::Declined));
        let mut custom: Value = serde_json::from_str(&text).unwrap();
        custom["mcpServers"]["powermove"]["env"] = json!({"custom":"kept"});
        assert_eq!(
            classify_claude(&custom.to_string(), Some(original), executable(), false),
            Found::Customized
        );
    }
    #[test]
    fn worker_cannot_modify_claude_before_durable_consent() {
        struct Host {
            calls: Vec<&'static str>,
            fail: bool,
        }
        impl ClientHost for Host {
            fn read(&mut self, _: Client) -> Result<String> {
                Ok("{}".into())
            }
            fn record_claude_consent(
                &mut self,
                _: super::super::client_consent::Answer,
            ) -> Result<()> {
                self.calls.push("consent");
                if self.fail {
                    Err(ServiceError::message(
                        ServiceErrorCode::DiskFull,
                        "fixture consent save failed",
                    ))
                } else {
                    Ok(())
                }
            }
            fn apply(&mut self, change: Change) -> Result<Receipt> {
                self.calls.push("settings");
                Ok(change.receipt)
            }
        }
        for fail in [true, false] {
            let mut controller = Controller::default();
            let work = controller
                .start(connect(Client::ClaudeDesktop, "{}", None))
                .unwrap();
            let mut host = Host {
                calls: vec![],
                fail,
            };
            controller.receive(work.run(&mut host));
            assert_eq!(
                host.calls,
                if fail {
                    vec!["consent"]
                } else {
                    vec!["consent", "settings"]
                }
            );
            assert_eq!(controller.receipt.is_some(), !fail);
        }
    }
}
