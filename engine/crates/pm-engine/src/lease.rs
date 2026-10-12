//! Budget leases inside an engine (contract 8, section 19 "Memory and GPU").
//!
//! The host grants each engine a lease; the engine enforces it across every
//! category it holds. [`LeaseBudget`] is the engine's ledger: owners charge
//! bytes to a category before they allocate and release them after they
//! free; a charge that would exceed the lease first asks the caches to evict
//! (through [`Evict`]) and fails with [`BudgetExceeded`] when they cannot
//! make room. A shrinking grant is acknowledged only once usage fits.

use pm_protocol::common::{ByteSize, HostMicros, Seq};
use pm_protocol::error::EngineProcess;
use pm_protocol::lease::{Lease, LeaseCategories, LeaseReason, LeaseRequest, LeaseUsage};

use crate::clock;

/// Memory categories an engine accounts for (contract 8, `LeaseCategories`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    FrameCache,
    RenderCache,
    BlenderSurfaces,
    Geometry3d,
    SurfacePool,
    Decoders,
    Pipelines,
    Other,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::FrameCache,
        Category::RenderCache,
        Category::BlenderSurfaces,
        Category::Geometry3d,
        Category::SurfacePool,
        Category::Decoders,
        Category::Pipelines,
        Category::Other,
    ];

    /// The order caches are asked to evict in: cheapest to rebuild first. The
    /// surface pool is last because its slots are in use by the viewer.
    pub const EVICTION_ORDER: [Category; 8] = [
        Category::FrameCache,
        Category::RenderCache,
        Category::BlenderSurfaces,
        Category::Geometry3d,
        Category::Decoders,
        Category::Pipelines,
        Category::Other,
        Category::SurfacePool,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Something that can free memory on request: the frame cache, the render
/// cache, decoder pools. Returns the bytes actually freed. The budget
/// releases those bytes itself, so an evictor must not also call
/// [`LeaseBudget::release`] for them.
pub trait Evict {
    fn evict(&mut self, category: Category, bytes: u64) -> u64;
}

/// Nothing to evict (an engine with no caches yet, and tests).
pub struct NoEviction;

impl Evict for NoEviction {
    fn evict(&mut self, _: Category, _: u64) -> u64 {
        0
    }
}

/// A charge that does not fit the lease even after eviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{category:?} needs {wanted} bytes; {used} of {granted} are in use")]
pub struct BudgetExceeded {
    pub category: Category,
    pub wanted: u64,
    pub used: u64,
    pub granted: u64,
}

/// A grant the engine cannot take.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrantError {
    #[error("the lease is for the {got:?} engine, this is the {expected:?} engine")]
    WrongProcess {
        expected: EngineProcess,
        got: EngineProcess,
    },
    #[error("lease epochs must increase ({previous} then {next})")]
    StaleEpoch { previous: Seq, next: Seq },
}

/// Reports go out at least this often while the engine is not idle.
pub const REPORT_INTERVAL_US: u64 = 5_000_000;

/// The engine's side of its lease.
#[derive(Debug, Clone)]
pub struct LeaseBudget {
    process: EngineProcess,
    lease: Option<Lease>,
    granted_at: Option<HostMicros>,
    used: [u64; 8],
    /// The epoch whose acknowledgement is owed (usage did not fit yet).
    owed_ack: Option<Seq>,
    last_report: Option<HostMicros>,
}

impl LeaseBudget {
    /// Before the first grant the engine may hold nothing.
    pub fn new(process: EngineProcess) -> Self {
        Self {
            process,
            lease: None,
            granted_at: None,
            used: [0; 8],
            owed_ack: None,
            last_report: None,
        }
    }

    pub fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// Granted bytes (zero before the first grant).
    pub fn granted(&self) -> u64 {
        self.lease.map_or(0, |lease| lease.bytes.get())
    }

    pub fn used(&self) -> u64 {
        self.used.iter().sum()
    }

    pub fn used_in(&self, category: Category) -> u64 {
        self.used[category.index()]
    }

    pub fn fits(&self) -> bool {
        self.used() <= self.granted()
    }

    /// Takes a new lease, evicting until usage fits. Returns the usage report
    /// that answers the grant: `ack: true` when usage fits, otherwise
    /// `ack: false` with the acknowledgement owed until [`release`]
    /// brings usage within the lease (then [`poll_report`] sends it).
    ///
    /// [`release`]: LeaseBudget::release
    /// [`poll_report`]: LeaseBudget::poll_report
    pub fn grant(
        &mut self,
        lease: Lease,
        now: HostMicros,
        evict: &mut dyn Evict,
    ) -> Result<LeaseUsage, GrantError> {
        if lease.process != self.process {
            return Err(GrantError::WrongProcess {
                expected: self.process,
                got: lease.process,
            });
        }
        if let Some(current) = self.lease
            && lease.epoch <= current.epoch
        {
            return Err(GrantError::StaleEpoch {
                previous: current.epoch,
                next: lease.epoch,
            });
        }
        self.lease = Some(lease);
        self.granted_at = Some(now);
        self.fit(evict);
        let ack = self.fits();
        self.owed_ack = (!ack).then_some(lease.epoch);
        self.last_report = Some(now);
        Ok(self.usage(ack))
    }

