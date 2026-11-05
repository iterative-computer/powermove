//! Editing-side render cache/task lifecycle. One worker owns this model and its
//! IO adapter; it never owns a document writer. The media/engine lane supplies
//! native rendering, byte verification, durable storage and process draining.
//! No IO method here is suitable for the AppKit/GPUI or kernel actor thread.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Provenance {
    Originals,
    Proxy,
    Optimized,
}
/// Complete semantic identity supplied by the render host, not a guessed hash
/// of document revision alone. Media includes actual selected content hashes;
/// graph includes effects, colour/LUT bytes, fonts and implementation versions.
/// Provenance is part of equality: a proxy hit can never satisfy Originals.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrameKey {
    pub graph: [u8; 32],
    pub media: [u8; 32],
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub renderer: String,
    pub provenance: Provenance,
}
impl FrameKey {
    fn valid(&self) -> bool {
        self.ticks >= 0
            && self.width > 0
            && self.height > 0
            && !self.format.is_empty()
            && !self.renderer.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub project: String,
    pub document: u64,
    pub revision: u64,
    /// Monotone native media/font/effect/colour-resource generation. Relink and
    /// proxy selection can change this without a document revision.
    pub resources: u64,
}
impl Version {
    pub fn from_published(project: &str, p: &crate::Published, resources: u64) -> Self {
        Self {
            project: project.into(),
            document: p.snapshot.doc_id,
            revision: p.revision(),
            resources,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPlan {
    pub version: Version,
    pub comp_id: String,
    pub key: FrameKey,
    /// Reserved output budget, including file/container overhead. This is not a
    /// claim about physical free disk; the host must check its volume too.
    pub reserve_bytes: u64,
    pub require_originals: bool,
    pub total_frames: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameReceipt {
    pub key: FrameKey,
    pub bytes: u64,
    pub checksum: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupReason {
    Cancelled,
    Superseded,
    Interrupted,
    LowDisk,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    CancelRequested(CleanupReason),
    /// Durable intent precedes draining and archiving. Failures remain here and
    /// retry cleanup; they never turn into a falsely acknowledged terminal row.
    Cleaning(CleanupReason),
    Completed(FrameReceipt),
    Cancelled,
    Interrupted,
    Failed(CleanupReason),
}
impl JobState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed(_) | Self::Cancelled | Self::Interrupted | Self::Failed(_)
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    /// Host-created fresh durable id, never a path. Reuse is refused even after
    /// completion; archive the record before retiring it at the host boundary.
    pub id: String,
    pub plan: RenderPlan,
    pub state: JobState,
    pub completed_frames: u64,
    /// Worker diagnostic, not presentation copy.
    pub diagnostic: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    DiskFull,
    Unavailable(String),
    Io(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobError {
    Unknown,
    Duplicate,
    InvalidPlan,
    InvalidProgress,
    InvalidTransition,
    StaleTicket,
    StaleVersion,
    OriginalsRequired,
    InvalidReceipt,
    Host(HostError),
}
impl From<HostError> for JobError {
    fn from(e: HostError) -> Self {
        Self::Host(e)
    }
}
/// Adapter contract: exclusively owned private roots and native volume leases.
/// `save` atomically persists a full record (including key/provenance) and fsyncs
/// before success. The host must load every retained record on startup, including
/// terminal ones, so identifiers are never reused. No serde/disk format is frozen
/// here; existing media/task persistence is the authority at integration time.
pub trait RenderJobHost {
    fn save(&mut self, record: &JobRecord) -> Result<(), HostError>;
    /// Make/reserve private staging for this id; check actual physical free
    /// space plus the host's safety floor. Never evict originals to make room.
    fn begin(&mut self, record: &JobRecord) -> Result<(), HostError>;
    /// Stop/wait/reap all producers and release output handles. Idempotent;
    /// cancellation cannot be acknowledged while a producer can still write.
    fn drain(&mut self, record: &JobRecord) -> Result<(), HostError>;
    /// Archive private partials without replacing any existing artifact. Verify
    /// and sync the archive before retiring active names. A low-disk archive
    /// failure leaves staging intact and returns an error; do not delete it.
    /// Must also be idempotent across a crash after archival but before save.
    fn archive(&mut self, record: &JobRecord, reason: CleanupReason) -> Result<(), HostError>;
    /// Verify real source provenance, frame identity/dimensions/representation,
    /// checksum and byte count. Publish immutable cache bytes without clobber.
    /// Drain the successful producer first. Publication is idempotent for equal
    /// bytes; any failure preserves partials for `archive`/startup recovery.
    fn publish(&mut self, record: &JobRecord) -> Result<FrameReceipt, HostError>;
    /// Read/checksum the actual cache object for this exact semantic key. A bar
    /// snapshot or a remembered completed task is not proof of available bytes.
    fn cached(&mut self, key: &FrameKey) -> Result<Option<FrameReceipt>, HostError>;
    /// Existing cache adapter persists explicit Keep Rendered state. Missing
    /// and corrupt objects must refuse; this never pins an absent key.
    fn keep(&mut self, key: &FrameKey, keep: bool) -> Result<(), HostError>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    id: String,
    epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheState {
    Missing,
    Rendering,
    Available(FrameReceipt),
}
/// Publication always follows successful `host.save`. A storage error is never
/// hidden by an in-memory terminal state. Recover an interrupted save from the
/// host's last durable record; retained stage bytes remain inspectable.
pub struct RenderJobs {
    epoch: u64,
    records: BTreeMap<String, JobRecord>,
    current: BTreeMap<String, Version>,
    retired: BTreeSet<(String, u64)>,
}
impl Default for RenderJobs {
    fn default() -> Self {
        static EPOCH: AtomicU64 = AtomicU64::new(1);
        let epoch = EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("render job lifetime exhausted");
        Self {
            epoch,
            records: BTreeMap::new(),
            current: BTreeMap::new(),
            retired: BTreeSet::new(),
        }
    }
}
impl RenderJobs {
    pub fn record(&self, id: &str) -> Option<&JobRecord> {
        self.records.get(id)
    }
    pub fn records(&self) -> impl Iterator<Item = &JobRecord> {
        self.records.values()
    }
    fn save(&mut self, record: JobRecord, host: &mut impl RenderJobHost) -> Result<(), JobError> {
        host.save(&record)?;
        self.records.insert(record.id.clone(), record);
        Ok(())
    }
    /// Called from current kernel publications. Retired/reopened document work
    /// is cancelled rather than attached to a new project lifetime. Revisions
    /// may increase after Undo even when content returns to an earlier value.
    pub fn observe(
        &mut self,
        version: Version,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        if self
            .retired
            .contains(&(version.project.clone(), version.document))
        {
            return Err(JobError::StaleVersion);
        }
        if version.project.is_empty() {
            return Err(JobError::InvalidPlan);
        }
        if self.current.get(&version.project).is_some_and(|old| {
            old.document == version.document
                && (old.revision > version.revision || old.resources > version.resources)
        }) {
            return Err(JobError::StaleVersion);
        }
        if let Some(old) = self.current.get(&version.project)
            && old.document != version.document
        {
            self.retired.insert((old.project.clone(), old.document));
        }
        // Install the fence before cancellation persistence. If save fails, a
        // late completion is still stale and startup sees the old live record.
        self.current
            .insert(version.project.clone(), version.clone());
        let ids: Vec<_> = self
            .records
            .values()
            .filter(|r| {
                r.plan.version.project == version.project
                    && r.plan.version != version
                    && !r.state.terminal()
            })
            .map(|r| r.id.clone())
            .collect();
        for id in ids {
            self.cancel_with(&id, CleanupReason::Superseded, host)?;
        }
        Ok(())
    }
    pub fn enqueue(
        &mut self,
        id: String,
        plan: RenderPlan,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        if self.records.contains_key(&id) {
            return Err(JobError::Duplicate);
        }
        if id.is_empty()
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || plan.comp_id.is_empty()
            || !plan.key.valid()
            || plan.reserve_bytes == 0
            || plan.total_frames == Some(0)
        {
            return Err(JobError::InvalidPlan);
        }
        if plan.require_originals && plan.key.provenance != Provenance::Originals {
            return Err(JobError::OriginalsRequired);
        }
        if self.current.get(&plan.version.project) != Some(&plan.version) {
            return Err(JobError::StaleVersion);
        }
        self.save(
            JobRecord {
                id,
                plan,
                state: JobState::Queued,
                completed_frames: 0,
                diagnostic: None,
            },
            host,
        )
    }
    pub fn start(&mut self, id: &str, host: &mut impl RenderJobHost) -> Result<Ticket, JobError> {
        let mut record = self.records.get(id).ok_or(JobError::Unknown)?.clone();
        if record.state != JobState::Queued {
            return Err(JobError::InvalidTransition);
        }
        if self.current.get(&record.plan.version.project) != Some(&record.plan.version) {
            return Err(JobError::StaleVersion);
        }
        record.state = JobState::Running;
        self.save(record.clone(), host)?;
        if let Err(e) = host.begin(&record) {
            self.fail(id, e.clone(), host)?;
            return Err(JobError::Host(e));
        }
        Ok(Ticket {
            id: id.into(),
            epoch: self.epoch,
        })
    }
    fn ticket(&self, ticket: &Ticket) -> Result<&JobRecord, JobError> {
        if ticket.epoch != self.epoch {
            return Err(JobError::StaleTicket);
        }
        self.records.get(&ticket.id).ok_or(JobError::StaleTicket)
    }
    pub fn progress(
        &mut self,
        ticket: &Ticket,
        completed: u64,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        let mut record = self.ticket(ticket)?.clone();
        if record.state != JobState::Running {
            return Err(JobError::InvalidTransition);
        }
        if self.current.get(&record.plan.version.project) != Some(&record.plan.version) {
            return Err(JobError::StaleVersion);
        }
        if completed < record.completed_frames
            || record
                .plan
                .total_frames
                .is_some_and(|total| completed > total)
        {
            return Err(JobError::InvalidProgress);
        }
        record.completed_frames = completed;
        self.save(record, host)
    }
    pub fn cancel(&mut self, id: &str, host: &mut impl RenderJobHost) -> Result<(), JobError> {
        self.cancel_with(id, CleanupReason::Cancelled, host)
    }
    fn cancel_with(
        &mut self,
        id: &str,
        reason: CleanupReason,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        let mut record = self.records.get(id).ok_or(JobError::Unknown)?.clone();
        if record.state.terminal() {
            return Err(JobError::InvalidTransition);
        }
        if matches!(
            record.state,
            JobState::CancelRequested(_) | JobState::Cleaning(_)
        ) {
            return Ok(());
        }
        record.state = JobState::CancelRequested(reason);
        self.save(record, host)
    }
    /// Finish a cancel only after the adapter has drained workers and archived
    /// outputs. Returns errors with `Cleaning` still inspectable and retryable.
    pub fn settle(&mut self, id: &str, host: &mut impl RenderJobHost) -> Result<(), JobError> {
        let mut record = self.records.get(id).ok_or(JobError::Unknown)?.clone();
        let reason = match record.state {
            JobState::CancelRequested(r) | JobState::Cleaning(r) => r,
            _ => return Err(JobError::InvalidTransition),
        };
        record.state = JobState::Cleaning(reason);
        self.save(record.clone(), host)?;
        host.drain(&record)?;
        host.archive(&record, reason)?;
        record.state = match reason {
            CleanupReason::Cancelled => JobState::Cancelled,
            CleanupReason::Interrupted => JobState::Interrupted,
            r => JobState::Failed(r),
        };
        self.save(record, host)
    }
    fn fail(
        &mut self,
        id: &str,
        e: HostError,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        let mut record = self.records.get(id).ok_or(JobError::Unknown)?.clone();
        record.state = JobState::Cleaning(if e == HostError::DiskFull {
            CleanupReason::LowDisk
        } else {
            CleanupReason::Failed
        });
        record.diagnostic = Some(format!("{e:?}"));
        self.save(record, host)?;
        self.settle(id, host)
    }
    pub fn complete(
        &mut self,
        ticket: &Ticket,
        host: &mut impl RenderJobHost,
    ) -> Result<FrameReceipt, JobError> {
        let mut record = self.ticket(ticket)?.clone();
        if record.state != JobState::Running {
            return Err(JobError::InvalidTransition);
        }
        if self.current.get(&record.plan.version.project) != Some(&record.plan.version) {
            return Err(JobError::StaleVersion);
        }
        // A successful producer must also release its handles before immutable
        // publication. Drain errors preserve Running for retry/recovery.
        host.drain(&record)?;
        let receipt = match host.publish(&record) {
            Ok(v) => v,
            Err(e) => {
                self.fail(&record.id, e.clone(), host)?;
                return Err(JobError::Host(e));
            }
        };
        if receipt.key != record.plan.key
            || receipt.bytes == 0
            || receipt.bytes > record.plan.reserve_bytes
        {
            self.fail(
                &record.id,
                HostError::Unavailable("Invalid render receipt".into()),
                host,
            )?;
            return Err(JobError::InvalidReceipt);
        }
        if let Some(total) = record.plan.total_frames {
            record.completed_frames = total;
        }
        record.state = JobState::Completed(receipt.clone());
        self.save(record, host)?;
        Ok(receipt)
    }
    /// Restore the host's full durable inventory before admitting new work.
    /// Validation is all-or-nothing; recovery cleanup itself can fail visibly.
    /// A new model lifetime invalidates every old worker ticket.
    pub fn recover(
        records: Vec<JobRecord>,
        host: &mut impl RenderJobHost,
    ) -> Result<Self, JobError> {
        let mut book = Self::default();
        for record in records {
            let id = record.id.clone();
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !record.plan.key.valid()
                || record.plan.reserve_bytes == 0
                || record.plan.comp_id.is_empty()
                || record.plan.version.project.is_empty()
                || record.plan.total_frames == Some(0)
                || record
                    .plan
                    .total_frames
                    .is_some_and(|n| record.completed_frames > n)
            {
                return Err(JobError::InvalidPlan);
            }
            if record.plan.require_originals && record.plan.key.provenance != Provenance::Originals
            {
                return Err(JobError::OriginalsRequired);
            }
            if let JobState::Completed(receipt) = &record.state
                && (receipt.key != record.plan.key
                    || receipt.bytes == 0
                    || receipt.bytes > record.plan.reserve_bytes)
            {
                return Err(JobError::InvalidReceipt);
            }
            if book.records.insert(id, record).is_some() {
                return Err(JobError::Duplicate);
            }
        }
        // No cleanup IO until the whole inventory has been validated.
        let ids: Vec<_> = book
            .records
            .values()
            .filter(|r| !r.state.terminal())
            .map(|r| r.id.clone())
            .collect();
        for id in ids {
            let mut r = book.records[&id].clone();
            if !matches!(
                r.state,
                JobState::Cleaning(_) | JobState::CancelRequested(_)
            ) {
                r.state = JobState::Cleaning(CleanupReason::Interrupted);
            }
            book.save(r, host)?;
            book.settle(&id, host)?;
        }
        Ok(book)
    }
    pub fn cache_state(
        &self,
        key: &FrameKey,
        require_originals: bool,
        host: &mut impl RenderJobHost,
    ) -> Result<CacheState, JobError> {
        if !key.valid() {
            return Err(JobError::InvalidPlan);
        }
        if require_originals && key.provenance != Provenance::Originals {
            return Err(JobError::OriginalsRequired);
        }
        if let Some(receipt) = host.cached(key)? {
            if receipt.key != *key || receipt.bytes == 0 {
                return Err(JobError::InvalidReceipt);
            }
            return Ok(CacheState::Available(receipt));
        }
        if self.records.values().any(|r| {
            r.plan.key == *key
                && matches!(r.state, JobState::Queued | JobState::Running)
                && self.current.get(&r.plan.version.project) == Some(&r.plan.version)
        }) {
            Ok(CacheState::Rendering)
        } else {
            Ok(CacheState::Missing)
        }
    }
    pub fn keep(
        &self,
        key: &FrameKey,
        keep: bool,
        host: &mut impl RenderJobHost,
    ) -> Result<(), JobError> {
        if !matches!(
            self.cache_state(key, false, host)?,
            CacheState::Available(_)
        ) {
            return Err(JobError::InvalidTransition);
        }
        host.keep(key, keep)?;
        Ok(())
    }
}
