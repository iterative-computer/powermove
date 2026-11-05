//! Nonvisual backing for Item 6B. Invoke effect adapters on ServiceWorker, never
//! from a view callback. Published kernel state remains the only edit authority.
//! State/copy authority: docs/native/design/surfaces, merged at ce36cb6fc.
//! No layout, colours, typography or new token fallbacks are defined here.
pub mod authorization;
pub mod copy;
pub mod generated;
pub mod maintenance;
pub mod mixer;
pub mod native;
pub mod onboarding;
pub mod projects;
pub mod scene;
pub mod workspaces;

use crate::{Result, ServiceError, ServiceErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// Session identity must change on close/reopen, including reopening the same id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Session {
    pub project: String,
    pub incarnation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);
pub(crate) fn ticket() -> Ticket {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut n = NEXT.load(Ordering::Relaxed);
    loop {
        match NEXT.compare_exchange_weak(
            n,
            n.checked_add(1).expect("surface ticket space exhausted"),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ticket(n),
            Err(actual) => n = actual,
        }
    }
}
pub(crate) fn invalid(why: &str) -> ServiceError {
    ServiceError::message(ServiceErrorCode::InvalidInput, why)
}
pub(crate) fn stale() -> ServiceError {
    ServiceError::message(
        ServiceErrorCode::Unavailable,
        "stale surface request; refresh required",
    )
}
pub(crate) fn unavailable(why: &str) -> ServiceError {
    ServiceError::message(ServiceErrorCode::Unavailable, why)
}
pub(crate) fn id(value: &Value) -> Result<&str> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("missing record id"))
}

/// Immutable read of a kernel-owned slot. No lossy schema conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct Records {
    pub revision: u64,
    pub entries: Vec<Value>,
}
impl Records {
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for item in &self.entries {
            if !ids.insert(id(item)?) {
                return Err(invalid("duplicate record id"));
            }
        }
        Ok(())
    }
    pub fn find(&self, record_id: &str) -> Result<&Value> {
        self.entries
            .iter()
            .find(|v| v.get("id").and_then(Value::as_str) == Some(record_id))
            .ok_or_else(|| ServiceError::message(ServiceErrorCode::NotFound, "record missing"))
    }
}

/// The kernel implementation MUST compare revision and archive the old slot in
/// the same transaction as replacement. A SettingsHook get/set pair is insufficient.
/// A failure leaves the old slot intact; retries use a fresh read. No filesystem IO here.
pub trait RecordHost {
    fn replace_retaining(&mut self, slot: &str, before: &Records, after: &[Value]) -> Result<u64>;
}
pub(crate) fn replace(
    host: &mut impl RecordHost,
    slot: &str,
    before: &Records,
    after: &[Value],
) -> Result<u64> {
    before.validate()?;
    Records {
        revision: before.revision,
        entries: after.to_vec(),
    }
    .validate()?;
    let revision = host.replace_retaining(slot, before, after)?;
    if revision <= before.revision {
        return Err(unavailable("host did not publish a new revision"));
    }
    Ok(revision)
}

#[cfg(test)]
mod tests;
