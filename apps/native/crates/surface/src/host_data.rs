//! Concrete worker-side data service. KernelData is the application adapter to
//! pm-kernel's immutable publications/QueryMirror and nonblocking actor mailbox.
//! No document store, command executor or success-shaped backend lives here.
use crate::host_api::{
    HOST_API_VERSION, HostAck, HostOperation, HostPublication, HostRequest, Identity, KernelQueue,
    Query, validate_operation,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::SyncSender;

const SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq)]
pub struct QuerySnapshot {
    pub revision: u64,
    pub value: Value,
}

pub trait KernelData {
    /// Read one immutable publication. Evaluated queries use a worker-owned
    /// QueryMirror synced to that publication; never combine revisions.
    fn query(
        &mut self,
        identity: &Identity,
        query: &Query,
        channels: Option<&[String]>,
        time: Option<f64>,
    ) -> Result<QuerySnapshot, String>;
    /// Route edits/commands to KernelHandle::call/call_with, settings to
    /// state_transaction, bindings to the actor. Admit expectedRevision and
    /// writer generation atomically there, with request-ID -> txid dedupe.
    /// A multi-command Edit is one transaction, including rollback on failure.
    /// Send only the actual actor/persistence ack; never wait in this method.
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String>;
    /// Seal the actor writer and release its bindings. Must be retryable.
    fn retire(&mut self, identity: &Identity) -> Result<(), String>;
}

struct Subscription {
    query: Query,
    channels: Option<Vec<String>>,
    last: QuerySnapshot,
    sequence: u64,
}

/// Use behind HostRouter for bounded request/ack journaling and lost-ack fencing.
/// All methods run on the native worker; the main thread only enqueues work.
pub struct HostDataService<K> {
    identity: Identity,
    kernel: K,
    subscriptions: BTreeMap<String, Subscription>,
    used_subscriptions: BTreeSet<String>,
    retired: bool,
}
impl<K: KernelData> HostDataService<K> {
    pub fn new(identity: Identity, kernel: K) -> Result<Self, String> {
        if identity.realm.is_empty()
            || identity.project.is_empty()
            || identity.generation == 0
            || identity.generation > SAFE_INTEGER
        {
            return Err("invalid host identity".into());
        }
        Ok(Self {
            identity,
            kernel,
            subscriptions: BTreeMap::new(),
            used_subscriptions: BTreeSet::new(),
            retired: false,
        })
    }

    /// Called after a kernel/transport publication wake. Time may change at the
    /// same document revision, so value and sequence both matter. The receiver
    /// receives an initial snapshot in the subscribe ack, closing the read/subscribe gap.
    pub fn publications(&mut self) -> Result<Vec<HostPublication>, String> {
        if self.retired {
            return Err("host generation retired".into());
        }
        // Stage before updating cursors: a failed read cannot lose earlier changes.
        let mut changes = Vec::new();
        for (id, subscription) in &self.subscriptions {
            let snapshot = self.kernel.query(
                &self.identity,
                &subscription.query,
                subscription.channels.as_deref(),
                None,
            )?;
            if snapshot.revision > SAFE_INTEGER || snapshot.revision < subscription.last.revision {
                return Err("stale/invalid kernel publication".into());
            }
            if snapshot != subscription.last {
                if subscription.sequence == SAFE_INTEGER {
                    return Err("publication sequence exhausted".into());
                }
                changes.push((id.clone(), snapshot));
            }
        }
        Ok(changes
            .into_iter()
            .map(|(id, snapshot)| {
                let subscription = self
                    .subscriptions
                    .get_mut(&id)
                    .expect("staged subscription");
                subscription.sequence += 1;
                subscription.last = snapshot.clone();
                HostPublication {
                    version: HOST_API_VERSION,
                    realm: self.identity.realm.clone(),
                    generation: self.identity.generation,
                    project: self.identity.project.clone(),
                    subscription: id,
                    revision: snapshot.revision,
                    sequence: subscription.sequence,
                    value: snapshot.value,
                }
            })
            .collect())
    }

    fn snapshot(
        &mut self,
        query: &Query,
        channels: Option<&[String]>,
        time: Option<f64>,
        expected: u64,
    ) -> Result<QuerySnapshot, String> {
        let snapshot = self.kernel.query(&self.identity, query, channels, time)?;
        if snapshot.revision < expected || snapshot.revision > SAFE_INTEGER {
            return Err("stale/invalid kernel query publication".into());
        }
        Ok(snapshot)
    }
}
impl<K: KernelData> KernelQueue for HostDataService<K> {
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        if self.retired
            || request.version != HOST_API_VERSION
            || request.realm != self.identity.realm
            || request.project != self.identity.project
            || request.generation != self.identity.generation
        {
            return Err("stale host generation/version/project".into());
        }
        validate_operation(&request.operation)?;
        let mut registration = None;
        let mut removal = None;
        let snapshot = match &request.operation {
            HostOperation::Query {
                query,
                channels,
                time,
            } => self.snapshot(query, channels.as_deref(), *time, request.expected_revision)?,
            HostOperation::Subscribe {
                subscription,
                query,
                channels,
            } => {
                if self.used_subscriptions.contains(subscription) {
                    return Err("duplicate or retired subscription ID".into());
                }
                if self.subscriptions.len() >= 256 || self.used_subscriptions.len() >= 4096 {
                    return Err("subscription capacity".into());
                }
                let snapshot =
                    self.snapshot(query, channels.as_deref(), None, request.expected_revision)?;
                registration = Some((
                    subscription.clone(),
                    Subscription {
                        query: query.clone(),
                        channels: channels.clone(),
                        last: snapshot.clone(),
                        sequence: 0,
                    },
                ));
                snapshot
            }
            HostOperation::Unsubscribe { subscription } => {
                let snapshot =
                    self.snapshot(&Query::Project, None, None, request.expected_revision)?;
                removal = Some(subscription.clone());
                QuerySnapshot {
                    revision: snapshot.revision,
                    value: Value::Null,
                }
            }
            _ => return self.kernel.enqueue(request, reply),
        };
        reply
            .try_send(HostAck {
                version: HOST_API_VERSION,
                realm: request.realm,
                generation: request.generation,
                project: request.project,
                request_id: request.request_id,
                revision: snapshot.revision,
                accepted: true,
                state_revision: None,
                value: Some(snapshot.value),
                error: None,
            })
            .map_err(|_| String::from("host reply queue unavailable"))?;
        if let Some((id, subscription)) = registration {
            self.used_subscriptions.insert(id.clone());
            self.subscriptions.insert(id, subscription);
        }
        if let Some(id) = removal {
            self.subscriptions.remove(&id);
        }
        Ok(())
    }
    fn retire(&mut self, identity: &Identity) -> Result<(), String> {
        if identity != &self.identity {
            return Err("foreign host identity".into());
        }
        if !self.retired {
            self.kernel.retire(identity)?;
            self.subscriptions.clear();
            self.retired = true;
        }
        Ok(())
    }
}
