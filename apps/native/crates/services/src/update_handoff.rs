//! First-run distribution handoff. The first-launch coordinator owns all data
//! migration and Keychain access. TR receipts alone never open a native library.
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchSource {
    TransitionRelease,
    DirectDmg,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    WaitingForElectron,
    NeedsSecretConsent,
    NeedsRecovery,
    Blocked,
    Ready,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct DeltaReport {
    pub complete: bool,
    pub six_owner_domains: bool,
    pub state_complete: bool,
    pub secrets_complete: bool,
    pub restore_verified: bool,
    pub source_generation: u64,
    pub checkpoint_generation: u64,
}
/// Host implementations verify signed source bundle/version/designated
/// requirement, hash-bound checkpoint, and exact production/beta Keychain ACL.
/// No plaintext secret or legacy Safe Storage key crosses this interface.
pub trait HandoffHost {
    fn electron_quiescent(&mut self) -> Result<bool>;
    fn verify_transition_receipt(&mut self) -> Result<bool>;
    /// Run pm-first-launch on a worker, preserving source/base/all deltas. Direct
    /// DMG and interrupted Squirrel launches must take exactly the same delta path.
    fn final_delta(&mut self, reuse_transition_evidence: bool) -> Result<DeltaReport>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handoff {
    pub status: HandoffStatus,
    /// Invalid/missing TR evidence chooses the direct importer, never bypasses it.
    pub reuse_transition_evidence: bool,
}
pub fn first_run(source: LaunchSource, host: &mut impl HandoffHost) -> Result<Handoff> {
    if !host.electron_quiescent()? {
        return Ok(Handoff {
            status: HandoffStatus::WaitingForElectron,
            reuse_transition_evidence: false,
        });
    }
    let reuse = source == LaunchSource::TransitionRelease && host.verify_transition_receipt()?;
    let delta = host.final_delta(reuse)?;
    let status = if !delta.complete {
        HandoffStatus::NeedsRecovery
    } else if !delta.secrets_complete {
        HandoffStatus::NeedsSecretConsent
    } else if !delta.six_owner_domains
        || !delta.state_complete
        || !delta.restore_verified
        || delta.source_generation != delta.checkpoint_generation
    {
        HandoffStatus::Blocked
    } else {
        HandoffStatus::Ready
    };
    Ok(Handoff {
        status,
        reuse_transition_evidence: reuse,
    })
}