    /// Asks the caches to evict until usage fits the lease, in
    /// [`Category::EVICTION_ORDER`]. Freed bytes are released from the budget.
    pub fn fit(&mut self, evict: &mut dyn Evict) {
        for category in Category::EVICTION_ORDER {
            let over = self.used().saturating_sub(self.granted());
            if over == 0 {
                return;
            }
            let have = self.used_in(category);
            if have == 0 {
                continue;
            }
            let freed = evict.evict(category, over.min(have)).min(have);
            self.used[category.index()] -= freed;
        }
    }

    /// Charges `bytes` to `category` if they fit the lease as it is.
    pub fn try_charge(&mut self, category: Category, bytes: u64) -> Result<(), BudgetExceeded> {
        let used = self.used();
        if used.saturating_add(bytes) > self.granted() {
            return Err(BudgetExceeded {
                category,
                wanted: bytes,
                used,
                granted: self.granted(),
            });
        }
        self.used[category.index()] += bytes;
        Ok(())
    }

    /// Charges `bytes` to `category`, evicting other memory first when the
    /// lease is full. Never evicts from `category` itself (the caller is
    /// about to use it).
    pub fn charge(
        &mut self,
        category: Category,
        bytes: u64,
        evict: &mut dyn Evict,
    ) -> Result<(), BudgetExceeded> {
        if self.try_charge(category, bytes).is_ok() {
            return Ok(());
        }
        for victim in Category::EVICTION_ORDER {
            if victim == category {
                continue;
            }
            let need = self
                .used()
                .saturating_add(bytes)
                .saturating_sub(self.granted());
            if need == 0 {
                break;
            }
            let have = self.used_in(victim);
            if have == 0 {
                continue;
            }
            let freed = evict.evict(victim, need.min(have)).min(have);
            self.used[victim.index()] -= freed;
        }
        self.try_charge(category, bytes)
    }

    /// Releases bytes an owner freed. Releasing more than was charged is a bug
    /// in the owner; the category floors at zero.
    pub fn release(&mut self, category: Category, bytes: u64) {
        let slot = &mut self.used[category.index()];
        debug_assert!(*slot >= bytes, "{category:?} released more than it holds");
        *slot = slot.saturating_sub(bytes);
    }

    /// The usage report for the current epoch.
    pub fn usage(&self, ack: bool) -> LeaseUsage {
        let b = |category: Category| ByteSize::new(self.used_in(category)).unwrap_or(ByteSize::MAX);
        let categories = LeaseCategories {
            frame_cache: b(Category::FrameCache),
            render_cache: b(Category::RenderCache),
            blender_surfaces: b(Category::BlenderSurfaces),
            geometry_3d: b(Category::Geometry3d),
            surface_pool: b(Category::SurfacePool),
            decoders: b(Category::Decoders),
            pipelines: b(Category::Pipelines),
            other: b(Category::Other),
        };
        LeaseUsage {
            process: self.process,
            epoch: self.lease.map_or(Seq::ZERO, |lease| lease.epoch),
            total: categories.total().unwrap_or(ByteSize::MAX),
            categories,
            ack,
        }
    }

    /// Whether an acknowledgement is owed and its deadline has passed.
    pub fn ack_overdue(&self, now: HostMicros) -> bool {
        match (self.owed_ack, self.lease, self.granted_at) {
            (Some(_), Some(lease), Some(at)) => {
                clock::since(now, at) >= u64::from(lease.ack_deadline_ms) * 1_000
            }
            _ => false,
        }
    }

    /// The epoch whose acknowledgement is still owed.
    pub fn owed_ack(&self) -> Option<Seq> {
        self.owed_ack
    }

    /// A report due now: the owed acknowledgement once usage fits, or the
    /// periodic report every [`REPORT_INTERVAL_US`] while `active`.
    pub fn poll_report(&mut self, now: HostMicros, active: bool) -> Option<LeaseUsage> {
        self.lease?;
        if self.owed_ack.is_some() && self.fits() {
            self.owed_ack = None;
            self.last_report = Some(now);
            return Some(self.usage(true));
        }
        let due = self
            .last_report
            .is_none_or(|last| clock::since(now, last) >= REPORT_INTERVAL_US);
        if active && due {
            self.last_report = Some(now);
            return Some(self.usage(self.owed_ack.is_none() && self.fits()));
        }
        None
    }

