//! Model-stream recordings and their replay. A recording holds every request a
//! harness sent to its model provider and the streamed (SSE) response it got,
//! normalised as `crates/agents/tests/replay` normalises its captures:
//! generated working folders become `/workspace/agent`, object key order is
//! insignificant, and everything else (ids, array order, text, null versus
//! absent) stays exact. The few volatile fields a provider protocol carries
//! (a per-run cache key) are listed by pointer in the recording itself, so
//! nothing is dropped silently. Headers are never recorded; only the
//! provider's known paths and query keys are; and a recording is validated
//! for credential fields, key-shaped strings and home paths whenever it is
//! loaded, finished or saved. A real recording also needs an explicit
//! [`Review`] before it may enter the public repo ([`Recording::validate_for_repo`]).
pub mod hold;
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
    /// Who read a real recording line by line before it was committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub by: String,
    /// ISO date.
    pub at: String,
    pub note: String,
}

/// Wire protocols a recording may hold, with the request paths and query
/// keys each one may record. Anything else is refused.
pub const PROVIDERS: [(&str, &[&str], &[&str]); 2] = [
    ("openai-responses", &["/v1/responses", "/responses"], &[]),
    (
        "anthropic-messages",
        &["/v1/messages", "/v1/messages/count_tokens"],
        &["beta"],
    ),
];

/// The path as it may be recorded: a known path of the provider, with only
/// its allowed query keys (short plain values), in order. Every other query
/// pair is dropped, so a credential in a query string is never written.
pub fn recorded_path(provider: &str, path: &str) -> Result<String, String> {
    let (_, paths, keys) = PROVIDERS
        .iter()
        .find(|(p, _, _)| *p == provider)
        .ok_or_else(|| format!("unknown provider {provider}"))?;
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    if !paths.contains(&base) {
        return Err(format!("{provider} never records the path {base}"));
    }
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            keys.contains(&k)
                && v.len() <= 32
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
        .collect();
    Ok(if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    })
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
    /// Validates first: an unvalidated recording is never written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()
            .map_err(|e| format!("{}: not saved: {e}", path.display()))?;
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
        if let Some(r) = &self.review
            && (r.by.trim().is_empty() || r.at.trim().is_empty())
        {
            return Err("a review names who read the recording and when".into());
        }
        for (i, e) in self.exchanges.iter().enumerate() {
            let at = |m: String| format!("exchange {i}: {m}");
            if !matches!(e.request.method.as_str(), "POST" | "GET") {
                return Err(at(format!("method {} is not recorded", e.request.method)));
            }
            let allowed = recorded_path(&self.provider, &e.request.path).map_err(at)?;
            if allowed != e.request.path {
                return Err(at(format!(
                    "path {} carries query keys the provider never records",
                    e.request.path
                )));
            }
            scan(&e.request.body, &format!("/exchanges/{i}/request/body")).map_err(at)?;
            for (j, ev) in e.response.events.iter().enumerate() {
                let p = format!("/exchanges/{i}/response/events/{j}");
                for s in [&ev.event, &ev.text].into_iter().flatten() {
                    scan(&Value::String(s.clone()), &p).map_err(at)?;
                }
                if let Some(d) = &ev.data {
                    scan(d, &p).map_err(at)?;
                }
            }
            if let Some(b) = &e.response.body {
                scan(b, &format!("/exchanges/{i}/response/body")).map_err(at)?;
            }
        }
        for s in [&self.harness, &self.model, &self.effort, &self.task] {
            scan(&Value::String(s.clone()), "/").map_err(|e| format!("metadata: {e}"))?;
        }
        Ok(())
    }

    /// The rule for `data/replay/` in the public repo: synthetic, or a real
    /// recording someone reviewed.
    pub fn validate_for_repo(&self) -> Result<(), String> {
        self.validate()?;
        if !self.synthetic && self.review.is_none() {
            return Err("a real recording needs an explicit review before it is committed".into());
        }
        Ok(())
    }
}

/// Object keys that carry credentials, compared without case, `-` or `_`.
const CREDENTIAL_KEYS: [&str; 20] = [
    "apikey",
    "xapikey",
    "authorization",
    "proxyauthorization",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "sessiontoken",
    "authtoken",
    "bearer",
    "password",
    "passwd",
    "secret",
    "clientsecret",
    "cookie",
    "setcookie",
    "userid",
    "accountid",
    "organizationid",
    "email",
];

/// Text that must never appear in a recording: home and temp folders on
/// every platform, auth schemes and private keys.
const LEAKS: [&str; 11] = [
    "/users/",
    "/home/",
    "/private/var/",
    "/var/folders/",
    "c:\\users\\",
    "c:/users/",
    "\\users\\",
    "%userprofile%",
    "bearer ",
    "-----begin",
    "/root/",
];

fn scan(v: &Value, at: &str) -> Result<(), String> {
    match v {
        Value::String(s) => {
            let lower = s.to_lowercase();
            if let Some(leak) = LEAKS.iter().find(|l| lower.contains(*l)) {
                return Err(format!(
                    "{at} contains \"{}\": normalise paths and never record credentials",
                    leak.trim()
                ));
            }
            if looks_like_key(s) {
                return Err(format!("{at} contains something shaped like an API key"));
            }
            Ok(())
        }
        Value::Array(a) => a
            .iter()
            .enumerate()
            .try_for_each(|(i, x)| scan(x, &format!("{at}/{i}"))),
        Value::Object(m) => m.iter().try_for_each(|(k, x)| {
            let norm: String = k
                .to_lowercase()
                .chars()
                .filter(|c| !matches!(c, '-' | '_'))
                .collect();
            let filled = !matches!(x, Value::Null) && x.as_str() != Some("");
            if filled && CREDENTIAL_KEYS.contains(&norm.as_str()) {
                return Err(format!("{at}/{k} is a credential field"));
            }
            scan(&Value::String(k.clone()), at)?;
            scan(x, &format!("{at}/{k}"))
        }),
        _ => Ok(()),
    }
}

/// Provider secrets by shape: `sk-…`/`sk-ant-…`, AWS, GitHub, Slack and
/// Google keys, at a token start, followed by enough key characters.
fn looks_like_key(text: &str) -> bool {
    let b = text.as_bytes();
    let key_run = |from: usize| {
        b[from..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'-' || **c == b'_')
            .count()
    };
    let prefixed = |prefix: &str, min: usize| {
        text.match_indices(prefix).any(|(i, _)| {
            (i == 0 || !b[i - 1].is_ascii_alphanumeric()) && key_run(i + prefix.len()) >= min
        })
    };
    prefixed("sk-", 20)
        || prefixed("AKIA", 16)
        || prefixed("ghp_", 30)
        || prefixed("gho_", 30)
        || prefixed("github_pat_", 30)
        || prefixed("xoxb-", 20)
        || prefixed("xoxp-", 20)
        || prefixed("AIza", 30)
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
