//! Background-worker integration. UI maps stable states to N001–N007 only after
//! Opus supplies the specs. DESIGN-TODO(opus): no new copy or interaction here.
use crate::{Config, Event, Outcome, delta, run};
use anyhow::Result;
use pm_distribution::update_handoff::{self, DeltaReport, Handoff, HandoffHost, LaunchSource};

/// Trusted application capability, never renderer flags. Implementations keep
/// source/import leases for the entire `first_run` call. Evidence must be read
/// again AFTER the final delta, including signed identity, full closure and a
/// current-state Restore round trip. Missing production adapters return false.
pub trait Evidence {
    fn electron_quiescent(&mut self) -> Result<bool>;
    fn verify_transition_receipt(&mut self) -> Result<bool>;
    fn after_delta(&mut self) -> Result<DeltaReport>;
}

/// Concrete bridge to the real coordinator, not a simulated migration report.
/// `Ready` is cutover readiness, not merely completion of isolated beta import.
/// Call on a Rust worker before attaching the library; never on AppKit's thread.
pub struct Coordinator<'a, E, F> {
    pub config: &'a Config,
    pub evidence: &'a mut E,
    pub events: F,
}
impl<E: Evidence, F: FnMut(Event) -> Result<()>> HandoffHost for Coordinator<'_, E, F> {
    fn electron_quiescent(&mut self) -> Result<bool> {
        // Both observations are required; no trusted receipt bypasses the lock.
        Ok(
            !pm_kernel::locks::older_electron_running(&self.config.electron_profile)
                && self.evidence.electron_quiescent()?,
        )
    }
    fn verify_transition_receipt(&mut self) -> Result<bool> {
        self.evidence.verify_transition_receipt()
    }
    fn final_delta(&mut self, _reuse_transition_evidence: bool) -> Result<DeltaReport> {
        // TR evidence is advisory. Direct/skipped TR use the identical importer.
        let outcome = run(self.config, &mut self.events)?;
        let complete = match outcome {
            Outcome::WaitingForElectron => return Ok(DeltaReport::default()),
            Outcome::NeedsSecretConsent { .. } => {
                return Ok(DeltaReport {
                    complete: true,
                    ..DeltaReport::default()
                });
            }
            Outcome::Complete {
                history_refusals,
                missing_projects,
                blocked_owners,
                ..
            } => history_refusals == 0 && missing_projects == 0 && blocked_owners == 0,
        };
        // A resumed bootstrap (including consent completion) may describe an old
        // checkpoint. Always capture a fresh delta before asking owners for proof.
        let fresh = delta(self.config, &mut self.events)?;
        let fresh_complete = matches!(
            fresh,
            Outcome::Complete {
                history_refusals: 0,
                missing_projects: 0,
                blocked_owners: 0,
                ..
            }
        );
        let mut proof = self.evidence.after_delta()?;
        proof.complete &= complete && fresh_complete;
        Ok(proof)
    }
}
impl<E: Evidence, F: FnMut(Event) -> Result<()>> Coordinator<'_, E, F> {
    pub fn first_run(&mut self, source: LaunchSource) -> Result<Handoff> {
        update_handoff::first_run(source, self)
    }
}

/// Existing shipped polling cadence, expressed as monotonic elapsed seconds.
/// No network or main-thread timer in this model; workers call the gated client.
#[derive(Debug)]
pub struct UpdateSchedule {
    next: u64,
}
impl UpdateSchedule {
    pub fn started(now: u64) -> Self {
        Self {
            next: now.saturating_add(30),
        }
    }
    pub fn due(&mut self, now: u64) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now.saturating_add(4 * 60 * 60);
        true
    }
}
