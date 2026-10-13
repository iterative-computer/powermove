//! Memory leases and hardware codec sessions for the CLI host (contracts 8,
//! 13, 14). The same rules as Electron main's arbiter
//! (`apps/desktop/src/main/engine/arbiter.ts`): one global budget split
//! between the engines, a shrink in force only once acknowledged, playback
//! moving memory from the job engine to the interactive engine, and
//! per-codec session caps across both engines.

use std::collections::BTreeMap;

use pm_protocol::common::{ByteSize, Seq};
use pm_protocol::error::EngineProcess;
use pm_protocol::host::{HardwareSessionGrant, HardwareSessionKind, HardwareSessionRequest};
use pm_protocol::lease::{Lease, LeaseReason, LeaseRequest, LeaseUsage};

const GIB: u64 = 1 << 30;

/// Half of physical memory, never more than physical memory minus 4 GiB,
/// never less than 1 GiB.
pub fn global_budget(physical: u64) -> u64 {
    (physical / 2)
        .min(physical.saturating_sub(4 * GIB))
        .max(GIB)
}

/// Shares of the global budget, in per-mille, idle and during playback.
const IDLE_SPLIT: [u64; 2] = [600, 400];
const PLAYBACK_SPLIT: [u64; 2] = [850, 150];
pub const ACK_DEADLINE_MS: u32 = 500;

const fn slot(process: EngineProcess) -> usize {
    match process {
        EngineProcess::Interactive => 0,
        EngineProcess::Job => 1,
    }
}

const fn other(process: EngineProcess) -> EngineProcess {
    match process {
        EngineProcess::Interactive => EngineProcess::Job,
        EngineProcess::Job => EngineProcess::Interactive,
    }
}

#[derive(Debug, Clone)]
struct LeaseState {
    current: Lease,
    acknowledged: bool,
    previous: u64,
}

/// A usage report the host must refuse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArbiterError {
    #[error("usage from the {0:?} engine before any lease")]
    NoLease(EngineProcess),
    #[error("usage for epoch {got} but the current epoch is {current}")]
    FutureEpoch { got: Seq, current: Seq },
    #[error("usage total does not equal the category sum")]
    BadTotal,
    #[error("acknowledged {used} bytes over a {granted}-byte lease")]
    AckOverLease { used: u64, granted: u64 },
}

/// One global budget, one lease per engine.
#[derive(Debug, Clone)]
pub struct MemoryArbiter {
    global: u64,
    leases: [Option<LeaseState>; 2],
    epochs: [u64; 2],
    deferred: [Option<(u64, LeaseReason)>; 2],
    playing: bool,
}

impl MemoryArbiter {
    pub fn new(global: u64) -> Self {
        Self {
            global,
            leases: [None, None],
            epochs: [0, 0],
            deferred: [None, None],
            playing: false,
        }
    }

    pub fn global(&self) -> u64 {
        self.global
    }

    fn target(&self, process: EngineProcess) -> u64 {
        let split = if self.playing {
            PLAYBACK_SPLIT
        } else {
            IDLE_SPLIT
        };
        self.global / 1000 * split[slot(process)] + self.global % 1000 * split[slot(process)] / 1000
    }

    fn committed(&self, process: EngineProcess) -> u64 {
        match &self.leases[slot(process)] {
            None => 0,
            Some(s) if s.acknowledged => s.current.bytes.get(),
            Some(s) => s.current.bytes.get().max(s.previous),
        }
    }

    fn make(&mut self, process: EngineProcess, bytes: u64, reason: LeaseReason) -> Lease {
        let epoch = self.epochs[slot(process)] + 1;
        self.epochs[slot(process)] = epoch;
        let previous = self.committed(process);
        let lease = Lease {
            process,
            epoch: Seq::new(epoch).unwrap_or(Seq::MAX),
            bytes: ByteSize::new(bytes).unwrap_or(ByteSize::MAX),
            reason,
            ack_deadline_ms: ACK_DEADLINE_MS,
        };
        self.leases[slot(process)] = Some(LeaseState {
            current: lease,
            acknowledged: false,
            previous,
        });
        lease
    }

    fn grant_up_to(&mut self, process: EngineProcess, bytes: u64, reason: LeaseReason) -> Lease {
        let room = self.global.saturating_sub(self.committed(other(process)));
        self.deferred[slot(process)] = (bytes > room).then_some((bytes, reason));
        self.make(process, bytes.min(room), reason)
    }

    /// A freshly connected engine's first lease. If the other engine grew
    /// past its share while this one was down, it is shrunk back to its
    /// share first (`MemoryPressure`), and this engine's grant grows to its
    /// share once that shrink is acknowledged.
    pub fn initial(&mut self, process: EngineProcess) -> Vec<Lease> {
        let mut grants = Vec::new();
        let peer = other(process);
        let peer_target = self.target(peer);
        if self
            .current(peer)
            .is_some_and(|l| l.bytes.get() > peer_target)
        {
            self.deferred[slot(peer)] = None;
            grants.push(self.make(peer, peer_target, LeaseReason::MemoryPressure));
        }
        let target = self.target(process);
        grants.push(self.grant_up_to(process, target, LeaseReason::Initial));
        grants
    }

