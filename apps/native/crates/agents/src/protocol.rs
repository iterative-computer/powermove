//! Wire envelopes follow `codex app-server generate-ts` (no jsonrpc field).
//! Params remain JSON at the boundary to preserve experimental fields exactly.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub id: RequestId,
    pub params: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UserInput {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "localImage")]
    LocalImage { path: String },
}
pub fn initialize() -> Value {
    json!({"clientInfo":{"name":"powermove_agent","title":"Powermove Agent","version":"1.0.0"},"capabilities":{"experimentalApi":true,"requestAttestation":false}})
}
pub fn method_is(method: &str, suffix: &str) -> bool {
    method == suffix
        || method
            .strip_suffix(suffix)
            .is_some_and(|s| s.ends_with('/'))
}
pub fn normalized_type(v: &Value) -> String {
    v.as_str().unwrap_or("").replace('_', "").to_lowercase()
}

/// Byte framing survives arbitrary UTF-8 chunk boundaries and discards an entire
/// oversized line (never interprets its suffix as a fresh permission request).
pub struct Lines {
    pending: Vec<u8>,
    dropping: bool,
    limit: usize,
}
impl Lines {
    pub fn new(limit: usize) -> Self {
        Self {
            pending: vec![],
            dropping: false,
            limit,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Value, String>> {
        let mut out = vec![];
        for &b in bytes {
            if b == b'\n' {
                if self.dropping {
                    self.dropping = false;
                    out.push(Err("Runtime emitted an oversized event".into()));
                } else if let Some(v) = self.parse() {
                    out.push(Ok(v));
                }
                self.pending.clear();
            } else if !self.dropping {
                self.pending.push(b);
                if self.pending.len() > self.limit {
                    self.pending.clear();
                    self.dropping = true;
                }
            }
        }
        out
    }
    fn parse(&self) -> Option<Value> {
        serde_json::from_slice::<Value>(&self.pending)
            .ok()
            .filter(Value::is_object)
    }
    pub fn finish(&mut self) -> Vec<Result<Value, String>> {
        if self.pending.is_empty() && !self.dropping {
            vec![]
        } else {
            self.push(b"\n")
        }
    }
}
