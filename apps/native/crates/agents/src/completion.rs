//! Bounded compatible-provider SSE decoder. Bytes and tool-call ordering are
//! independent of HTTP chunk boundaries; the worker owns request cancellation.
use crate::error;
use serde_json::{Value, json};
use std::io;
const INVALID: &str =
    "The provider returned an invalid response stream. Your conversation is kept; try again.";
const INCOMPLETE: &str =
    "The connection ended before the model finished. Your conversation is kept; try again.";
#[derive(Default)]
pub struct Completion {
    buffer: Vec<u8>,
    content: String,
    pending: String,
    bytes: usize,
    finished: bool,
    calls: Vec<(u64, Value)>,
}
impl Completion {
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<String>> {
        if self.finished {
            return Ok(vec![]);
        }
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > 2_000_000 {
            return Err(error(
                "The model response is too large. Try a smaller request.",
            ));
        }
        self.buffer.extend_from_slice(bytes);
        let mut events = vec![];
        while let Some(end) = self.buffer.iter().position(|c| *c == b'\n') {
            let line = String::from_utf8_lossy(&self.buffer[..end])
                .trim_end_matches('\r')
                .to_owned();
            self.buffer.drain(..end + 1);
            self.accept(&line, &mut events)?;
        }
        Ok(events)
    }
    fn accept(&mut self, line: &str, events: &mut Vec<String>) -> io::Result<()> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data.is_empty() {
            return Ok(());
        }
        if data.trim() == "[DONE]" {
            if !self.pending.is_empty() {
                return Err(error(INVALID));
            }
            self.finished = true;
            return Ok(());
        }
        let candidates = if self.pending.is_empty() {
            vec![data.to_owned()]
        } else {
            vec![
                format!("{}{data}", self.pending),
                format!("{}\n{data}", self.pending),
            ]
        };
        let mut value = None;
        let mut incomplete = false;
        for candidate in &candidates {
            match serde_json::from_str::<Value>(candidate) {
                Ok(v) => {
                    value = Some(v);
                    break;
                }
                Err(e) => incomplete |= e.is_eof(),
            }
        }
        let Some(event) = value else {
            if incomplete {
                self.pending = candidates[0].clone();
                return Ok(());
            }
            return Err(error(INVALID));
        };
        self.pending.clear();
        if !event.is_object() {
            return Err(error(INVALID));
        }
        if event
            .get("error")
            .is_some_and(|v| !v.is_null() && v != false && v != 0 && v != "")
        {
            return Err(error(
                "The provider interrupted the response. Your conversation is kept; try again.",
            ));
        }
        let choice = &event["choices"][0];
        if choice["finish_reason"] == "length" {
            return Err(error(
                "The model reached its output limit. Try a smaller request.",
            ));
        }
        if choice["finish_reason"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        {
            self.finished = true;
        }
        let delta = &choice["delta"];
        if let Some(content) = delta["content"].as_str() {
            self.content.push_str(content);
            events.push(content.into());
        }
        for part in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = part["index"]
                .as_u64()
                .filter(|i| *i < 80)
                .ok_or_else(|| error("Invalid tool response from model."))?;
            let position = self
                .calls
                .iter()
                .position(|(i, _)| *i == index)
                .unwrap_or_else(|| {
                    self.calls.push((
                        index,
                        json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                    ));
                    self.calls.len() - 1
                });
            let tool = &mut self.calls[position].1;
            if let Some(id) = part["id"].as_str().filter(|v| !v.is_empty()) {
                tool["id"] = json!(id);
            }
            for key in ["name", "arguments"] {
                if let Some(value) = part["function"][key].as_str().filter(|v| !v.is_empty()) {
                    let current = tool["function"][key].as_str().unwrap();
                    tool["function"][key] = json!(format!("{current}{value}"));
                }
            }
        }
        Ok(())
    }
    pub fn finish(mut self) -> io::Result<(Value, Vec<String>)> {
        let mut events = vec![];
        let tail = String::from_utf8_lossy(&self.buffer).into_owned();
        if !tail.trim().is_empty() {
            self.accept(tail.trim_end_matches('\r'), &mut events)?;
        }
        if !self.pending.is_empty() || !self.finished {
            return Err(error(INCOMPLETE));
        }
        let mut result = json!({"content":self.content});
        if !self.calls.is_empty() {
            if self.calls.iter().any(|(_, c)| {
                c["id"].as_str().is_none_or(|s| s.is_empty())
                    || c["function"]["name"].as_str().is_none_or(|s| s.is_empty())
            }) {
                return Err(error("The model returned an incomplete tool call."));
            }
            result["tool_calls"] =
                json!(self.calls.into_iter().map(|(_, v)| v).collect::<Vec<_>>());
        }
        Ok((result, events))
    }
}