    pub fn current(&self, process: EngineProcess) -> Option<&Lease> {
        self.leases[slot(process)].as_ref().map(|s| &s.current)
    }

    pub fn is_acknowledged(&self, process: EngineProcess) -> bool {
        self.leases[slot(process)]
            .as_ref()
            .is_some_and(|s| s.acknowledged)
    }

    /// Applies a usage report; returns leases an acknowledgement unblocked.
    pub fn report(&mut self, usage: &LeaseUsage) -> Result<Vec<Lease>, ArbiterError> {
        let process = usage.process;
        let state = self.leases[slot(process)]
            .as_mut()
            .ok_or(ArbiterError::NoLease(process))?;
        if usage.epoch < state.current.epoch {
            return Ok(Vec::new());
        }
        if usage.epoch > state.current.epoch {
            return Err(ArbiterError::FutureEpoch {
                got: usage.epoch,
                current: state.current.epoch,
            });
        }
        if usage.categories.total() != Some(usage.total) {
            return Err(ArbiterError::BadTotal);
        }
        if usage.ack {
            if usage.total > state.current.bytes {
                return Err(ArbiterError::AckOverLease {
                    used: usage.total.get(),
                    granted: state.current.bytes.get(),
                });
            }
            state.acknowledged = true;
            if let Some((bytes, reason)) = self.deferred[slot(other(process))] {
                return Ok(vec![self.grant_up_to(other(process), bytes, reason)]);
            }
        }
        Ok(Vec::new())
    }

    pub fn playback_started(&mut self) -> Vec<Lease> {
        if self.playing {
            return Vec::new();
        }
        self.playing = true;
        let mut grants = Vec::new();
        if self.leases[1].is_some() {
            let target = self.target(EngineProcess::Job);
            grants.push(self.make(EngineProcess::Job, target, LeaseReason::PlaybackStarted));
        }
        if self.leases[0].is_some() {
            let target = self.target(EngineProcess::Interactive);
            grants.push(self.grant_up_to(
                EngineProcess::Interactive,
                target,
                LeaseReason::PlaybackStarted,
            ));
        }
        grants
    }

    pub fn playback_stopped(&mut self, reason: LeaseReason) -> Vec<Lease> {
        if !self.playing {
            return Vec::new();
        }
        self.playing = false;
        let mut grants = Vec::new();
        if self.leases[0].is_some() {
            let target = self.target(EngineProcess::Interactive);
            grants.push(self.make(EngineProcess::Interactive, target, reason));
        }
        if self.leases[1].is_some() {
            let target = self.target(EngineProcess::Job);
            grants.push(self.grant_up_to(EngineProcess::Job, target, reason));
        }
        grants
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// `requestLease`: what is free next to the other engine, never less
    /// than the current lease.
    pub fn request(&mut self, request: &LeaseRequest) -> Result<Lease, ArbiterError> {
        let process = request.process;
        let current = self
            .current(process)
            .ok_or(ArbiterError::NoLease(process))?
            .bytes
            .get();
        let room = self.global.saturating_sub(self.committed(other(process)));
        let bytes = current.max(request.wanted.get().min(room));
        Ok(self.make(process, bytes, LeaseReason::Requested))
    }

    pub fn exited(&mut self, process: EngineProcess) -> Vec<Lease> {
        self.leases[slot(process)] = None;
        self.deferred[slot(process)] = None;
        let mut grants = if process == EngineProcess::Interactive {
            self.playback_stopped(LeaseReason::PeerExited)
        } else {
            Vec::new()
        };
        if let Some((bytes, reason)) = self.deferred[slot(other(process))] {
            grants.push(self.grant_up_to(other(process), bytes, reason));
        }
        grants
    }
}

/// Concurrent sessions per codec kind across both engines.
pub fn hardware_limit(kind: HardwareSessionKind) -> u16 {
    match kind {
        HardwareSessionKind::Decode => 16,
        HardwareSessionKind::Encode => 2,
        HardwareSessionKind::ProResDecode => 8,
        HardwareSessionKind::ProResEncode => 2,
    }
}

#[derive(Debug, Clone, Default)]
pub struct HardwareArbiter {
    held: BTreeMap<(EngineProcess, u8), u16>,
}

const fn kind_slot(kind: HardwareSessionKind) -> u8 {
    match kind {
        HardwareSessionKind::Decode => 0,
        HardwareSessionKind::Encode => 1,
        HardwareSessionKind::ProResDecode => 2,
        HardwareSessionKind::ProResEncode => 3,
    }
}

impl HardwareArbiter {
    pub fn held_by(&self, process: EngineProcess, kind: HardwareSessionKind) -> u16 {
        self.held
            .get(&(process, kind_slot(kind)))
            .copied()
            .unwrap_or(0)
    }

    pub fn total(&self, kind: HardwareSessionKind) -> u16 {
        self.held_by(EngineProcess::Interactive, kind) + self.held_by(EngineProcess::Job, kind)
    }

