//! Main's `storage-upgrade-v1.json` receipt. Browser progress is per origin;
//! native migration keeps this as an independent receipt, never a release flag.
//! Each durable step must be verified by its worker before checkpointing. The
//! cleanup worker archives superseded bytes under N4; it must not delete them.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

pub const STAGES: [&str; 4] = ["recovery", "browser", "sources", "cleanup"];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub steps: Vec<String>,
    pub before_bytes: Number,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freed_bytes: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notified: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub origins: BTreeMap<String, Origin>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            version: 1,
            origins: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}
impl Journal {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let raw: Value = serde_json::from_slice(bytes)?;
        if let Some(origins) = raw.get("origins").and_then(Value::as_object) {
            for origin in origins.values() {
                ensure!(
                    origin.get("freedBytes").is_none_or(Value::is_number)
                        && origin.get("notified").is_none_or(Value::is_boolean),
                    "invalid optional storage upgrade fields"
                );
            }
        }
        let journal: Self = serde_json::from_slice(bytes)?;
        ensure!(journal.version == 1, "unsupported storage upgrade receipt");
        for origin in journal.origins.values() {
            ensure!(
                origin.steps.len() <= STAGES.len()
                    && origin.steps.iter().zip(STAGES).all(|(a, b)| a == b),
                "invalid storage upgrade stage prefix"
            );
            ensure!(
                origin
                    .before_bytes
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && n >= 0.0)
                    && origin
                        .freed_bytes
                        .as_ref()
                        .is_none_or(|n| n.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0)),
                "invalid storage upgrade byte counts"
            );
        }
        Ok(journal)
    }
}

