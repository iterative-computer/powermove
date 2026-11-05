mod common;
use pm_kernel::pro_jobs::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::PathBuf,
};

/// Synthetic worker/storage adapter using real private files and durable JSON
/// records. These bytes test lifecycle/provenance, not decoder/render fidelity.
struct DiskHost {
    _temp: common::TempDir,
    root: PathBuf,
    calls: Vec<String>,
    live: BTreeSet<String>,
    save_error: bool,
    archive_error: bool,
    drain_error: bool,
    disk_full: bool,
    wrong_receipt: bool,
    pinned: BTreeSet<FrameKey>,
}
fn io(e: std::io::Error) -> HostError {
    HostError::Io(e.to_string())
}
impl DiskHost {
    fn new() -> Self {
        let temp = common::TempDir::new("pro-render-jobs");
        let root = fs::canonicalize(&temp.0).unwrap();
        for dir in ["stage", "records", "archive", "cache"] {
            fs::create_dir(root.join(dir)).unwrap();
        }
        fs::write(
            root.join("original"),
            b"authored-original-bytes-retain-exactly",
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            calls: vec![],
            live: BTreeSet::new(),
            save_error: false,
            archive_error: false,
            drain_error: false,
            disk_full: false,
            wrong_receipt: false,
            pinned: BTreeSet::new(),
        }
    }
    fn stage(&self, id: &str) -> PathBuf {
        self.root.join("stage").join(id)
    }
    fn archive_path(&self, id: &str) -> PathBuf {
        self.root.join("archive").join(id)
    }
    fn cache(&self, key: &FrameKey) -> PathBuf {
        let hash = Sha256::digest(serde_json::to_vec(key).unwrap());
        let digest: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        self.root.join("cache").join(digest)
    }
    fn load(&self) -> Vec<JobRecord> {
        fs::read_dir(self.root.join("records"))
            .unwrap()
            .map(|e| serde_json::from_slice(&fs::read(e.unwrap().path()).unwrap()).unwrap())
            .collect()
    }
    fn record_receipt(&self, key: &FrameKey, bytes: &[u8]) -> FrameReceipt {
        FrameReceipt {
            key: key.clone(),
            bytes: bytes.len() as u64,
            checksum: Sha256::digest(bytes).into(),
        }
    }
}
impl RenderJobHost for DiskHost {
    fn save(&mut self, r: &JobRecord) -> Result<(), HostError> {
        self.calls.push(format!("save:{}:{:?}", r.id, r.state));
        if self.save_error {
            return Err(HostError::DiskFull);
        }
        let dir = self.root.join("records");
        let target = dir.join(&r.id);
        let temp = dir.join(format!("{}.new", r.id));
        fs::write(&temp, serde_json::to_vec(r).unwrap()).map_err(io)?;
        File::open(&temp).map_err(io)?.sync_all().map_err(io)?;
        fs::rename(temp, target).map_err(io)?;
        File::open(dir).map_err(io)?.sync_all().map_err(io)
    }
    fn begin(&mut self, r: &JobRecord) -> Result<(), HostError> {
        self.calls.push(format!("begin:{}", r.id));
        assert_eq!(
            self.load().iter().find(|v| v.id == r.id).unwrap().state,
            JobState::Running
        );
        fs::write(self.stage(&r.id), b"synthetic-frame-payload-12345").map_err(io)?;
        if self.disk_full {
            return Err(HostError::DiskFull);
        }
        self.live.insert(r.id.clone());
        Ok(())
    }
    fn drain(&mut self, r: &JobRecord) -> Result<(), HostError> {
        self.calls.push(format!("drain:{}", r.id));
        if self.drain_error {
            return Err(HostError::Unavailable("producer still owns staging".into()));
        }
        self.live.remove(&r.id);
        Ok(())
    }
    fn archive(&mut self, r: &JobRecord, _: CleanupReason) -> Result<(), HostError> {
        self.calls.push(format!("archive:{}", r.id));
        assert!(!self.live.contains(&r.id));
        if self.archive_error {
            return Err(HostError::DiskFull);
        }
        let source = self.stage(&r.id);
        let target = self.archive_path(&r.id);
        if source.exists() {
            if target.exists() {
                return Err(HostError::Io("archive collision: source retained".into()));
            }
            fs::rename(source, target).map_err(io)?;
            File::open(self.root.join("archive"))
                .map_err(io)?
                .sync_all()
                .map_err(io)?;
        }
        Ok(())
    }
    fn publish(&mut self, r: &JobRecord) -> Result<FrameReceipt, HostError> {
        self.calls.push(format!("publish:{}", r.id));
        assert!(!self.live.contains(&r.id));
        if self.disk_full {
            return Err(HostError::DiskFull);
        }
        let bytes = fs::read(self.stage(&r.id)).map_err(io)?;
        let mut receipt = self.record_receipt(&r.plan.key, &bytes);
        if self.wrong_receipt {
            receipt.key.provenance = Provenance::Proxy;
            return Ok(receipt);
        }
        let target = self.cache(&r.plan.key);
        if target.exists() {
            if fs::read(&target).map_err(io)? != bytes {
                return Err(HostError::Io("cache key has different bytes".into()));
            }
        } else {
            fs::hard_link(self.stage(&r.id), &target).map_err(io)?;
        }
        File::open(&target).map_err(io)?.sync_all().map_err(io)?;
        let meta = target.with_extension("json");
        fs::write(&meta, serde_json::to_vec(&receipt).unwrap()).map_err(io)?;
        File::open(&meta).map_err(io)?.sync_all().map_err(io)?;
        File::open(self.root.join("cache"))
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        self.archive(r, CleanupReason::Failed)?;
        Ok(receipt)
    }
    fn cached(&mut self, key: &FrameKey) -> Result<Option<FrameReceipt>, HostError> {
        let target = self.cache(key);
        if !target.exists() {
            return Ok(None);
        }
        let stored: FrameReceipt =
            serde_json::from_slice(&fs::read(target.with_extension("json")).map_err(io)?).unwrap();
        let bytes = fs::read(target).map_err(io)?;
        if stored != self.record_receipt(key, &bytes) {
            return Err(HostError::Io("cache corruption".into()));
        }
        Ok(Some(stored))
    }
    fn keep(&mut self, key: &FrameKey, keep: bool) -> Result<(), HostError> {
        if self.cached(key)?.is_none() {
            return Err(HostError::Io("missing cache object".into()));
        }
        if keep {
            self.pinned.insert(key.clone());
        } else {
            self.pinned.remove(key);
        }
        Ok(())
    }
}
fn version(revision: u64) -> Version {
    Version {
        project: "P".into(),
        document: 42,
        revision,
        resources: 0,
    }
}
fn plan() -> RenderPlan {
    RenderPlan {
        version: version(1),
        comp_id: "C1".into(),
        key: FrameKey {
            graph: [1; 32],
            media: [2; 32],
            ticks: 100,
            width: 10,
            height: 10,
            format: "rgba16f".into(),
            renderer: "synthetic-worker-v1".into(),
            provenance: Provenance::Originals,
        },
        reserve_bytes: 4096,
        require_originals: true,
        total_frames: Some(3),
    }
}
fn setup() -> (RenderJobs, DiskHost) {
    let mut h = DiskHost::new();
    let mut b = RenderJobs::default();
    b.observe(version(1), &mut h).unwrap();
    (b, h)
}
fn queue(b: &mut RenderJobs, h: &mut DiskHost, id: &str) -> Ticket {
    b.enqueue(id.into(), plan(), h).unwrap();
    b.start(id, h).unwrap()
}
#[test]
fn successful_job_persists_progress_publishes_exact_cache_and_pins_only_existing_bytes() {
    let (mut b, mut h) = setup();
    let key = plan().key;
    assert_eq!(b.keep(&key, true, &mut h), Err(JobError::InvalidTransition));
    let t = queue(&mut b, &mut h, "render-1");
    assert_eq!(
        b.cache_state(&key, true, &mut h).unwrap(),
        CacheState::Rendering
    );
    b.progress(&t, 2, &mut h).unwrap();
    assert_eq!(b.progress(&t, 1, &mut h), Err(JobError::InvalidProgress));
    assert_eq!(b.progress(&t, 4, &mut h), Err(JobError::InvalidProgress));
    let receipt = b.complete(&t, &mut h).unwrap();
    assert_eq!(b.record("render-1").unwrap().completed_frames, 3);
    assert_eq!(
        b.cache_state(&key, true, &mut h).unwrap(),
        CacheState::Available(receipt.clone())
    );
    b.keep(&key, true, &mut h).unwrap();
    assert!(h.pinned.contains(&key));
    assert_eq!(h.load()[0].state, JobState::Completed(receipt));
    assert!(!h.stage("render-1").exists());
    assert!(h.archive_path("render-1").exists());
    assert_eq!(
        fs::read(h.root.join("original")).unwrap(),
        b"authored-original-bytes-retain-exactly"
    );
    let mut changed = key.clone();
    changed.media[0] = 3;
    assert_eq!(
        b.cache_state(&changed, true, &mut h).unwrap(),
        CacheState::Missing
    );
    h.calls.clear();
    b.keep(&key, false, &mut h).unwrap();
    assert!(!h.pinned.contains(&key));
}
#[test]
fn cancellation_rejects_late_progress_and_completion_until_worker_is_drained_and_archived() {
    let (mut b, mut h) = setup();
    let t = queue(&mut b, &mut h, "cancel");
    b.cancel("cancel", &mut h).unwrap();
    assert!(h.live.contains("cancel"));
    assert!(h.stage("cancel").exists());
    assert_eq!(b.complete(&t, &mut h), Err(JobError::InvalidTransition));
    assert_eq!(b.progress(&t, 1, &mut h), Err(JobError::InvalidTransition));
    h.drain_error = true;
    assert!(b.settle("cancel", &mut h).is_err());
    assert_eq!(
        b.record("cancel").unwrap().state,
        JobState::Cleaning(CleanupReason::Cancelled)
    );
    assert!(h.live.contains("cancel"));
    assert!(!h.archive_path("cancel").exists());
    h.drain_error = false;
    b.settle("cancel", &mut h).unwrap();
    assert_eq!(b.record("cancel").unwrap().state, JobState::Cancelled);
    assert!(!h.live.contains("cancel"));
    assert!(h.archive_path("cancel").exists());
    assert!(!h.cache(&plan().key).exists());
}
#[test]
fn cleanup_failure_remains_visible_and_retry_preserves_partial_and_original_bytes() {
    let (mut b, mut h) = setup();
    queue(&mut b, &mut h, "partial");
    let bytes = fs::read(h.stage("partial")).unwrap();
    b.cancel("partial", &mut h).unwrap();
    h.archive_error = true;
    assert_eq!(
        b.settle("partial", &mut h),
        Err(JobError::Host(HostError::DiskFull))
    );
    assert_eq!(
        b.record("partial").unwrap().state,
        JobState::Cleaning(CleanupReason::Cancelled)
    );
    assert_eq!(fs::read(h.stage("partial")).unwrap(), bytes);
    h.archive_error = false;
    b.settle("partial", &mut h).unwrap();
    assert_eq!(fs::read(h.archive_path("partial")).unwrap(), bytes);
    assert!(h.root.join("original").exists());
}
#[test]
fn startup_recovery_drains_archives_and_fences_every_precrash_ticket() {
    let (mut b, mut h) = setup();
    let t = queue(&mut b, &mut h, "crashed");
    b.progress(&t, 1, &mut h).unwrap();
    let before = h.load();
    drop(b);
    let mut recovered = RenderJobs::recover(before, &mut h).unwrap();
    assert_eq!(
        recovered.record("crashed").unwrap().state,
        JobState::Interrupted
    );
    assert!(h.archive_path("crashed").exists());
    assert_eq!(recovered.complete(&t, &mut h), Err(JobError::StaleTicket));
    let records = h.load();
    h.calls.clear();
    let recovered = RenderJobs::recover(records, &mut h).unwrap();
    assert!(h.calls.is_empty());
    assert_eq!(recovered.records().count(), 1);
}
#[test]
fn crash_after_archive_before_terminal_save_is_idempotent() {
    let (mut b, mut h) = setup();
    queue(&mut b, &mut h, "archive-window");
    b.cancel("archive-window", &mut h).unwrap();
    // Model a crash after the durable Cleaning intent and archival but before
    // terminal acknowledgement. Recover the exact serialized earlier record.
    let mut record = h.load().remove(0);
    record.state = JobState::Cleaning(CleanupReason::Cancelled);
    h.save(&record).unwrap();
    h.drain(&record).unwrap();
    h.archive(&record, CleanupReason::Cancelled).unwrap();
    let recovered = RenderJobs::recover(h.load(), &mut h).unwrap();
    assert_eq!(
        recovered.record("archive-window").unwrap().state,
        JobState::Cancelled
    );
    assert!(h.archive_path("archive-window").exists());
}
#[test]
fn low_disk_on_begin_or_publish_never_reports_success_and_archives_partials() {
    for during_publish in [false, true] {
        let (mut b, mut h) = setup();
        b.enqueue("low-disk".into(), plan(), &mut h).unwrap();
        if during_publish {
            let t = b.start("low-disk", &mut h).unwrap();
            h.disk_full = true;
            assert_eq!(
                b.complete(&t, &mut h),
                Err(JobError::Host(HostError::DiskFull))
            );
        } else {
            h.disk_full = true;
            assert_eq!(
                b.start("low-disk", &mut h),
                Err(JobError::Host(HostError::DiskFull))
            );
        }
        assert_eq!(
            b.record("low-disk").unwrap().state,
            JobState::Failed(CleanupReason::LowDisk)
        );
        assert!(h.archive_path("low-disk").exists());
        assert!(h.root.join("original").exists());
        assert!(!h.cache(&plan().key).exists());
    }
}
#[test]
fn failed_persistence_cannot_create_optimistic_running_or_cancelled_state() {
    let (mut b, mut h) = setup();
    b.enqueue("journal".into(), plan(), &mut h).unwrap();
    h.save_error = true;
    assert_eq!(
        b.start("journal", &mut h),
        Err(JobError::Host(HostError::DiskFull))
    );
    assert_eq!(b.record("journal").unwrap().state, JobState::Queued);
    assert!(!h.stage("journal").exists());
    h.save_error = false;
    let t = b.start("journal", &mut h).unwrap();
    h.save_error = true;
    assert!(b.cancel("journal", &mut h).is_err());
    assert_eq!(b.record("journal").unwrap().state, JobState::Running);
    assert!(h.live.contains("journal"));
    h.save_error = false;
    b.cancel("journal", &mut h).unwrap();
    b.settle("journal", &mut h).unwrap();
    assert_eq!(b.complete(&t, &mut h), Err(JobError::InvalidTransition));
}
#[test]
fn original_requests_never_admit_or_return_proxy_only_bytes() {
    let (mut b, mut h) = setup();
    let mut p = plan();
    p.key.provenance = Provenance::Proxy;
    assert_eq!(
        b.enqueue("proxy".into(), p.clone(), &mut h),
        Err(JobError::OriginalsRequired)
    );
    p.require_originals = false;
    b.enqueue("proxy".into(), p.clone(), &mut h).unwrap();
    let t = b.start("proxy", &mut h).unwrap();
    b.complete(&t, &mut h).unwrap();
    assert_eq!(
        b.cache_state(&p.key, true, &mut h),
        Err(JobError::OriginalsRequired)
    );
    assert!(matches!(
        b.cache_state(&p.key, false, &mut h).unwrap(),
        CacheState::Available(_)
    ));
    assert_eq!(
        b.cache_state(&plan().key, true, &mut h).unwrap(),
        CacheState::Missing
    );
    let t = queue(&mut b, &mut h, "wrong-receipt");
    h.wrong_receipt = true;
    assert_eq!(b.complete(&t, &mut h), Err(JobError::InvalidReceipt));
    assert_eq!(
        b.record("wrong-receipt").unwrap().state,
        JobState::Failed(CleanupReason::Failed)
    );
}
#[test]
fn cache_lookup_checks_real_bytes_and_does_not_trust_completed_task_state() {
    let (mut b, mut h) = setup();
    let t = queue(&mut b, &mut h, "cache");
    b.complete(&t, &mut h).unwrap();
    let key = plan().key;
    fs::write(h.cache(&key), b"corrupt-cache-payload").unwrap();
    assert!(matches!(
        b.cache_state(&key, true, &mut h),
        Err(JobError::Host(HostError::Io(_)))
    ));
    assert!(b.keep(&key, true, &mut h).is_err());
    fs::remove_file(h.cache(&key)).unwrap();
    assert_eq!(
        b.cache_state(&key, true, &mut h).unwrap(),
        CacheState::Missing
    );
}
#[test]
fn edits_reopen_and_failed_cancel_persistence_still_fence_old_render_work() {
    let (mut b, mut h) = setup();
    let t = queue(&mut b, &mut h, "old-revision");
    h.save_error = true;
    assert!(b.observe(version(2), &mut h).is_err());
    assert_eq!(b.complete(&t, &mut h), Err(JobError::StaleVersion));
    assert_eq!(b.observe(version(1), &mut h), Err(JobError::StaleVersion));
    h.save_error = false;
    b.observe(version(2), &mut h).unwrap();
    b.settle("old-revision", &mut h).unwrap();
    let mut next = version(1);
    next.document = 43;
    b.observe(next, &mut h).unwrap();
    assert_eq!(b.observe(version(2), &mut h), Err(JobError::StaleVersion));
}
#[test]
fn corrupt_or_duplicate_recovery_inventory_causes_no_cleanup_side_effects() {
    let (mut b, mut h) = setup();
    queue(&mut b, &mut h, "inventory");
    let records = h.load();
    for bad in [
        vec![records[0].clone(), records[0].clone()],
        {
            let mut r = records[0].clone();
            r.id = "../original".into();
            vec![r]
        },
        {
            let mut r = records[0].clone();
            r.completed_frames = 4;
            vec![r]
        },
    ] {
        h.calls.clear();
        assert!(RenderJobs::recover(bad, &mut h).is_err());
        assert!(h.calls.is_empty());
        assert!(h.stage("inventory").exists());
    }
}
#[test]
fn queued_cancel_and_unknown_total_have_explicit_inspectable_states() {
    let (mut b, mut h) = setup();
    let mut p = plan();
    p.total_frames = None;
    b.enqueue("unknown".into(), p, &mut h).unwrap();
    let t = b.start("unknown", &mut h).unwrap();
    b.progress(&t, 500, &mut h).unwrap();
    assert_eq!(b.record("unknown").unwrap().plan.total_frames, None);
    b.enqueue("queued".into(), plan(), &mut h).unwrap();
    b.cancel("queued", &mut h).unwrap();
    b.settle("queued", &mut h).unwrap();
    assert_eq!(b.record("queued").unwrap().state, JobState::Cancelled);
    assert_eq!(b.start("queued", &mut h), Err(JobError::InvalidTransition));
    assert_eq!(
        b.enqueue("queued".into(), plan(), &mut h),
        Err(JobError::Duplicate)
    );
}

