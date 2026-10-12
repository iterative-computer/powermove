//! Budget leases (contract 8, section 19 "Memory and GPU").
//!
//! Main grants each engine a memory lease from one global budget and shrinks
//! the job engine's lease during playback. Each engine enforces its lease
//! across frame cache, render cache, Blender surfaces, 3D geometry, surface
//! pool, decoders, and pipelines. A shrink is acknowledged once usage fits.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{ByteSize, Seq};
use crate::error::EngineProcess;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Why a lease changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LeaseReason {
    Initial,
    PlaybackStarted,
    PlaybackStopped,
    MemoryPressure,
    Requested,
    PeerExited,
}

/// A memory allowance from the host. Epochs increase with every change;
/// only the latest epoch is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub process: EngineProcess,
    pub epoch: Seq,
    /// Total bytes the process may hold across every category (unified memory).
    pub bytes: ByteSize,
    pub reason: LeaseReason,
    /// When the lease shrinks, the engine evicts and acknowledges within this time.
    pub ack_deadline_ms: u32,
}

/// Bytes held per category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaseCategories {
    pub frame_cache: ByteSize,
    pub render_cache: ByteSize,
    pub blender_surfaces: ByteSize,
    pub geometry_3d: ByteSize,
    pub surface_pool: ByteSize,
    pub decoders: ByteSize,
    pub pipelines: ByteSize,
    pub other: ByteSize,
}

impl LeaseCategories {
    /// The sum, or `None` if it leaves the safe integer range.
    pub fn total(&self) -> Option<ByteSize> {
        let parts = [
            self.frame_cache,
            self.render_cache,
            self.blender_surfaces,
            self.geometry_3d,
            self.surface_pool,
            self.decoders,
            self.pipelines,
            self.other,
        ];
        let sum = parts
            .iter()
            .try_fold(0u64, |acc, part| acc.checked_add(part.get()))?;
        ByteSize::new(sum).ok()
    }
}

/// Engine usage against a lease epoch. Sent as the acknowledgement of a
/// lease change and periodically (at least every 5 s while non-idle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaseUsage {
    pub process: EngineProcess,
    pub epoch: Seq,
    pub total: ByteSize,
    pub categories: LeaseCategories,
    /// True when this report acknowledges `epoch` (usage fits the lease).
    pub ack: bool,
}

/// Engine to host: ask for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaseRequest {
    pub process: EngineProcess,
    pub wanted: ByteSize,
    pub reason: LeaseReason,
}

impl Validate for LeaseUsage {
    fn validate(&self) -> Result<(), Violation> {
        ensure(
            self.categories.total() == Some(self.total),
            ContractId::Lease,
            || {
                format!(
                    "usage total {} does not equal the category sum {:?}",
                    self.total,
                    self.categories.total()
                )
            },
        )
    }
}

impl Validate for Lease {
    fn validate(&self) -> Result<(), Violation> {
        ensure(self.ack_deadline_ms > 0, ContractId::Lease, || {
            "a lease needs an acknowledgement deadline".into()
        })
    }
}

/// Host-side view of one engine's lease: tracks epochs and checks that
/// acknowledgements fit.
#[derive(Debug, Clone)]
pub struct LeaseLedger {
    current: Lease,
    acknowledged: Option<Seq>,
}

/// A usage report the host must act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    #[error("usage for epoch {got} but the current epoch is {current}")]
    StaleEpoch { got: Seq, current: Seq },
    #[error("acknowledged epoch {epoch} with {used} bytes over a {granted}-byte lease")]
    AckOverLease {
        epoch: Seq,
        used: ByteSize,
        granted: ByteSize,
    },
    #[error("lease epochs must increase ({previous} then {next})")]
    EpochNotIncreasing { previous: Seq, next: Seq },
}

impl LeaseLedger {
    pub fn new(initial: Lease) -> Self {
        Self {
            current: initial,
            acknowledged: None,
        }
    }

    pub fn current(&self) -> &Lease {
        &self.current
    }

    pub fn is_acknowledged(&self) -> bool {
        self.acknowledged == Some(self.current.epoch)
    }

    /// Replaces the lease with a newer epoch.
    pub fn grant(&mut self, next: Lease) -> Result<(), LeaseError> {
        if next.epoch <= self.current.epoch {
            return Err(LeaseError::EpochNotIncreasing {
                previous: self.current.epoch,
                next: next.epoch,
            });
        }
        self.current = next;
        Ok(())
    }

    /// Applies a usage report. Reports for older epochs are stale; an
    /// acknowledgement must fit the lease.
    pub fn report(&mut self, usage: &LeaseUsage) -> Result<(), LeaseError> {
        if usage.epoch != self.current.epoch {
            return Err(LeaseError::StaleEpoch {
                got: usage.epoch,
                current: self.current.epoch,
            });
        }
        if usage.ack {
            if usage.total > self.current.bytes {
                return Err(LeaseError::AckOverLease {
                    epoch: usage.epoch,
                    used: usage.total,
                    granted: self.current.bytes,
                });
            }
            self.acknowledged = Some(usage.epoch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(n: u64) -> ByteSize {
        ByteSize::new(n).unwrap()
    }

    fn lease(epoch: u64, size: u64) -> Lease {
        Lease {
            process: EngineProcess::Job,
            epoch: Seq::new(epoch).unwrap(),
            bytes: bytes(size),
            reason: LeaseReason::Initial,
            ack_deadline_ms: 500,
        }
    }

    fn usage(epoch: u64, frame_cache: u64, ack: bool) -> LeaseUsage {
        let categories = LeaseCategories {
            frame_cache: bytes(frame_cache),
            ..Default::default()
        };
        LeaseUsage {
            process: EngineProcess::Job,
            epoch: Seq::new(epoch).unwrap(),
            total: categories.total().unwrap(),
            categories,
            ack,
        }
    }

    #[test]
    fn totals_must_equal_the_category_sum() {
        let mut report = usage(1, 100, false);
        assert!(report.validate().is_ok());
        report.total = bytes(99);
        assert!(report.validate().is_err());
        let overflow = LeaseCategories {
            frame_cache: ByteSize::MAX,
            render_cache: bytes(1),
            ..Default::default()
        };
        assert_eq!(overflow.total(), None);
    }

    #[test]
    fn shrink_is_acknowledged_only_when_usage_fits() {
        let mut ledger = LeaseLedger::new(lease(1, 1000));
        ledger.report(&usage(1, 800, true)).unwrap();
        assert!(ledger.is_acknowledged());
        ledger
            .grant(Lease {
                reason: LeaseReason::PlaybackStarted,
                ..lease(2, 500)
            })
            .unwrap();
        assert!(!ledger.is_acknowledged());
        assert!(matches!(
            ledger.report(&usage(1, 400, true)),
            Err(LeaseError::StaleEpoch { .. })
        ));
        assert!(matches!(
            ledger.report(&usage(2, 800, true)),
            Err(LeaseError::AckOverLease { .. })
        ));
        ledger.report(&usage(2, 800, false)).unwrap();
        ledger.report(&usage(2, 450, true)).unwrap();
        assert!(ledger.is_acknowledged());
        assert!(matches!(
            ledger.grant(lease(2, 900)),
            Err(LeaseError::EpochNotIncreasing { .. })
        ));
    }
}