/// Receipt publication is a host responsibility: atomic write, fsync and
/// rename. In-memory state changes only after this callback succeeds.
pub trait ReceiptStore {
    fn publish(&mut self, journal: &Journal) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    owner: u64,
    origin: String,
    token: u64,
}
#[derive(Debug)]
pub struct Upgrade {
    pub journal: Journal,
    claim: Option<Lease>,
    generation: u64,
}
impl Upgrade {
    pub fn new(journal: Journal) -> Result<Self> {
        let journal = Journal::decode(&serde_json::to_vec(&journal)?)?;
        Ok(Self {
            journal,
            claim: None,
            generation: 0,
        })
    }
    pub fn start(
        &mut self,
        owner: u64,
        origin: &str,
        before: f64,
        store: &mut impl ReceiptStore,
    ) -> Result<Option<Lease>> {
        if let Some(claim) = &self.claim {
            return Ok((claim.owner == owner && claim.origin == origin).then(|| claim.clone()));
        }
        ensure!(
            before.is_finite() && before >= 0.0,
            "invalid storage measurement"
        );
        if !self.journal.origins.contains_key(origin) {
            let mut next = self.journal.clone();
            next.origins.insert(
                origin.into(),
                Origin {
                    steps: Vec::new(),
                    before_bytes: Number::from_f64(before).unwrap(),
                    freed_bytes: None,
                    notified: None,
                    extra: Map::new(),
                },
            );
            store.publish(&next)?;
            self.journal = next;
        }
        self.generation += 1;
        let lease = Lease {
            owner,
            origin: origin.into(),
            token: self.generation,
        };
        self.claim = Some(lease.clone());
        Ok(Some(lease))
    }
    pub fn release(&mut self, owner: u64) {
        if self.claim.as_ref().is_some_and(|c| c.owner == owner) {
            self.claim = None;
        }
    }
    fn check(&self, lease: &Lease) -> Result<()> {
        ensure!(
            self.claim.as_ref() == Some(lease),
            "storage upgrade session is stale"
        );
        Ok(())
    }
    /// Execute exactly one uncompleted step. `Ok(None)` from the worker means
    /// it is still waiting (e.g. browser-origin transfer). `Some(bytes_after)`
    /// certifies durable verified completion. Error/restart reruns that step.
    pub fn next(
        &mut self,
        lease: &Lease,
        store: &mut impl ReceiptStore,
        work: impl FnOnce(&str) -> Result<Option<f64>>,
    ) -> Result<bool> {
        self.check(lease)?;
        let current = &self.journal.origins[&lease.origin];
        let Some(stage) = STAGES.get(current.steps.len()) else {
            return Ok(true);
        };
        let Some(after) = work(stage)? else {
            return Ok(false);
        };
        ensure!(
            after.is_finite() && after >= 0.0,
            "invalid storage measurement"
        );
        let mut next = self.journal.clone();
        let origin = next.origins.get_mut(&lease.origin).unwrap();
        origin.steps.push((*stage).into());
        let complete = origin.steps.len() == STAGES.len();
        if complete {
            origin.freed_bytes =
                Number::from_f64((origin.before_bytes.as_f64().unwrap() - after).max(0.0));
        }
        store.publish(&next)?;
        self.journal = next;
        Ok(complete)
    }
    pub fn acknowledge(&mut self, lease: &Lease, store: &mut impl ReceiptStore) -> Result<()> {
        self.check(lease)?;
        ensure!(
            self.journal.origins[&lease.origin].steps.len() == STAGES.len(),
            "storage upgrade is not complete"
        );
        let mut next = self.journal.clone();
        next.origins.get_mut(&lease.origin).unwrap().notified = Some(true);
        store.publish(&next)?;
        self.journal = next;
        self.claim = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Store {
        bytes: Vec<u8>,
        fail: bool,
    }
    impl ReceiptStore for Store {
        fn publish(&mut self, journal: &Journal) -> Result<()> {
            ensure!(!self.fail, "injected publication failure");
            self.bytes = serde_json::to_vec(journal)?;
            Ok(())
        }
    }
    #[test]
    fn main_receipt_round_trip_and_invalid_prefixes_fail_without_reset() {
        let bytes = include_bytes!("../tests/fixtures/storage-upgrade-v1.json");
        let raw: Value = serde_json::from_slice(bytes).unwrap();
        let journal = Journal::decode(bytes).unwrap();
        assert_eq!(serde_json::to_value(journal).unwrap(), raw);
        for steps in [
            serde_json::json!(["browser"]),
            serde_json::json!(["recovery", "sources"]),
            serde_json::json!(["recovery", "browser", "sources", "cleanup", "future"]),
        ] {
            let mut invalid = raw.clone();
            invalid["origins"]["app://powermove"]["steps"] = steps;
            assert!(Journal::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }
    #[test]
    fn origin_lease_interruption_and_completion_ack_are_durable() {
        let mut upgrade = Upgrade::new(Journal::default()).unwrap();
        let mut store = Store::default();
        let lease = upgrade
            .start(1, "app://powermove", 100.0, &mut store)
            .unwrap()
            .unwrap();
        assert!(
            upgrade
                .start(2, "http://localhost", 100.0, &mut store)
                .unwrap()
                .is_none()
        );
        assert!(upgrade.acknowledge(&lease, &mut store).is_err());
        assert!(
            !upgrade
                .next(&lease, &mut store, |_| Ok(Some(100.0)))
                .unwrap()
        );
        let before = store.bytes.clone();
        store.fail = true;
        assert!(
            upgrade
                .next(&lease, &mut store, |_| Ok(Some(100.0)))
                .is_err()
        );
        assert_eq!(store.bytes, before);
        assert_eq!(upgrade.journal.origins["app://powermove"].steps.len(), 1);
        store.fail = false;
        upgrade.release(1);
        assert!(
            upgrade
                .next(&lease, &mut store, |_| Ok(Some(100.0)))
                .is_err()
        );
        let mut resumed = Upgrade::new(Journal::decode(&store.bytes).unwrap()).unwrap();
        let lease = resumed
            .start(2, "app://powermove", 100.0, &mut store)
            .unwrap()
            .unwrap();
        assert!(!resumed.next(&lease, &mut store, |_| Ok(None)).unwrap());
        for stage in ["browser", "sources", "cleanup"] {
            resumed
                .next(&lease, &mut store, |actual| {
                    assert_eq!(actual, stage);
                    Ok(Some(100.0))
                })
                .unwrap();
        }
        resumed.acknowledge(&lease, &mut store).unwrap();
        assert_eq!(
            Journal::decode(&store.bytes).unwrap().origins["app://powermove"].notified,
            Some(true)
        );
        resumed
            .start(3, "http://localhost", 50.0, &mut store)
            .unwrap()
            .unwrap();
        assert!(resumed.journal.origins["http://localhost"].steps.is_empty());
    }
}
