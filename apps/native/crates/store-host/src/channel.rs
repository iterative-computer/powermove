//! Transport-neutral protocol, authenticated identity comes from the host's
//! capability-bound connection, never a JSON sender/origin assertion.
use crate::{Error, Result, identity::ExtensionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

pub const PROTOCOL_VERSION: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    View,
    Realm,
    Kernel,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Ready {
        origin: String,
    },
    Input {
        node: u64,
        event: String,
        data: Value,
    },
    PaintAck {
        revision: u64,
    },
    DomBatch {
        revision: u64,
        operations: Value,
    },
    Snapshot {
        revision: u64,
        document: Value,
    },
    ServiceCall {
        request: u64,
        method: String,
        args: Value,
    },
    ServiceReply {
        request: u64,
        result: WireResult,
    },
    KernelEvent {
        revision: u64,
        event: String,
        data: Value,
    },
    Failure {
        code: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireResult {
    Ok { value: Value },
    Error { code: String, message: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub extension: ExtensionId,
    pub view: String,
    pub generation: u64,
    pub sequence: u64,
    pub from: Endpoint,
    pub to: Endpoint,
    pub message: Message,
}
#[derive(Debug, Clone, Copy)]
pub struct ChannelLimits {
    pub message_bytes: usize,
    pub queued_bytes: usize,
    pub queued_messages: usize,
}
impl Default for ChannelLimits {
    fn default() -> Self {
        Self {
            message_bytes: 1 << 20,
            queued_bytes: 8 << 20,
            queued_messages: 256,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receipt {
    Queued,
    Duplicate { completed: bool },
}
#[derive(Debug)]
pub struct Channel {
    extension: ExtensionId,
    view: String,
    generation: u64,
    limits: ChannelLimits,
    admitted: BTreeMap<Endpoint, u64>,
    completed: BTreeMap<Endpoint, (u64, [u8; 32])>,
    queue: VecDeque<(Envelope, usize)>,
    bytes: usize,
}
impl Channel {
    pub fn new(
        extension: ExtensionId,
        view: String,
        generation: u64,
        limits: ChannelLimits,
    ) -> Result<Self> {
        if view.is_empty()
            || view.len() > 128
            || generation == 0
            || limits.message_bytes == 0
            || limits.queued_bytes == 0
            || limits.queued_messages == 0
        {
            return Err(Error::Invalid("channel config"));
        }
        Ok(Self {
            extension,
            view,
            generation,
            limits,
            admitted: BTreeMap::new(),
            completed: BTreeMap::new(),
            queue: VecDeque::new(),
            bytes: 0,
        })
    }
    pub fn admit(&mut self, authenticated_source: Endpoint, json: &[u8]) -> Result<Receipt> {
        if json.len() > self.limits.message_bytes {
            return Err(Error::Invalid("message size"));
        }
        let m: Envelope =
            serde_json::from_slice(json).map_err(|_| Error::Invalid("message schema"))?;
        if m.version != PROTOCOL_VERSION {
            return Err(Error::Invalid("protocol version"));
        }
        if m.extension != self.extension || m.view != self.view || m.generation != self.generation {
            return Err(Error::Stale);
        }
        if m.from != authenticated_source || !valid_route(&m) {
            return Err(Error::Denied);
        }
        let last = *self.admitted.get(&m.from).unwrap_or(&0);
        if m.sequence == 0 {
            return Err(Error::Invalid("sequence"));
        }
        if m.sequence <= last {
            if let Some((pending, _)) = self
                .queue
                .iter()
                .find(|(old, _)| old.from == m.from && old.sequence == m.sequence)
            {
                return if *pending == m {
                    Ok(Receipt::Duplicate { completed: false })
                } else {
                    Err(Error::Conflict)
                };
            }
            if let Some((sequence, digest)) = self.completed.get(&m.from)
                && *sequence == m.sequence
            {
                return if *digest == fingerprint(&m)? {
                    Ok(Receipt::Duplicate { completed: true })
                } else {
                    Err(Error::Conflict)
                };
            }
            // Only the last completion per endpoint is retained, bounding replay
            // memory. Older retries must query the kernel transaction receipt.
            return Err(Error::Stale);
        }
        if last.checked_add(1) != Some(m.sequence) {
            return Err(Error::Invalid("sequence gap"));
        }
        if self.queue.len() >= self.limits.queued_messages
            || json.len() > self.limits.queued_bytes.saturating_sub(self.bytes)
        {
            return Err(Error::Backpressure);
        }
        self.admitted.insert(m.from, m.sequence);
        self.bytes += json.len();
        self.queue.push_back((m, json.len()));
        Ok(Receipt::Queued)
    }
    pub fn front(&self) -> Option<&Envelope> {
        self.queue.front().map(|(m, _)| m)
    }
    /// Complete only after the consumer accepts the work. Lost transport replies
    /// cannot repeat a kernel edit; ServiceCall.request also needs kernel txids.
    pub fn complete(&mut self, source: Endpoint, sequence: u64) -> Result<()> {
        if !self
            .queue
            .front()
            .is_some_and(|(m, _)| m.from == source && m.sequence == sequence)
        {
            return Err(Error::Stale);
        }
        let digest = fingerprint(self.front().expect("checked front"))?;
        let (_, bytes) = self.queue.pop_front().expect("checked front");
        self.bytes -= bytes;
        self.completed.insert(source, (sequence, digest));
        Ok(())
    }
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}
fn fingerprint(m: &Envelope) -> Result<[u8; 32]> {
    let bytes = serde_json::to_vec(m).map_err(|_| Error::Invalid("message encoding"))?;
    Ok(Sha256::digest(bytes).into())
}
fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}
fn valid_route(m: &Envelope) -> bool {
    use Endpoint::*;
    match (&m.message, m.from, m.to) {
        (Message::Ready { origin }, View, Realm) => origin == "null",
        (Message::Input { node, event, .. }, View, Realm) => *node > 0 && name(event),
        (Message::PaintAck { .. }, View, Realm) => true,
        (Message::Failure { code }, View, Realm) => name(code),
        (Message::DomBatch { .. } | Message::Snapshot { .. }, Realm, View) => true,
        (
            Message::ServiceCall {
                request, method, ..
            },
            Realm,
            Kernel,
        ) => *request > 0 && name(method),
        (Message::ServiceReply { request, .. }, Kernel, Realm) => *request > 0,
        (Message::KernelEvent { event, .. }, Kernel, Realm) => name(event),
        _ => false,
    }
}