    pub fn request(
        &mut self,
        process: EngineProcess,
        request: &HardwareSessionRequest,
    ) -> HardwareSessionGrant {
        let mine = self.held_by(process, request.kind);
        let next = if request.delta > 0 {
            let free = hardware_limit(request.kind).saturating_sub(self.total(request.kind));
            mine + (request.delta as u16).min(free)
        } else {
            mine.saturating_sub(request.delta.unsigned_abs())
        };
        self.held.insert((process, kind_slot(request.kind)), next);
        HardwareSessionGrant {
            kind: request.kind,
            held: next,
        }
    }

    pub fn release_all(&mut self, process: EngineProcess) {
        self.held.retain(|(p, _), _| *p != process);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::lease::LeaseCategories;

    fn usage(process: EngineProcess, lease: &Lease, total: u64, ack: bool) -> LeaseUsage {
        LeaseUsage {
            process,
            epoch: lease.epoch,
            total: ByteSize::new(total).unwrap(),
            categories: LeaseCategories {
                frame_cache: ByteSize::new(total).unwrap(),
                ..Default::default()
            },
            ack,
        }
    }

    #[test]
    fn global_budget_matches_main() {
        assert_eq!(global_budget(8 * GIB), 4 * GIB);
        assert_eq!(global_budget(128 * GIB), 64 * GIB);
        assert_eq!(global_budget(4 * GIB), GIB);
    }

    #[test]
    fn playback_moves_memory_only_after_the_job_engine_gives_it_back() {
        let mut arbiter = MemoryArbiter::new(10_000);
        let i = arbiter.initial(EngineProcess::Interactive)[0];
        let j = arbiter.initial(EngineProcess::Job)[0];
        assert_eq!((i.bytes.get(), j.bytes.get()), (6000, 4000));
        arbiter
            .report(&usage(EngineProcess::Interactive, &i, 5000, true))
            .unwrap();
        arbiter
            .report(&usage(EngineProcess::Job, &j, 3500, true))
            .unwrap();
        let grants = arbiter.playback_started();
        assert_eq!(grants[0].bytes.get(), 1500);
        assert_eq!(grants[1].bytes.get(), 6000, "waits for the job engine");
        let unblocked = arbiter
            .report(&usage(EngineProcess::Job, &grants[0], 1400, true))
            .unwrap();
        assert_eq!(unblocked[0].bytes.get(), 8500);
        assert!(
            arbiter
                .report(&usage(EngineProcess::Job, &grants[0], 1600, true))
                .is_err()
        );
        let after = arbiter.exited(EngineProcess::Interactive);
        assert!(!arbiter.is_playing());
        assert_eq!(after.last().unwrap().reason, LeaseReason::PeerExited);
    }

    #[test]
    fn a_restarted_engine_reclaims_its_share_from_one_that_grew_meanwhile() {
        let mut arbiter = MemoryArbiter::new(10_000);
        let i = arbiter.initial(EngineProcess::Interactive)[0];
        let j = arbiter.initial(EngineProcess::Job)[0];
        arbiter
            .report(&usage(EngineProcess::Interactive, &i, 100, true))
            .unwrap();
        arbiter
            .report(&usage(EngineProcess::Job, &j, 100, true))
            .unwrap();
        arbiter.exited(EngineProcess::Job);
        let grown = arbiter
            .request(&LeaseRequest {
                process: EngineProcess::Interactive,
                wanted: ByteSize::new(9900).unwrap(),
                reason: LeaseReason::Requested,
            })
            .unwrap();
        assert_eq!(grown.bytes.get(), 9900);
        arbiter
            .report(&usage(EngineProcess::Interactive, &grown, 9000, true))
            .unwrap();
        let back = arbiter.initial(EngineProcess::Job);
        let summary: Vec<_> = back
            .iter()
            .map(|l| (l.process, l.bytes.get(), l.reason))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    EngineProcess::Interactive,
                    6000,
                    LeaseReason::MemoryPressure
                ),
                (EngineProcess::Job, 100, LeaseReason::Initial),
            ]
        );
        let unblocked = arbiter
            .report(&usage(EngineProcess::Interactive, &back[0], 5800, true))
            .unwrap();
        assert_eq!(
            unblocked
                .iter()
                .map(|l| (l.process, l.bytes.get(), l.reason))
                .collect::<Vec<_>>(),
            vec![(EngineProcess::Job, 4000, LeaseReason::Initial)]
        );
        // An engine within its share is left alone.
        arbiter.exited(EngineProcess::Job);
        assert_eq!(arbiter.initial(EngineProcess::Job).len(), 1);
    }

    #[test]
    fn codec_sessions_are_capped_across_engines() {
        let mut hw = HardwareArbiter::default();
        let req = |delta| HardwareSessionRequest {
            kind: HardwareSessionKind::Encode,
            delta,
        };
        assert_eq!(hw.request(EngineProcess::Job, &req(1)).held, 1);
        assert_eq!(hw.request(EngineProcess::Interactive, &req(5)).held, 1);
        assert_eq!(hw.request(EngineProcess::Job, &req(-4)).held, 0);
        hw.release_all(EngineProcess::Interactive);
        assert_eq!(hw.total(HardwareSessionKind::Encode), 0);
    }
}