    /// A request for enough to hold `extra` more bytes.
    pub fn request_more(&self, extra: u64) -> LeaseRequest {
        LeaseRequest {
            process: self.process,
            wanted: ByteSize::new(self.used().saturating_add(extra)).unwrap_or(ByteSize::MAX),
            reason: LeaseReason::Requested,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(epoch: u64, bytes: u64) -> Lease {
        Lease {
            process: EngineProcess::Job,
            epoch: Seq::new(epoch).unwrap(),
            bytes: ByteSize::new(bytes).unwrap(),
            reason: LeaseReason::Initial,
            ack_deadline_ms: 500,
        }
    }

    fn at(us: u64) -> HostMicros {
        HostMicros::new(us).unwrap()
    }

    /// A cache holding bytes per category that frees what it is asked for.
    struct Caches {
        frees: Vec<(Category, u64)>,
        limit: u64,
    }

    impl Evict for Caches {
        fn evict(&mut self, category: Category, bytes: u64) -> u64 {
            let freed = bytes.min(self.limit);
            self.frees.push((category, freed));
            freed
        }
    }

    #[test]
    fn nothing_is_charged_before_the_first_grant() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        assert!(budget.try_charge(Category::Other, 1).is_err());
        assert!(budget.poll_report(at(0), true).is_none());
    }

    #[test]
    fn charges_within_the_lease_and_fails_beyond_it() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        let usage = budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        assert!(usage.ack);
        budget.try_charge(Category::FrameCache, 600).unwrap();
        budget.try_charge(Category::Decoders, 400).unwrap();
        let error = budget.try_charge(Category::Pipelines, 1).unwrap_err();
        assert_eq!(error.used, 1000);
        budget.release(Category::FrameCache, 100);
        budget.try_charge(Category::Pipelines, 100).unwrap();
        let usage = budget.usage(true);
        assert_eq!(usage.total.get(), 1000);
        assert_eq!(usage.categories.pipelines.get(), 100);
        assert!(pm_protocol::Validate::validate(&usage).is_ok());
    }

    #[test]
    fn charging_evicts_other_categories_in_order() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        budget.try_charge(Category::RenderCache, 300).unwrap();
        budget.try_charge(Category::FrameCache, 500).unwrap();
        let mut caches = Caches {
            frees: vec![],
            limit: u64::MAX,
        };
        budget.charge(Category::Decoders, 400, &mut caches).unwrap();
        assert_eq!(caches.frees, vec![(Category::FrameCache, 200)]);
        assert_eq!(budget.used(), 1000);
        // The category being charged is never its own victim.
        let mut stingy = Caches {
            frees: vec![],
            limit: 0,
        };
        assert!(budget.charge(Category::FrameCache, 10, &mut stingy).is_err());
        assert!(!stingy.frees.iter().any(|(c, _)| *c == Category::FrameCache));
    }

    #[test]
    fn a_shrink_is_acknowledged_once_usage_fits() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        budget.try_charge(Category::SurfacePool, 800).unwrap();
        // Shrink to 500: nothing evictable outside the pool.
        let usage = budget
            .grant(
                Lease {
                    reason: LeaseReason::PlaybackStarted,
                    ..lease(2, 500)
                },
                at(10),
                &mut NoEviction,
            )
            .unwrap();
        assert!(!usage.ack);
        assert_eq!(budget.owed_ack(), Some(Seq::new(2).unwrap()));
        assert!(!budget.ack_overdue(at(400_000)));
        assert!(budget.ack_overdue(at(500_010)));
        assert!(budget.poll_report(at(20), false).is_none());
        budget.release(Category::SurfacePool, 300);
        let ack = budget.poll_report(at(30), false).unwrap();
        assert!(ack.ack);
        assert_eq!(ack.epoch.get(), 2);
        assert_eq!(budget.owed_ack(), None);
        // Stale and foreign grants are refused.
        assert!(matches!(
            budget.grant(lease(2, 900), at(40), &mut NoEviction),
            Err(GrantError::StaleEpoch { .. })
        ));
        let foreign = Lease {
            process: EngineProcess::Interactive,
            ..lease(3, 900)
        };
        assert!(matches!(
            budget.grant(foreign, at(40), &mut NoEviction),
            Err(GrantError::WrongProcess { .. })
        ));
    }

    #[test]
    fn a_shrink_evicts_caches_before_answering() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        budget.try_charge(Category::FrameCache, 300).unwrap();
        budget.try_charge(Category::SurfacePool, 400).unwrap();
        let mut caches = Caches {
            frees: vec![],
            limit: u64::MAX,
        };
        let usage = budget.grant(lease(2, 500), at(1), &mut caches).unwrap();
        assert!(usage.ack);
        assert_eq!(caches.frees, vec![(Category::FrameCache, 200)]);
        assert_eq!(usage.total.get(), 500);
    }

    #[test]
    fn periodic_reports_every_five_seconds_while_active() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        assert!(budget.poll_report(at(1_000_000), true).is_none());
        assert!(budget.poll_report(at(5_000_000), false).is_none(), "idle");
        let report = budget.poll_report(at(5_000_000), true).unwrap();
        assert!(report.ack);
        assert!(budget.poll_report(at(6_000_000), true).is_none());
        assert!(budget.poll_report(at(10_000_000), true).is_some());
    }

    #[test]
    fn request_more_asks_for_usage_plus_extra() {
        let mut budget = LeaseBudget::new(EngineProcess::Job);
        budget.grant(lease(1, 1000), at(0), &mut NoEviction).unwrap();
        budget.try_charge(Category::Other, 700).unwrap();
        let request = budget.request_more(500);
        assert_eq!(request.wanted.get(), 1200);
        assert_eq!(request.reason, LeaseReason::Requested);
    }
}
