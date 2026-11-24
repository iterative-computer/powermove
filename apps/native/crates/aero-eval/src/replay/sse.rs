//! Server-sent events: parse a stream into events and write them back.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The data, parsed when it is JSON (so diffs and reviews read it) …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// … or verbatim when it is not (e.g. `[DONE]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Event {
    pub fn json(event: Option<&str>, data: Value) -> Self {
        Self {
            event: event.map(str::to_owned),
            data: Some(data),
            text: None,
        }
    }
    fn from_parts(event: Option<String>, data: String) -> Self {
        match serde_json::from_str::<Value>(&data) {
            Ok(v) if v.is_object() || v.is_array() => Self {
                event,
                data: Some(v),
                text: None,
            },
            _ => Self {
                event,
                data: None,
                text: Some(data),
            },
        }
    }
    pub fn data_string(&self) -> String {
        match (&self.data, &self.text) {
            (Some(v), _) => v.to_string(),
            (None, Some(t)) => t.clone(),
            (None, None) => String::new(),
        }
    }
    pub fn write(&self, out: &mut Vec<u8>) {
        if let Some(e) = &self.event {
            out.extend_from_slice(format!("event: {e}\n").as_bytes());
        }
        for line in self.data_string().split('\n') {
            out.extend_from_slice(format!("data: {line}\n").as_bytes());
        }
        out.push(b'\n');
    }
}

/// Incremental parser: feed bytes, take complete events.
#[derive(Default)]
pub struct Parser {
    buf: String,
    event: Option<String>,
    data: Vec<String>,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(i) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=i).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() || self.event.is_some() {
                    out.push(Event::from_parts(
                        self.event.take(),
                        std::mem::take(&mut self.data).join("\n"),
                    ));
                }
            } else if line.starts_with(':') {
                // comment / keep-alive
            } else if let Some(v) = field(line, "event") {
                self.event = Some(v.to_owned());
            } else if let Some(v) = field(line, "data") {
                self.data.push(v.to_owned());
            }
        }
        out
    }
    /// An unterminated final event, if the stream ended without a blank line.
    pub fn finish(&mut self) -> Option<Event> {
        let rest = std::mem::take(&mut self.buf);
        let mut tail = self.feed(format!("{rest}\n\n").as_bytes());
        tail.pop()
    }
}

fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

pub fn parse(bytes: &[u8]) -> Vec<Event> {
    let mut p = Parser::default();
    let mut out = p.feed(bytes);
    out.extend(p.finish());
    out
}
