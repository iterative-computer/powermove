//! Binary-safe framing shared with `apps/desktop/src/shared/wire.ts`.
//!
//! A frame is `u32 header length | header (UTF-8 JSON) | (u32 length | bytes)*`,
//! all lengths big-endian. Byte arrays inside the header are replaced by
//! `{"$bin": <index>}` placeholders that point into the trailing blob table.

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use serde_json::{Value, json};

const BIN_KEY: &str = "$bin";

/// JSON with its byte attachments. `json` may hold `{"$bin": n}` nodes.
#[derive(Debug, Clone, Default)]
pub struct Payload {
    pub json: Value,
    pub blobs: Vec<Bytes>,
}

impl Payload {
    pub fn json(json: Value) -> Self {
        Self { json, blobs: Vec::new() }
    }

    /// Attach bytes and return the placeholder to put in the JSON.
    pub fn attach(&mut self, bytes: Bytes) -> Value {
        self.blobs.push(bytes);
        json!({ BIN_KEY: self.blobs.len() - 1 })
    }

    /// A payload that is just one byte array (e.g. a read result).
    pub fn bytes(bytes: Bytes) -> Self {
        let mut payload = Self::default();
        payload.json = payload.attach(bytes);
        payload
    }
}

/// Resolve a `{"$bin": n}` placeholder against a blob table.
pub fn blob<'a>(value: &Value, blobs: &'a [Bytes]) -> Option<&'a Bytes> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let index = object.get(BIN_KEY)?.as_u64()? as usize;
    blobs.get(index)
}

pub fn encode(payload: &Payload) -> Vec<u8> {
    let header = serde_json::to_vec(&payload.json).expect("serde_json::Value always serializes");
    let total = 4 + header.len() + payload.blobs.iter().map(|blob| 4 + blob.len()).sum::<usize>();
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&header);
    for blob in &payload.blobs {
        frame.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        frame.extend_from_slice(blob);
    }
    frame
}

pub fn decode(frame: Bytes) -> Result<Payload> {
    let read_u32 = |offset: usize| -> Result<usize> {
        let slice = frame.get(offset..offset + 4).context("wire: truncated frame")?;
        Ok(u32::from_be_bytes(slice.try_into().unwrap()) as usize)
    };
    let header_length = read_u32(0)?;
    let mut offset = 4;
    let header = frame.get(offset..offset + header_length).context("wire: truncated header")?;
    let json: Value = serde_json::from_slice(header).context("wire: invalid header")?;
    offset += header_length;
    let mut blobs = Vec::new();
    while offset < frame.len() {
        let length = read_u32(offset).context("wire: truncated blob table")?;
        offset += 4;
        if offset + length > frame.len() {
            bail!("wire: truncated blob");
        }
        blobs.push(frame.slice(offset..offset + length));
        offset += length;
    }
    Ok(Payload { json, blobs })
}

/// Browser → host message, mirroring `ClientMessage` in wire.ts.
#[derive(Debug)]
pub enum ClientMessage {
    Invoke { id: u64, channel: String, args: Vec<Value> },
    Send { channel: String, args: Vec<Value> },
    Sync { id: u64, channel: String, args: Vec<Value> },
    Answer { id: u64, value: Value },
}

impl ClientMessage {
    pub fn parse(json: Value) -> Result<Self> {
        let Value::Object(mut map) = json else { bail!("wire: message is not an object") };
        let kind = map.get("t").and_then(Value::as_str).unwrap_or_default().to_owned();
        let id = map.get("id").and_then(Value::as_u64);
        let channel = map.get("ch").and_then(Value::as_str).map(str::to_owned);
        let args = match map.remove("args") {
            Some(Value::Array(args)) => Some(args),
            _ => None,
        };
        Ok(match (kind.as_str(), id, channel, args) {
            ("invoke", Some(id), Some(channel), Some(args)) => Self::Invoke { id, channel, args },
            ("sync", Some(id), Some(channel), Some(args)) => Self::Sync { id, channel, args },
            ("send", _, Some(channel), Some(args)) => Self::Send { channel, args },
            ("answer", Some(id), _, _) => Self::Answer { id, value: map.remove("value").unwrap_or(Value::Null) },
            _ => bail!("wire: unknown message"),
        })
    }
}

/// Host → browser messages, mirroring `ServerMessage` in wire.ts. The JSON
/// keeps any `$bin` placeholders from the payload it wraps.
pub fn result_ok(id: u64, value: Payload) -> Payload {
    wrap(value, |json| json!({ "t": "result", "id": id, "ok": true, "value": json }))
}

pub fn result_err(id: u64, error: &str) -> Payload {
    Payload::json(json!({ "t": "result", "id": id, "ok": false, "error": error }))
}

pub fn event(channel: &str, args: Payload) -> Payload {
    wrap(args, |json| json!({ "t": "event", "ch": channel, "args": as_args(json) }))
}

pub fn ask(id: u64, channel: &str, args: Payload) -> Payload {
    wrap(args, |json| json!({ "t": "ask", "id": id, "ch": channel, "args": as_args(json) }))
}

fn as_args(json: Value) -> Value {
    match json {
        Value::Array(_) => json,
        Value::Null => Value::Array(Vec::new()),
        other => Value::Array(vec![other]),
    }
}

fn wrap(payload: Payload, build: impl FnOnce(Value) -> Value) -> Payload {
    Payload { json: build(payload.json), blobs: payload.blobs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bytes_through_placeholders() {
        let mut payload = Payload::default();
        let data = payload.attach(Bytes::from_static(b"\x00\x01\x02"));
        payload.json = json!({ "token": "a", "data": data });
        let decoded = decode(Bytes::from(encode(&payload))).unwrap();
        assert_eq!(decoded.json["token"], "a");
        assert_eq!(blob(&decoded.json["data"], &decoded.blobs).unwrap().as_ref(), b"\x00\x01\x02");
    }

    #[test]
    fn parses_client_messages() {
        let message = ClientMessage::parse(json!({ "t": "invoke", "id": 3, "ch": "app:ping", "args": [] })).unwrap();
        assert!(matches!(message, ClientMessage::Invoke { id: 3, ref channel, .. } if channel == "app:ping"));
    }
}
