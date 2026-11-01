//! Headless agent services. All runtime and disk work belongs on workers.
//! OAuth homes are opaque runtime-owned paths, never read or modified here.
pub mod approvals;
pub mod claude;
pub mod codex;
pub mod codex_exec;
pub mod conversation;
pub mod protocol;
pub mod trace;
pub mod transport;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Editor,
    Project,
    Computer,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMode {
    Supervised,
    #[default]
    Auto,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Chatgpt,
    Claude,
    Compatible,
}
impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
            Self::Claude => "claude",
            Self::Compatible => "compatible",
        }
    }
}

/// Public result shape emitted by the clients (shared/ipc.ts).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunResult {
    Success {
        ok: bool,
        text: String,
        access: Access,
    },
    Failure {
        ok: bool,
        error: String,
        cancelled: bool,
    },
}
impl RunResult {
    pub fn success(text: String, access: Access) -> Self {
        Self::Success {
            ok: true,
            text,
            access,
        }
    }
    pub fn failure(error: impl Into<String>, cancelled: bool) -> Self {
        Self::Failure {
            ok: false,
            error: error.into(),
            cancelled,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Ready,
    Trace { run: String, value: Value },
    Progress { run: String, text: String },
    Session { run: String, id: String },
    Completed { run: String, result: RunResult },
    Notification(Value),
    Warning(String),
    Stopped(String),
}
#[derive(Default, Debug)]
pub struct Effects {
    pub outgoing: Vec<Value>,
    pub events: Vec<Event>,
    pub close_stdin: bool,
}
impl Effects {
    pub fn trace(&mut self, run: &str, value: Value) {
        self.events.push(Event::Trace {
            run: run.into(),
            value,
        });
    }
    pub fn progress(&mut self, run: &str, text: impl Into<String>) {
        self.events.push(Event::Progress {
            run: run.into(),
            text: text.into(),
        });
    }
    pub fn append(&mut self, mut next: Self) {
        self.outgoing.append(&mut next.outgoing);
        self.events.append(&mut next.events);
        self.close_stdin |= next.close_stdin;
    }
}
pub(crate) fn valid_id(s: &str, limit: usize) -> bool {
    !s.is_empty()
        && s.len() <= limit
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
pub(crate) fn error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}
pub(crate) fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}
