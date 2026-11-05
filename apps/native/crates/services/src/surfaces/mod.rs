//! Non-visual secondary-surface backing. No IO is performed by reducers.
//! Worker adapters use `ServiceWorker::submit`/`Pending::try_take`; the kernel
//! remains the sole document/settings writer. No v1–3 consumer is replaced.
//! Presentation follows Opus's design/surfaces specs through 78294c4c9; `copy` projects
//! their supplied strings. No geometry, material, colour or new UX is defined.
//! S066 is deliberately not built. Existing agent panels retain their consumers.
pub const BEHAVIOR_REFERENCE: &str = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d";
pub const LEGACY_REFERENCE: &str = "07ed5fa17ee640659cb075f9cad1af2095011d60";
pub mod agent_auth;
pub mod agent_clients;
pub mod agent_reply;
pub mod captions;
pub mod client_consent;
pub mod copy;
pub mod easing;
pub mod providers;
pub mod publishing;
pub mod shader;
pub mod store;
pub mod transcription;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local effect identity, deliberately not a durable resume credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket(u64);

#[derive(Debug, Default)]
pub struct Flight(Option<Ticket>);
impl Flight {
    pub fn start(&mut self) -> Ticket {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("surface ticket space exhausted");
        let ticket = Ticket(id);
        self.0 = Some(ticket);
        ticket
    }
    pub fn current(&self) -> Option<Ticket> {
        self.0
    }
    pub fn accepts(&self, ticket: Ticket) -> bool {
        self.0 == Some(ticket)
    }
    pub fn finish(&mut self, ticket: Ticket) -> bool {
        if !self.accepts(ticket) {
            return false;
        }
        self.0 = None;
        true
    }
    pub fn cancel(&mut self) -> Option<Ticket> {
        self.0.take()
    }
}

/// Exact document lifetime, supplied by the application publication adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentStamp {
    pub project: String,
    pub session: u64,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tickets_reject_replacement_replay_and_recreated_controller() {
        let mut flight = Flight::default();
        let first = flight.start();
        let second = flight.start();
        assert!(!flight.finish(first));
        assert!(flight.finish(second));
        assert!(!flight.finish(second));
        let mut next = Flight::default();
        assert_ne!(next.start(), second);
        assert!(!next.accepts(second));
        let cancelled = next.cancel().unwrap();
        assert!(!next.finish(cancelled));
    }
}
