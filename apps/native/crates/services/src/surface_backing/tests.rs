//! Synthetic fixtures only; no external repository fixtures or user data.
mod flows;
mod generated;
mod mixer;
mod projects;
mod spec_states;

use super::*;
use serde_json::{Value, json};

fn session(n: u64) -> Session {
    Session {
        project: "synthetic-project".into(),
        incarnation: n,
    }
}
fn failure() -> ServiceError {
    ServiceError::message(ServiceErrorCode::DiskFull, "synthetic full volume")
}
fn records(entries: Vec<Value>) -> Records {
    Records {
        revision: 7,
        entries,
    }
}
#[derive(Default)]
struct MemoryRecords {
    slot: String,
    current: Option<Records>,
    history: Vec<Records>,
    fail: bool,
}
impl RecordHost for MemoryRecords {
    fn replace_retaining(&mut self, slot: &str, before: &Records, after: &[Value]) -> Result<u64> {
        if self.fail {
            return Err(failure());
        }
        if self.current.as_ref() != Some(before) {
            return Err(stale());
        }
        self.slot = slot.into();
        self.history.push(before.clone());
        self.current = Some(Records {
            revision: before.revision + 1,
            entries: after.to_vec(),
        });
        Ok(before.revision + 1)
    }
}
#[test]
fn tickets_do_not_repeat_between_controllers() {
    assert_ne!(ticket(), ticket());
}
#[test]
fn records_refuse_duplicate_and_missing_ids() {
    assert!(
        records(vec![json!({"id":"a"}), json!({"id":"a"})])
            .validate()
            .is_err()
    );
    assert!(records(vec![json!({"unknown":3})]).validate().is_err());
}
