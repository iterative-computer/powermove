//! Versioned worker -> pm-kernel adapter protocol. The adapter owns authorization,
//! command admission, durable settings and publications; this module owns no document.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

pub const HOST_API_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub realm: String,
    pub generation: u64,
    pub project: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum HostOperation {
    #[serde(rename = "binding.register")]
    RegisterBinding {
        handle: String,
        channels: Vec<String>,
    },
    #[serde(rename = "binding.dispose")]
    DisposeBinding { handle: String },
    #[serde(rename = "query")]
    Query {
        query: Query,
        channels: Option<Vec<String>>,
        time: Option<f64>,
    },
    #[serde(rename = "subscribe")]
    Subscribe {
        subscription: String,
        query: Query,
        channels: Option<Vec<String>>,
    },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { subscription: String },
    #[serde(rename = "command")]
    Command { command: String, args: Value },
    /// One actor-admitted, undoable transaction, never a sequence of local writes.
    #[serde(rename = "edit")]
    Edit {
        label: String,
        commands: Vec<EditCommand>,
    },
    #[serde(rename = "settings.get")]
    GetSetting { key: String },
    #[serde(rename = "settings.set")]
    SetSetting { key: String, value: Value },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Query {
    Project,
    Selection,
    Time,
    Evaluated,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditCommand {
    pub command: String,
    pub args: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPublication {
    pub version: u32,
    pub realm: String,
    pub generation: u64,
    pub project: String,
    pub subscription: String,
    pub revision: u64,
    pub sequence: u64,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostRequest {
    pub version: u32,
    pub realm: String,
    pub generation: u64,
    pub project: String,
    pub request_id: u64,
    pub expected_revision: u64,
    pub expected_state_revision: Option<u64>,
    pub operation: HostOperation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostAck {
    pub version: u32,
    pub realm: String,
    pub generation: u64,
    pub project: String,
    pub request_id: u64,
    pub revision: u64,
    pub accepted: bool,
    pub state_revision: Option<u64>,
    pub value: Option<Value>,
    pub error: Option<String>,
}
/// Implement in the application adapter with KernelHandle::call/call_with,
/// QueryMirror publications and KernelHandle::state_transaction. Never wait here.
pub trait KernelQueue {
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String>;
    /// Must seal this writer and release its handles/subscriptions via the actor.
    fn retire(&mut self, identity: &Identity) -> Result<(), String>;
}
/// Worker-only route: bounded, nonblocking, generation checked, ack journal retained.
/// Lost acks are inspected by request ID; they are never blindly replayed as edits.
pub struct HostRouter<Q> {
    identity: Identity,
    queue: Q,
    pending: BTreeMap<u64, (u64, Option<u64>, Receiver<HostAck>)>,
    acks: BTreeMap<u64, HostAck>,
    seen: BTreeSet<u64>,
    retired: bool,
    fenced: bool,
}
impl<Q: KernelQueue> HostRouter<Q> {
    pub fn new(identity: Identity, queue: Q) -> Result<Self, String> {
        if identity.realm.is_empty()
            || identity.project.is_empty()
            || identity.generation == 0
            || identity.generation > 9_007_199_254_740_991
        {
            return Err("invalid host identity".into());
        }
        Ok(Self {
            identity,
            queue,
            pending: BTreeMap::new(),
            acks: BTreeMap::new(),
            seen: BTreeSet::new(),
            retired: false,
            fenced: false,
        })
    }
    pub fn enqueue(&mut self, request: HostRequest) -> Result<(), String> {
        if self.retired
            || self.fenced
            || request.version != HOST_API_VERSION
            || request.realm != self.identity.realm
            || request.generation != self.identity.generation
            || request.project != self.identity.project
        {
            return Err("stale host generation/version/project".into());
        }
        if request.request_id == 0
            || request.request_id > 9_007_199_254_740_991
            || request.expected_revision > 9_007_199_254_740_991
            || request
                .expected_state_revision
                .is_some_and(|v| v > 9_007_199_254_740_991)
            || self.seen.contains(&request.request_id)
        {
            return Err("invalid or previously submitted request ID; inspect ack".into());
        }
        if self.pending.len() >= 256 || self.seen.len() >= 4096 {
            return Err("host queue capacity; retire generation".into());
        }
        validate_operation(&request.operation)?;
        let (tx, rx) = mpsc::sync_channel(1);
        let id = request.request_id;
        let revision = request.expected_revision;
        let state_revision = request.expected_state_revision;
        self.queue.enqueue(request, tx)?;
        self.seen.insert(id);
        self.pending.insert(id, (revision, state_revision, rx));
        Ok(())
    }
    pub fn poll(&mut self, id: u64) -> Result<Option<HostAck>, String> {
        if self.retired {
            return Err("host generation retired".into());
        }
        if let Some(ack) = self.acks.get(&id) {
            return Ok(Some(ack.clone()));
        }
        let (expected, state_revision, receiver) =
            self.pending.get(&id).ok_or("unknown host request")?;
        let ack = match receiver.try_recv() {
            Ok(ack) => ack,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.fenced = true;
                return Err("host ack lost; query kernel txid or reset generation".into());
            }
        };
        if ack.version != HOST_API_VERSION
            || ack.realm != self.identity.realm
            || ack.project != self.identity.project
            || ack.generation != self.identity.generation
            || ack.request_id != id
            || ack.revision < *expected
            || ack.revision > 9_007_199_254_740_991
            || (ack.accepted
                && state_revision.is_some_and(|expected| {
                    ack.state_revision
                        .is_none_or(|actual| actual < expected || actual > 9_007_199_254_740_991)
                }))
        {
            self.fenced = true;
            return Err("mismatched/stale host ack; generation reset required".into());
        }
        self.pending.remove(&id);
        self.acks.insert(id, ack.clone());
        Ok(Some(ack))
    }
    pub fn retire(&mut self) -> Result<(), String> {
        if self.retired {
            return Ok(());
        }
        self.queue.retire(&self.identity)?;
        self.retired = true;
        self.pending.clear();
        Ok(())
    }
}
impl<K: crate::host_data::KernelData> HostRouter<crate::host_data::HostDataService<K>> {
    pub fn publications(&mut self) -> Result<Vec<HostPublication>, String> {
        if self.retired || self.fenced {
            return Err("host generation retired/fenced".into());
        }
        self.queue.publications()
    }
}
pub(crate) fn validate_operation(operation: &HostOperation) -> Result<(), String> {
    let valid = match operation {
        HostOperation::RegisterBinding { handle, channels } => {
            !handle.is_empty() && !channels.is_empty() && channels.iter().all(|s| !s.is_empty())
        }
        HostOperation::DisposeBinding { handle } => !handle.is_empty(),
        HostOperation::Query { time, .. } => time.is_none_or(f64::is_finite),
        HostOperation::Subscribe { subscription, .. }
        | HostOperation::Unsubscribe { subscription } => !subscription.is_empty(),
        HostOperation::Command { command, .. } => !command.is_empty(),
        HostOperation::Edit { label, commands } => {
            !label.is_empty()
                && !commands.is_empty()
                && commands.len() <= 256
                && commands.iter().all(|c| !c.command.is_empty())
        }
        HostOperation::GetSetting { key } | HostOperation::SetSetting { key, .. } => {
            !key.is_empty()
        }
    };
    if valid {
        Ok(())
    } else {
        Err("invalid host operation".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Queue {
        requests: Vec<(HostRequest, SyncSender<HostAck>)>,
        retired: bool,
    }
    impl KernelQueue for Queue {
        fn enqueue(
            &mut self,
            request: HostRequest,
            reply: SyncSender<HostAck>,
        ) -> Result<(), String> {
            self.requests.push((request, reply));
            Ok(())
        }
        fn retire(&mut self, _: &Identity) -> Result<(), String> {
            self.retired = true;
            Ok(())
        }
    }
    fn route() -> HostRouter<Queue> {
        HostRouter::new(
            Identity {
                realm: "test".into(),
                generation: 2,
                project: "synthetic".into(),
            },
            Queue::default(),
        )
        .unwrap()
    }
    fn request() -> HostRequest {
        serde_json::from_str(r#"{"version":1,"realm":"test","generation":2,"project":"synthetic","requestId":1,"expectedRevision":3,"operation":{"type":"command","command":"test","args":{"exact":false}}}"#).unwrap()
    }
    fn ack() -> HostAck {
        HostAck {
            version: 1,
            realm: "test".into(),
            generation: 2,
            project: "synthetic".into(),
            request_id: 1,
            revision: 4,
            accepted: true,
            state_revision: None,
            value: Some(Value::Bool(false)),
            error: None,
        }
    }
    #[test]
    fn nonblocking_ack_journal_and_no_duplicate_execution() {
        let mut route = route();
        route.enqueue(request()).unwrap();
        assert_eq!(route.poll(1).unwrap(), None);
        route.queue.requests[0].1.try_send(ack()).unwrap();
        assert_eq!(route.poll(1).unwrap(), Some(ack()));
        assert_eq!(route.poll(1).unwrap(), Some(ack()));
        assert!(route.enqueue(request()).is_err());
        assert_eq!(route.queue.requests.len(), 1);
    }
    #[test]
    fn stale_generation_and_wrong_ack_never_publish() {
        let mut route = route();
        let mut stale = request();
        stale.generation = 1;
        assert!(route.enqueue(stale).is_err());
        route.enqueue(request()).unwrap();
        let mut stale = ack();
        stale.project = "other".into();
        route.queue.requests[0].1.try_send(stale).unwrap();
        assert!(route.poll(1).is_err());
        route.retire().unwrap();
        route.retire().unwrap();
        assert!(route.queue.retired);
        assert!(route.poll(1).is_err());
    }
    #[test]
    fn lost_ack_fences_instead_of_replaying() {
        let mut route = route();
        route.enqueue(request()).unwrap();
        route.queue.requests.clear();
        assert!(route.poll(1).is_err());
        let mut next = request();
        next.request_id = 2;
        assert!(route.enqueue(next).is_err());
    }
    #[test]
    fn settings_revision_is_independent_and_required_on_accepted_settings() {
        let mut route = route();
        let mut req = request();
        req.expected_state_revision = Some(20);
        req.operation = HostOperation::SetSetting {
            key: "recents".into(),
            value: Value::Array(vec![]),
        };
        route.enqueue(req).unwrap();
        let mut result = ack();
        result.state_revision = Some(21);
        route.queue.requests[0].1.try_send(result.clone()).unwrap();
        assert_eq!(route.poll(1).unwrap(), Some(result));
        let mut req = request();
        req.request_id = 2;
        req.expected_state_revision = Some(21);
        route.enqueue(req).unwrap();
        let mut result = ack();
        result.request_id = 2;
        route.queue.requests[1].1.try_send(result).unwrap();
        assert!(route.poll(2).is_err());
    }

    #[test]
    fn malformed_operation_and_envelope_are_refused() {
        let mut route = route();
        let mut bad = request();
        bad.operation = HostOperation::RegisterBinding {
            handle: "test".into(),
            channels: vec![],
        };
        assert!(route.enqueue(bad).is_err());
        assert!(route.queue.requests.is_empty());
        assert!(serde_json::from_str::<HostRequest>(r#"{"version":1,"extra":true}"#).is_err());
    }
}