#[test]
fn crash_after_cache_publication_before_terminal_record_keeps_verified_bytes_and_never_fakes_task_success()
 {
    let (mut b, mut h) = setup();
    let ticket = queue(&mut b, &mut h, "publish-window");
    h.save_error = true;
    assert_eq!(
        b.complete(&ticket, &mut h),
        Err(JobError::Host(HostError::DiskFull))
    );
    assert_eq!(b.record("publish-window").unwrap().state, JobState::Running);
    assert_eq!(h.load()[0].state, JobState::Running);
    assert!(h.cache(&plan().key).exists());
    assert!(h.archive_path("publish-window").exists());
    h.save_error = false;
    let recovered = RenderJobs::recover(h.load(), &mut h).unwrap();
    assert_eq!(
        recovered.record("publish-window").unwrap().state,
        JobState::Interrupted
    );
    assert!(matches!(
        recovered.cache_state(&plan().key, true, &mut h).unwrap(),
        CacheState::Available(_)
    ));
}
#[test]
fn recovery_archive_failure_retains_durable_cleanup_intent_for_a_later_restart() {
    let (mut b, mut h) = setup();
    let _ = queue(&mut b, &mut h, "recovery-low-disk");
    let bytes = fs::read(h.stage("recovery-low-disk")).unwrap();
    h.archive_error = true;
    assert!(matches!(
        RenderJobs::recover(h.load(), &mut h),
        Err(JobError::Host(HostError::DiskFull))
    ));
    assert_eq!(
        h.load()[0].state,
        JobState::Cleaning(CleanupReason::Interrupted)
    );
    assert_eq!(fs::read(h.stage("recovery-low-disk")).unwrap(), bytes);
    h.archive_error = false;
    let recovered = RenderJobs::recover(h.load(), &mut h).unwrap();
    assert_eq!(
        recovered.record("recovery-low-disk").unwrap().state,
        JobState::Interrupted
    );
    assert_eq!(
        fs::read(h.archive_path("recovery-low-disk")).unwrap(),
        bytes
    );
}

#[test]
fn resource_relink_without_document_edit_cancels_old_work_and_rejects_old_catalog_observations() {
    let (mut b, mut h) = setup();
    let ticket = queue(&mut b, &mut h, "old-catalog");
    let mut relinked = version(1);
    relinked.resources = 1;
    b.observe(relinked.clone(), &mut h).unwrap();
    assert!(matches!(
        b.record("old-catalog").unwrap().state,
        JobState::CancelRequested(CleanupReason::Superseded)
    ));
    assert_eq!(
        b.complete(&ticket, &mut h),
        Err(JobError::InvalidTransition)
    );
    assert_eq!(b.observe(version(1), &mut h), Err(JobError::StaleVersion));
    b.settle("old-catalog", &mut h).unwrap();
    let mut p = plan();
    p.version = relinked;
    p.key.media = [3; 32];
    b.enqueue("current-catalog".into(), p, &mut h).unwrap();
}
