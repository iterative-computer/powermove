//! Model-stream recordings and their replay. A recording holds every request a
//! harness sent to its model provider and the streamed (SSE) response it got,
//! normalised as `crates/agents/tests/replay` normalises its captures:
//! generated working folders become `/workspace/agent`, object key order is
//! insignificant, and everything else (ids, array order, text, null versus
//! absent) stays exact. The few volatile fields a provider protocol carries
//! (a per-run cache key) are listed by pointer in the recording itself, so
//! nothing is dropped silently. Credentials and headers are never recorded.
pub mod http;
pub mod recorder;
pub mod replayer;
pub mod sse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const FORMAT: &str = "aero-eval-recording";
/// The folder generated working paths are rewritten to (as in agents replay).
pub const WORKSPACE: &str = "/workspace/agent";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub format: String,
    pub version: u32,
    /// True for hand-written or generated streams; false only for recordings
    /// of approved real-model batches.
    pub synthetic: bool,
    /// Wire protocol: `openai-responses`, `anthropic-messages` or `other`.
    pub provider: String,
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub task: String,
    #[serde(default)]
    pub normalise: Normalise,
    pub exchanges: Vec<Exchange>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Normalise {
    /// JSON pointers into request bodies whose values change every run; they
    /// compare as present-or-absent only.
    pub volatile: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub request: Request,
    pub response: Response,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub method: String,
    /// Path and query, without the origin.
    pub path: String,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub status: u16,
    /// A streamed response: its server-sent events in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<sse::Event>,
    /// A plain JSON response (when `events` is empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl Recording {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let r: Recording =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        r.validate()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(r)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT || self.version != 1 {
            return Err(format!(
                "not an {FORMAT} v1 file ({} v{})",
                self.format, self.version
            ));
        }
        if self.exchanges.is_empty() {
            return Err("a recording needs at least one exchange".into());
        }
        for p in &self.normalise.volatile {
            if !p.starts_with('/') {
                return Err(format!("volatile entry {p} is not a JSON pointer"));
            }
        }
        let text = serde_json::to_string(self).unwrap_or_default();
        for leak in ["/Users/", "/home/", "/private/var/", "Bearer "] {
            if text.contains(leak) {
                return Err(format!(
                    "recording contains \"{leak}\": normalise paths and never record credentials"
                ));
            }
        }
        if looks_like_key(&text) {
            return Err("recording contains something shaped like an API key".into());
        }
        Ok(())
    }
}

/// `sk-…` / `sk-ant-…` style secrets: the prefix at a token start followed by
/// at least 20 key characters.
fn looks_like_key(text: &str) -> bool {
    let b = text.as_bytes();
    text.match_indices("sk-").any(|(i, _)| {
        let starts_token = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let run = b[i + 3..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'-' || **c == b'_')
            .count();
        starts_token && run >= 20
    })
}

/// Rewrite every occurrence of the given working folders in strings to
/// [`WORKSPACE`] (longest first, so nested folders map cleanly).
pub fn normalise_paths(v: &mut Value, folders: &[String]) {
    let mut folders: Vec<&String> = folders.iter().filter(|f| !f.is_empty()).collect();
    folders.sort_by_key(|f| std::cmp::Reverse(f.len()));
    fn walk(v: &mut Value, folders: &[&String]) {
        match v {
            Value::String(s) => {
                for f in folders {
                    if s.contains(f.as_str()) {
                        *s = s.replace(f.as_str(), WORKSPACE);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, folders)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, folders)),
            _ => {}
        }
    }
    walk(v, &folders);
}

/// Replace volatile values with a marker so they compare by presence only.
pub fn mask_volatile(v: &mut Value, pointers: &[String]) {
    for p in pointers {
        if let Some(x) = v.pointer_mut(p) {
            *x = Value::String("<volatile>".into());
        }
    }
}

/// One differing leaf; `None` is absent (distinct from JSON null).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    pub path: String,
    pub expected: Option<Value>,
    pub actual: Option<Value>,
}

/// Exact JSON comparison, object key order insignificant, every differing leaf
/// reported (the agents replay harness's rule).
pub fn diff(expected: &Value, actual: &Value) -> Vec<Difference> {
    fn walk(path: String, a: Option<&Value>, b: Option<&Value>, out: &mut Vec<Difference>) {
        if a == b {
            return;
        }
        match (a, b) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
                for k in keys {
                    walk(
                        format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                        a.get(k),
                        b.get(k),
                        out,
                    );
                }
            }
            (Some(Value::Array(a)), Some(Value::Array(b))) => {
                for i in 0..a.len().max(b.len()) {
                    walk(format!("{path}/{i}"), a.get(i), b.get(i), out);
                }
            }
            _ => out.push(Difference {
                path,
                expected: a.cloned(),
                actual: b.cloned(),
            }),
        }
    }
    let mut out = Vec::new();
    walk(String::new(), Some(expected), Some(actual), &mut out);
    out
}
