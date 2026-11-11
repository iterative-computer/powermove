//! Same-executable --engine-job child. The parent and child talk in
//! [`ParentMessage`] and [`ChildMessage`] lines on inherited pipes; pixels go
//! through private files. A line either end cannot read fails the job.
use anyhow::{Result, ensure};
use pm_protocol::{
    common::{AssetId, JobId},
    error::{EngineError, ErrorCode, ErrorDetail, ErrorScope, Retry},
    priority::{JobSpec, JobState, JobStatus, PriorityClass},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Serialize, Deserialize)]
pub struct ExportRequest {
    pub job: JobId,
    pub spec: JobSpec,
    pub revision: u64,
    /// Already canonical, from the kernel's frozen snapshot.
    pub document: String,
    pub originals: std::collections::BTreeMap<String, PathBuf>,
    pub proxies: std::collections::BTreeMap<String, PathBuf>,
    pub destination: PathBuf,
    #[serde(default)]
    pub saving_app_version: Option<String>,
    #[serde(default)]
    pub resources: crate::resources::ResourceSnapshot,
    /// An export stops, and removes its partial output, before the
    /// destination volume has less free space than this.
    #[serde(default = "disk_reserve")]
    pub disk_reserve: u64,
    /// A look at the project (an agent's still), not a delivery: an effect
    /// whose provider isn't installed is bypassed, as the viewer does,
    /// instead of refusing the job.
    #[serde(default)]
    pub bypass_missing_effects: bool,
    /// Offline clips draw nothing instead of refusing the job (the shipped
    /// app's agent exports never asked). A clip the export would hear still
    /// refuses it by name.
    #[serde(default)]
    pub without_offline: bool,
    /// A look at the project, not a delivery (agent stills): offline media
    /// draws the Media Offline card as in the viewer instead of refusing.
    /// Never set for an export or a
    /// delivery capture.
    #[serde(default)]
    pub preview: bool,
    /// Render these frames into the disk render cache instead of delivering
    /// a file (`crate::render_cache`).
    #[serde(default)]
    pub cache: Option<crate::render_cache::CacheFill>,
}

/// Export children running now (render cache fills not counted).
static EXPORTS_RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Whether an export (the person's, an agent's, or a still) is rendering:
/// background rendering waits for it.
pub fn exports_running() -> bool {
    EXPORTS_RUNNING.load(Ordering::Acquire) > 0
}
/// Counts an export child while its reader lives.
struct Exporting;
impl Exporting {
    fn start() -> Self {
        EXPORTS_RUNNING.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for Exporting {
    fn drop(&mut self) {
        EXPORTS_RUNNING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The viewer's activity, as exports and cache fills see it: whether it is
/// drawing or playing now, and an epoch that rises every time it starts, so
/// an activity shorter than any sampling interval is still seen.
#[derive(Clone, Default)]
pub struct Activity {
    busy: Arc<AtomicBool>,
    epoch: Arc<std::sync::atomic::AtomicU64>,
    /// Wakes the jobs waiting for the viewer to go quiet or to be cancelled.
    quiet: Arc<(Mutex<()>, Condvar)>,
}
impl Activity {
    pub fn set(&self, busy: bool) {
        if busy {
            // Under the lock a background grant is checked and sent with
            // (`grant_if_idle`): activity lands before it or after it.
            let _grant = self.quiet.0.lock().unwrap();
            self.epoch.fetch_add(1, Ordering::AcqRel);
            self.busy.store(true, Ordering::Release);
            return;
        }
        self.busy.store(false, Ordering::Release);
        self.wake();
    }
    /// Sends a background grant (`send`) only if the viewer is still idle
    /// as last seen (`seen`): no activity can land between this check and
    /// the send. Whether it sent.
    fn grant_if_idle(&self, seen: u64, send: impl FnOnce()) -> bool {
        let _grant = self.quiet.0.lock().unwrap();
        if self.busy() || self.epoch() != seen {
            return false;
        }
        send();
        true
    }
    fn wake(&self) {
        let _quiet = self.quiet.0.lock().unwrap();
        self.quiet.1.notify_all();
    }
    /// Waits while the viewer is busy, unless `cancel` is set (whoever sets
    /// it wakes the wait). Whether it waited.
    fn wait_quiet(&self, cancel: &AtomicBool) -> bool {
        let mut quiet = self.quiet.0.lock().unwrap();
        let mut waited = false;
        while self.busy() && !cancel.load(Ordering::Acquire) {
            waited = true;
            quiet = self.quiet.1.wait(quiet).unwrap();
        }
        waited
    }
    /// Waits until the viewer has been inactive for `delay` (milliseconds,
    /// read live), unless `cancel` is set. `seen` and `active_at` carry the
    /// epoch last seen and when activity was last seen, across calls.
    fn wait_idle(
        &self,
        cancel: &AtomicBool,
        delay: &AtomicU64,
        seen: &mut u64,
        active_at: &mut Option<std::time::Instant>,
    ) {
        let mut quiet = self.quiet.0.lock().unwrap();
        while !cancel.load(Ordering::Acquire) {
            let (busy, epoch) = (self.busy(), self.epoch());
            if busy || epoch != *seen {
                *seen = epoch;
                *active_at = Some(std::time::Instant::now());
            }
            let delay = std::time::Duration::from_millis(delay.load(Ordering::Acquire));
            let left = active_at.map_or(std::time::Duration::ZERO, |at| {
                delay.saturating_sub(at.elapsed())
            });
            if !busy && left.is_zero() {
                return;
            }
            let poll = std::time::Duration::from_millis(100);
            let wait = if busy { poll } else { left.min(poll) };
            quiet = self.quiet.1.wait_timeout(quiet, wait).unwrap().0;
        }
    }
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    /// The bare flag, for workers that only defer while the viewer is busy.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.busy.clone()
    }
}
/// An export frame's raster override answer from the window that has the
/// document open.
#[derive(Debug)]
pub enum CompatReply {
    /// Passed through: the export draws the layer itself.
    Native,
    /// The override returned nothing: the layer draws nothing.
    Hidden,
    /// Premultiplied RGBA8 rows with the quad's size and anchor (layer
    /// pixels) and the part of the canvas it samples.
    Raster {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        size: [f64; 2],
        anchor: [f64; 2],
        uv: [f64; 4],
    },
}
/// One layer an export frame needs an override answer for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompatJobNeed {
    pub composition: String,
    pub layer: String,
    pub seconds: f64,
    pub scale: f64,
}
/// Answers a frame's override needs, in order, on the job's reader thread,
/// for the window that has the project open. What it pinned (the frozen
/// project, an export's realm) it lets go once the job's
/// [`JobSession::cancellation`] is set. An error fails the frame (and the
/// job).
pub type OverrideAnswers =
    Box<dyn Fn(&[CompatJobNeed]) -> std::result::Result<Vec<CompatReply>, String> + Send>;
/// A job before it starts: its frozen input, what answers its raster
/// overrides, and its cancellation. Dropped unstarted, or started and then
/// ended, cancelled or dropped, the job's cancellation is set.
pub struct JobSession {
    pub request: ExportRequest,
    overrides: Option<OverrideAnswers>,
    cancel: Cancel,
    /// Background work: its GPU submissions and override answers wait until
    /// the viewer has been inactive this long (milliseconds, read live).
    idle: Option<Arc<AtomicU64>>,
}
/// Set when dropped: whatever a job pinned is let go when it ends, however
/// it ends.
struct Cancel(Arc<AtomicBool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl JobSession {
    pub fn new(request: ExportRequest) -> Self {
        Self {
            request,
            overrides: None,
            cancel: Cancel(Arc::default()),
            idle: None,
        }
    }
    /// The job is background work: it submits GPU work and has its
    /// overrides answered only once the viewer has been inactive for
    /// `delay` milliseconds, so any interaction pauses it at once.
    pub fn in_background(&mut self, delay: Arc<AtomicU64>) {
        self.idle = Some(delay);
    }
    /// Set when the job is cancelled or over.
    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancel.0.clone()
    }
    /// The job draws its raster override layers with `answers`' answers.
    pub fn answer_overrides(&mut self, answers: OverrideAnswers) {
        let override_source = pm_protocol::snapshot::CompatSource::RasterOverride;
        if let JobSpec::Export { settings, .. } = &mut self.request.spec
            && !settings.delivery.compat.contains(&override_source)
        {
            settings.delivery.compat.push(override_source);
        }
        self.overrides = Some(answers);
    }
}
impl From<ExportRequest> for JobSession {
    fn from(request: ExportRequest) -> Self {
        Self::new(request)
    }
}
/// What the parent sends the child, one line each on its stdin.
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ParentMessage {
    /// The job. First, and only once.
    Start(Box<ExportRequest>),
    /// The GPU submission the child asked for: the rest of its frame, or one
    /// bounded by [`Costs::bounded`].
    Grant {
        whole_frame: bool,
    },
    /// The answers to compat request `id`, one per need, or why there are none.
    Compat {
        id: u64,
        answers: std::result::Result<Vec<WireAnswer>, String>,
        /// Microseconds the answer was held for viewer activity (background
        /// work): not the frame's cost.
        #[serde(default)]
        deferred_us: u64,
    },
    Cancel,
}
/// What the child sends the parent, one line each on its stdout.
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ChildMessage {
    Status(Box<JobStatus>),
    /// Asks for a GPU submission; the child waits for a [`ParentMessage::Grant`].
    Gpu,
    /// A frame's raster override needs, answered by a
    /// [`ParentMessage::Compat`] with the same `id`.
    Compat {
        id: u64,
        needs: Vec<CompatJobNeed>,
    },
    /// A render cache fill's frame, before the progress that counts it.
    Frame(crate::render_cache::FrameReport),
}
/// One need's answer on the child's stdin.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum WireAnswer {
    Native,
    Hidden,
    /// See [`CompatReply::Raster`]; the rows are in `file`, which the child
    /// reads and removes.
    Raster {
        width: u32,
        height: u32,
        size: [f64; 2],
        anchor: [f64; 2],
        uv: [f64; 4],
        file: PathBuf,
    },
}
fn send(out: &mut impl Write, message: &impl Serialize) -> std::io::Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.flush()
}
/// A compat request's answers as they cross to the child. Pixels go through
/// private files, never the pipe; their names are the job's own, so
/// simultaneous exports never share one.
fn wire_answers(
    job: &str,
    id: u64,
    answered: std::result::Result<Vec<CompatReply>, String>,
) -> std::result::Result<Vec<WireAnswer>, String> {
    let job: String = job
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut written = vec![];
    let mut wire = vec![];
    for (i, reply) in answered?.into_iter().enumerate() {
        wire.push(match reply {
            CompatReply::Native => WireAnswer::Native,
            CompatReply::Hidden => WireAnswer::Hidden,
            CompatReply::Raster {
                width,
                height,
                rgba,
                size,
                anchor,
                uv,
            } => {
                let file = std::env::temp_dir().join(format!(
                    "powermove-compat-{}-{job}-{id}-{i}.rgba",
                    std::process::id()
                ));
                if let Err(e) = std::fs::write(&file, &rgba) {
                    for file in written {
                        let _ = std::fs::remove_file(file);
                    }
                    return Err(e.to_string());
                }
                written.push(file.clone());
                WireAnswer::Raster {
                    width,
                    height,
                    size,
                    anchor,
                    uv,
                    file,
                }
            }
        });
    }
    Ok(wire)
}
/// Room left for macOS and a project save when a long export fills a disk.
pub const DISK_RESERVE: u64 = 2 << 30;
fn disk_reserve() -> u64 {
    DISK_RESERVE
}
/// Space an export may use on the volume that will hold `destination` (its
/// nearest existing ancestor: a PNG folder does not exist yet). This is the
/// figure Finder shows: free space plus what macOS purges on demand.
fn free_bytes(destination: &Path) -> Result<u64> {
    use objc2_foundation::{NSNumber, NSString, NSURL};
    let volume = destination
        .ancestors()
        .find(|p| p.exists())
        .ok_or_else(|| anyhow::anyhow!("export destination has no existing folder"))?;
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&volume.to_string_lossy()));
        let mut value = None;
        // SAFETY: the key is a resource key; the value it returns is an NSNumber.
        unsafe {
            url.getResourceValue_forKey_error(
                &mut value,
                objc2_foundation::NSURLVolumeAvailableCapacityForImportantUsageKey,
            )
        }
        .map_err(|e| anyhow::anyhow!("{}", e.localizedDescription()))?;
        value
            .and_then(|v| v.downcast::<NSNumber>().ok())
            .map(|n| n.unsignedLongLongValue())
            .ok_or_else(|| anyhow::anyhow!("the destination volume does not report free space"))
    })
}
/// Stops an export (through `cancel`) once `free` drops below `reserve`,
/// checking every `every` until `finished`.
fn watch_disk(
    free: impl Fn() -> Result<u64>,
    reserve: u64,
    every: std::time::Duration,
    finished: &AtomicBool,
    low_disk: &AtomicBool,
    cancel: &AtomicBool,
) {
    while !finished.load(Ordering::Acquire) {
        if free().is_ok_and(|free| free < reserve) {
            low_disk.store(true, Ordering::Release);
            cancel.store(true, Ordering::Release);
            return;
        }
        std::thread::sleep(every);
    }
}
pub(crate) const LOW_DISK: &str = "The export stopped because the destination disk is almost full. Free some space and export again.";
impl ExportRequest {
    pub fn new(
        job: JobId,
        snapshot: &crate::Snapshot,
        spec: JobSpec,
        media: crate::MediaCatalog,
        destination: PathBuf,
    ) -> Result<Self> {
        let document = snapshot
            .document
            .to_value()
            .ok_or_else(|| anyhow::anyhow!("missing document"))?;
        Ok(Self {
            job,
            spec,
            revision: snapshot.revision,
            document: pm_model::json::to_canonical_string(&document),
            originals: media.originals,
            proxies: media.proxies,
            destination,
            saving_app_version: snapshot.document.saving_app_version().map(str::to_owned),
            resources: Default::default(),
            disk_reserve: DISK_RESERVE,
            bypass_missing_effects: false,
            without_offline: false,
            preview: false,
            cache: None,
        })
    }
}
fn status(
    id: &JobId,
    state: JobState,
    progress: f64,
    path: Option<String>,
    error: Option<String>,
) -> JobStatus {
    JobStatus {
        job: id.clone(),
        class: PriorityClass::UserJob,
        state,
        progress,
        path,
        error: error.map(|e| {
            EngineError::new(
                ErrorCode::ExportFailed,
                ErrorScope::Job { job: id.clone() },
                e,
            )
        }),
        preview: None,
    }
}
/// An export refused because clips it draws are offline (audio it would
/// hear still fails by name, even without offline clips). Its status error
/// is [`ErrorCode::MediaOffline`] with the clips' assets, on which the Export
/// sheet offers Export Without Them.
#[derive(Debug)]
struct OfflineRefusal(Vec<pm_export::render::OfflineClip>);
impl std::fmt::Display for OfflineRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.0.iter().take(3).map(|c| c.name.as_str()).collect();
        match self.0.len() {
            1 => write!(f, "1 clip is offline: ")?,
            n => write!(f, "{n} clips are offline: ")?,
        }
        write!(f, "{}", names.join(", "))?;
        if let n @ 1.. = self.0.len().saturating_sub(3) {
            write!(f, " and {n} more")?;
        }
        write!(f, ". Relink the originals, or export without them.")
    }
}
impl std::error::Error for OfflineRefusal {}
impl OfflineRefusal {
    fn error(&self, job: &JobId) -> EngineError {
        let assets = self.0.iter().map(|c| AssetId::new(&c.asset)).collect();
        EngineError::new(
            ErrorCode::MediaOffline,
            ErrorScope::Job { job: job.clone() },
            self.to_string(),
        )
        .with_retry(Retry::AfterUserAction)
        .with_detail(ErrorDetail::Offline { assets })
    }
}
/// Stops a running job from any thread: its cancellation is set, a wait for
/// the viewer wakes, and the child is told.
#[derive(Clone)]
pub struct JobControl {
    cancel: Arc<AtomicBool>,
    input: Arc<Mutex<ChildStdin>>,
    interactive: Activity,
}
impl JobControl {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.interactive.wake();
        // A child that no longer reads its stdin has stopped already.
        let _ = send(&mut *self.input.lock().unwrap(), &ParentMessage::Cancel);
    }
}
/// A job's latest status; waiters wake when it changes.
struct Reported {
    now: Mutex<JobStatus>,
    changed: Condvar,
}
impl Reported {
    fn set(&self, status: JobStatus) {
        *self.now.lock().unwrap() = status;
        self.changed.notify_all();
    }
}
pub struct ExportHandle {
    child: Option<Child>,
    control: JobControl,
    status: Arc<Reported>,
    /// Ends with the child's output; true if it stopped at a line it could
    /// not use, with the child still running.
    reader: Option<std::thread::JoinHandle<bool>>,
    submissions: Arc<std::sync::atomic::AtomicU64>,
    asks: Arc<std::sync::atomic::AtomicU64>,
    /// A render cache fill's frame reports not taken yet.
    frames: Arc<Mutex<Vec<crate::render_cache::FrameReport>>>,
}
impl ExportHandle {
    /// Call from a job worker: process spawn and snapshot serialization are
    /// deliberately excluded from the UI event path.
    pub fn start(executable: &Path, session: impl Into<JobSession>) -> Result<Self> {
        Self::start_scheduled(executable, session, Activity::default())
    }
    /// Interactive service activity takes precedence at every submission:
    /// none is granted while the viewer is busy; within [`QUIET`] of its
    /// last activity a grant admits work bounded by [`BAND_BUDGET_US`] (see
    /// [`ChildGate`]); after that, whole frames. One already admitted
    /// submission cannot be preempted by Metal.
    pub fn start_scheduled(
        executable: &Path,
        session: impl Into<JobSession>,
        interactive: Activity,
    ) -> Result<Self> {
        let JobSession {
            request,
            overrides,
            cancel: release,
            idle,
        } = session.into();
        request.resources.validate()?;
        ensure!(
            matches!(request.spec, JobSpec::Export { .. }),
            "expected export job"
        );
        let mut child = Command::new(executable)
            .arg("--engine-job")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut input = child.stdin.take().unwrap();
        let id = request.job.clone();
        let exporting = request.cache.is_none().then(Exporting::start);
        if let Err(e) = send(&mut input, &ParentMessage::Start(Box::new(request))) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e.into());
        }
        let control = JobControl {
            cancel: release.0.clone(),
            input: Arc::new(Mutex::new(input)),
            interactive: interactive.clone(),
        };
        let (child_input, cancel) = (control.input.clone(), control.cancel.clone());
        let output = child.stdout.take().unwrap();
        let reported = Arc::new(Reported {
            now: Mutex::new(status(&id, JobState::Queued, 0., None, None)),
            changed: Condvar::new(),
        });
        let events = reported.clone();
        let submissions = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let granted = submissions.clone();
        let asks = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let asked = asks.clone();
        let frames: Arc<Mutex<Vec<crate::render_cache::FrameReport>>> = Arc::default();
        let reports = frames.clone();
        let reader = std::thread::spawn(move || {
            // Dropped with the reader: the job is over, and what its
            // overrides pinned goes.
            let _release = release;
            let _exporting = exporting;
            // The viewer's activity epoch at the last grant, and when it was
            // last seen to move (an epoch moved since counts as now).
            let mut seen = interactive.epoch();
            let mut active_at: Option<std::time::Instant> = None;
            // A child that stopped reading its stdin has exited or is
            // exiting: its last lines still say how it ended.
            let reply = |message: &ParentMessage| {
                let _ = send(&mut *child_input.lock().unwrap(), message);
            };
            let mut lines = std::io::BufReader::new(output).lines();
            let broken = loop {
                let message = match lines.next() {
                    None => break None,
                    Some(Err(e)) => break Some(format!("Powermove lost the export engine: {e}")),
                    Some(Ok(line)) => serde_json::from_str::<ChildMessage>(&line),
                };
                match message {
                    Err(e) => {
                        break Some(format!("The export engine sent an invalid message: {e}"));
                    }
                    Ok(ChildMessage::Status(s)) if s.job != id => {
                        break Some(format!(
                            "The export engine reported on another job, {}",
                            s.job.as_str()
                        ));
                    }
                    Ok(ChildMessage::Status(s)) => events.set(*s),
                    Ok(ChildMessage::Frame(report)) => reports.lock().unwrap().push(report),
                    Ok(ChildMessage::Compat { id: ask, needs }) => {
                        // Background work runs extensions only while the
                        // person is away, as it renders.
                        let held = std::time::Instant::now();
                        if let Some(delay) = &idle {
                            interactive.wait_idle(&cancel, delay, &mut seen, &mut active_at);
                        }
                        let deferred_us = held.elapsed().as_micros() as u64;
                        let answered = match &overrides {
                            Some(answer) => answer(&needs),
                            None => Err("No window has this project's extensions open".into()),
                        };
                        let answers = wire_answers(id.as_str(), ask, answered);
                        reply(&ParentMessage::Compat {
                            id: ask,
                            answers,
                            deferred_us,
                        });
                    }
                    Ok(ChildMessage::Gpu) => {
                        asked.fetch_add(1, Ordering::Release);
                        if let Some(delay) = &idle {
                            // Background work: granted only while the viewer
                            // is still idle, checked and sent under one lock;
                            // an activity in between waits for idle again.
                            // Never a whole frame by grant (an indivisible
                            // frame is admitted whole only by this idle grant).
                            while !cancel.load(Ordering::Acquire) {
                                interactive.wait_idle(&cancel, delay, &mut seen, &mut active_at);
                                if cancel.load(Ordering::Acquire) {
                                    break;
                                }
                                let grant = ParentMessage::Grant { whole_frame: false };
                                if interactive.grant_if_idle(seen, || reply(&grant)) {
                                    granted.fetch_add(1, Ordering::Relaxed);
                                    break;
                                }
                            }
                            continue;
                        } else if interactive.wait_quiet(&cancel) {
                            active_at = Some(std::time::Instant::now());
                        }
                        if !cancel.load(Ordering::Acquire) {
                            let epoch = interactive.epoch();
                            if epoch != seen {
                                seen = epoch;
                                active_at = Some(std::time::Instant::now());
                            }
                            let whole_frame = grants_whole_frames(false, active_at);
                            reply(&ParentMessage::Grant { whole_frame });
                            granted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            };
            let last = events.now.lock().unwrap().clone();
            if !last.state.is_terminal() {
                let error = broken
                    .clone()
                    .unwrap_or_else(|| "engine job child exited without a terminal status".into());
                events.set(status(
                    &id,
                    JobState::Failed,
                    last.progress,
                    None,
                    Some(error),
                ));
            }
            broken.is_some()
        });
        Ok(Self {
            child: Some(child),
            control,
            status: reported,
            reader: Some(reader),
            submissions,
            asks,
            frames,
        })
    }
    /// A render cache fill's frame reports since the last call, in order.
    pub fn take_frames(&self) -> Vec<crate::render_cache::FrameReport> {
        std::mem::take(&mut *self.frames.lock().unwrap())
    }
    pub fn status(&self) -> JobStatus {
        self.status.now.lock().unwrap().clone()
    }
    /// The first status that differs from `seen`, waited for.
    pub fn next_status(&self, seen: &JobStatus) -> JobStatus {
        let mut now = self.status.now.lock().unwrap();
        while *now == *seen {
            now = self.status.changed.wait(now).unwrap();
        }
        now.clone()
    }
    /// GPU submissions granted so far: one per frame while the viewer is
    /// quiet, one per band of a frame after viewer activity.
    pub fn submissions(&self) -> u64 {
        self.submissions.load(Ordering::Relaxed)
    }
    /// GPU submissions the child has asked for, granted or still waiting.
    pub fn gpu_requests(&self) -> u64 {
        self.asks.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.control.cancel();
    }
    /// Cancels this job from another thread.
    pub fn control(&self) -> JobControl {
        self.control.clone()
    }
    /// Worker-only wait. Drop also reaps the child (never a detached export).
    pub fn wait(mut self) -> Result<JobStatus> {
        let broken = match self.reader.take() {
            Some(reader) => reader
                .join()
                .map_err(|_| anyhow::anyhow!("export status reader panicked"))?,
            None => false,
        };
        if let Some(mut child) = self.child.take() {
            if broken {
                child.kill()?;
            }
            child.wait()?;
        }
        Ok(self.status())
    }
}
impl Drop for ExportHandle {
    fn drop(&mut self) {
        self.control.cancel();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}
/// Host binary calls this before initializing GPUI if --engine-job is present.
/// No window, Node, file transport or mutable kernel authority is created.
pub fn run_child() -> Result<()> {
    let mut lines = std::io::BufReader::new(std::io::stdin()).lines();
    let first = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing export request"))??;
    let ParentMessage::Start(request) = serde_json::from_str(&first)? else {
        anyhow::bail!("the export request did not come first");
    };
    let request = *request;
    let cancel = Arc::new(AtomicBool::new(false));
    request.resources.validate()?;
    let (grant, permits) = std::sync::mpsc::channel();
    let stopped = cancel.clone();
    let rasters = Arc::new(JobRasters {
        stop: cancel.clone(),
        ..JobRasters::default()
    });
    let replies = rasters.clone();
    let override_error = rasters.clone();
    // Why the parent's messages stopped making sense, if they did.
    let broken: Arc<Mutex<Option<String>>> = Arc::default();
    let unreadable = broken.clone();
    std::thread::spawn(move || {
        // A read error is the parent's end, as its EOF is.
        for line in lines.map_while(Result::ok) {
            let failure = match serde_json::from_str(&line) {
                Ok(ParentMessage::Grant { whole_frame }) => {
                    let _ = grant.send(whole_frame);
                    continue;
                }
                Ok(ParentMessage::Compat {
                    id,
                    answers,
                    deferred_us,
                }) => {
                    replies.deferred_us.fetch_add(deferred_us, Ordering::AcqRel);
                    match replies.reply(id, answers) {
                        Ok(()) => continue,
                        Err(e) => e,
                    }
                }
                Ok(ParentMessage::Cancel) => break,
                Ok(ParentMessage::Start(_)) => "a second export request".to_owned(),
                Err(e) => e.to_string(),
            };
            *unreadable.lock().unwrap() = Some(failure);
            break;
        }
        stopped.store(true, Ordering::Release);
    });
    let emit = |s: JobStatus| -> Result<()> {
        send(
            &mut std::io::stdout().lock(),
            &ChildMessage::Status(Box::new(s)),
        )?;
        Ok(())
    };
    emit(status(&request.job, JobState::Running, 0., None, None))?;
    // CoreText registry stays on main; rendering waits only on this serviced
    // broker, never on GPUI or a Node event loop.
    let mut font_service = pm_text::host::FontService::new().map_err(anyhow::Error::msg)?;
    let broker = pm_text::host::FontBroker::new();
    // This windowless host serves the broker below when an ask wakes it. A
    // registered wake admits requests; without it layout fails immediately
    // and can lose text glyphs.
    let (wake, woken) = std::sync::mpsc::sync_channel(1);
    let asked = wake.clone();
    broker.set_wake(Arc::new(move || {
        let _ = asked.try_send(());
    }));
    broker.set_generation(font_service.generation());
    let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
    let job_id = request.job.clone();
    // Low disk: refuse to start, then watch the volume while frames land.
    let reserve = request.disk_reserve;
    if free_bytes(&request.destination)? < reserve {
        emit(status(
            &job_id,
            JobState::Failed,
            0.,
            None,
            Some(LOW_DISK.into()),
        ))?;
        return Ok(());
    }
    let low_disk = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    {
        let (low_disk, finished, cancel) = (low_disk.clone(), finished.clone(), cancel.clone());
        let destination = request.destination.clone();
        std::thread::spawn(move || {
            watch_disk(
                || free_bytes(&destination),
                reserve,
                std::time::Duration::from_millis(250),
                &finished,
                &low_disk,
                &cancel,
            )
        });
    }
    let worker_cancel = cancel.clone();
    let done = Finished(finished.clone(), wake);
    let worker = std::thread::spawn(move || -> Result<PathBuf> {
        let _done = done;
        let cancel = worker_cancel;
        let gate = ChildGate::new(permits);
        let fill = request.cache;
        let JobSpec::Export {
            composition,
            settings,
            ..
        } = request.spec
        else {
            anyhow::bail!("expected export spec");
        };
        use pm_protocol::validate::Validate;
        settings.validate().map_err(|e| anyhow::anyhow!("{e:?}"))?;
        ensure!(
            settings
                .delivery
                .compat
                .iter()
                .all(|s| *s == pm_protocol::snapshot::CompatSource::RasterOverride),
            "this export needs an extension's WebGL context, which the app does not host"
        );
        let overriding = !settings.delivery.compat.is_empty();
        let catalog = crate::MediaCatalog {
            originals: request.originals,
            proxies: request.proxies,
            ..Default::default()
        };
        let media = catalog.export_paths(settings.draft_from_proxies);
        let mut doc = pm_model::Document::new(
            Some(pm_model::json::parse(&request.document)?),
            request.revision,
        );
        doc.set_saving_app_version(request.saving_app_version);
        // An output without audio doesn't need an offline audio clip.
        let offline = pm_export::render::offline_media_with_audio(
            &doc,
            composition.as_str(),
            &media,
            settings.audio,
        );
        // Going without offline clips never drops sound: a clip the export
        // would hear stops it by name, as the shipped app's exports did. It is
        // checked before the offline refusal so Export Without Them is never
        // offered for a retry that can't succeed.
        let tick = |t: pm_protocol::common::Ticks| {
            t.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64
        };
        if !request.preview && settings.audio && !settings.format.is_image() {
            let range = (tick(settings.range.start), tick(settings.range.end));
            if let Some(clip) =
                pm_export::render::offline_audible_media(&doc, composition.as_str(), &media, range)
                    .first()
            {
                anyhow::bail!(
                    "Audio media is missing for “{}”. Relink or import it again before exporting.",
                    clip.name
                );
            }
        }
        // A preview (an agent's still) draws offline media as the Media
        // Offline card instead.
        if !offline.is_empty() && !request.without_offline && !request.preview {
            return Err(OfflineRefusal(offline).into());
        }
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!("cancelled");
        }
        let mut renderer = pm_render::Renderer::headless()?;
        // Previews isolate failing layers, as the viewer: the card draws.
        renderer.set_isolate_layers(request.preview);
        if overriding {
            renderer.set_compat_rasters(Some(rasters.clone()));
        }
        // An export never ships silently without an effect the project uses.
        let failures = renderer
            .set_document_effects(&doc, &request.resources.definitions)
            .map_err(anyhow::Error::msg)?;
        // Only what this export draws: enabled effects in its composition.
        let uninstalled = renderer.missing_effects_in(&doc, composition.as_str());
        if request.bypass_missing_effects {
            renderer.set_bypass_missing_effects(true);
        } else if let Some(missing) = uninstalled.first() {
            let uses = match missing.layers {
                1 => "1 layer uses it".to_owned(),
                n => format!("{n} layers use it"),
            };
            let others = match uninstalled.len() - 1 {
                0 => String::new(),
                n => format!("; {n} more effects are missing too"),
            };
            match &missing.revision {
                Some(revision) => anyhow::bail!(
                    "{} version {revision} was saved by a newer Powermove ({uses}{others}). Export it there, or turn the effect off, then export again.",
                    missing.effect
                ),
                None => anyhow::bail!(
                    "{} isn't installed ({uses}{others}). Install the extension that provides it, or turn the effect off, then export again.",
                    missing.effect
                ),
            }
        }
        if let Some(failure) = failures.iter().find(|failure| {
            matches!(&failure.scope, pm_protocol::error::ErrorScope::Definition { definition }
                if request.document.contains(&format!("\"{}\"", definition.as_str())))
        }) {
            anyhow::bail!("{}", failure.message);
        }
        #[cfg(feature = "kernel")]
        crate::studio::attach(&mut renderer, Arc::new(doc.clone()), &media);
        fonts.sync(&mut renderer, &mut None);
        if let Some(fill) = fill {
            let job = request.job.clone();
            return fill_cache(
                &mut renderer,
                Arc::new(doc),
                composition.as_str(),
                media,
                &fill,
                &gate,
                &cancel,
                &mut |fraction| {
                    let _ = emit(status(&job, JobState::Running, fraction, None, None));
                },
                &mut |frame| {
                    let _ = send(&mut std::io::stdout().lock(), &ChildMessage::Frame(frame));
                },
                &rasters.deferred_us,
                // Stored frames are exported ones: the export's request.
                &pm_export::render::frame_request(
                    &settings,
                    pm_export::settings::AlphaOutput::None,
                    None,
                ),
            );
        }
        let job = pm_export::render::ExportJob {
            document: Arc::new(doc),
            composition: composition.as_str().to_owned(),
            settings,
            media: Arc::new(media),
            destination: request.destination,
            overwrite: pm_export::settings::Overwrite::Fail,
            mode: Some(0o600),
            seed: 1,
        };
        let plan = pm_export::render::plan_export_for(&job, &renderer)?;
        let sink = pm_export::render::sink_for(&plan)?;
        let delivered = pm_export::render::run_export_into(
            &mut renderer,
            &job,
            plan,
            sink,
            &gate,
            &cancel,
            &mut |p| {
                let _ = emit(status(
                    &request.job,
                    JobState::Running,
                    p.fraction(),
                    None,
                    None,
                ));
            },
        )?;
        Ok(delivered.path)
    });
    while !finished.load(Ordering::Acquire) && woken.recv().is_ok() {
        pm_text::host::serve(&mut font_service, broker.take_asks());
    }
    let result = worker
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("export worker panicked")));
    match result {
        // The worker's output was dropped unpublished: its temporary is gone.
        Err(_) if low_disk.load(Ordering::Acquire) => emit(status(
            &job_id,
            JobState::Failed,
            0.,
            None,
            Some(LOW_DISK.into()),
        ))?,
        Ok(path) => emit(status(
            &job_id,
            JobState::Done,
            1.,
            Some(path.to_string_lossy().into()),
            None,
        ))?,
        Err(_) if let Some(error) = broken.lock().unwrap().take() => emit(status(
            &job_id,
            JobState::Failed,
            0.,
            None,
            Some(format!("The export engine got an invalid message: {error}")),
        ))?,
        Err(_) if cancel.load(Ordering::Acquire) => {
            emit(status(&job_id, JobState::Cancelled, 0., None, None))?
        }
        Err(ref e) if let Some(refusal) = e.downcast_ref::<OfflineRefusal>() => emit(JobStatus {
            error: Some(refusal.error(&job_id)),
            ..status(&job_id, JobState::Failed, 0., None, None)
        })?,
        Err(e) => {
            // A layer an extension draws failed the frame: say why.
            let message = match override_error.error.lock().unwrap().take() {
                Some(error) => format!("An extension could not draw a layer: {error}"),
                None => e.to_string(),
            };
            emit(status(&job_id, JobState::Failed, 0., None, Some(message)))?
        }
    }
    Ok(())
}

/// Set when the export worker ends, however it ends: the disk watch stops,
/// and the font service's thread wakes to see it.
struct Finished(Arc<AtomicBool>, std::sync::mpsc::SyncSender<()>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
        let _ = self.1.try_send(());
    }
}
/// An export's raster override layers, asked of the parent (which asks the
/// window's extensions) as the frame reaches each one; the frame waits (an
/// export has no per-frame deadline). A need with no answer fails the frame.
/// Asking a frame's layers in one batch measured slower: the realm's work
/// then runs ahead of, not between, the frame's GPU work.
#[derive(Default)]
struct JobRasters {
    /// Cancellation or the parent's end: a frame waiting on an answer stops.
    stop: Arc<AtomicBool>,
    /// Why the last need went unanswered, for the export's failure.
    error: Mutex<Option<String>>,
    next: std::sync::atomic::AtomicU64,
    waiting: Mutex<std::collections::HashMap<u64, std::sync::mpsc::Sender<Answers>>>,
    answered: Mutex<JobAnswers>,
    /// Microseconds the parent held answers for viewer activity, so far.
    deferred_us: AtomicU64,
}
type Answers = std::result::Result<Vec<WireAnswer>, String>;
type JobKey = (String, String, u64, u64);
#[derive(Default)]
struct JobAnswers {
    map: std::collections::HashMap<JobKey, pm_render::compat::CompatAnswer>,
    time: u64,
}
impl JobRasters {
    /// The parent's answers to request `id`. One for a request whose frame
    /// stopped waiting is dropped; one for a request never made is an error.
    fn reply(&self, id: u64, answers: Answers) -> std::result::Result<(), String> {
        if id >= self.next.load(Ordering::Acquire) {
            return Err(format!(
                "an answer to compat request {id}, which was never made"
            ));
        }
        if let Some(tx) = self.waiting.lock().unwrap().remove(&id) {
            let _ = tx.send(answers);
        }
        Ok(())
    }
    fn raster(answer: WireAnswer) -> std::result::Result<pm_render::compat::CompatAnswer, String> {
        use pm_render::compat::CompatAnswer;
        let WireAnswer::Raster {
            width,
            height,
            size,
            anchor,
            uv,
            file,
        } = answer
        else {
            return Ok(match answer {
                WireAnswer::Hidden => crate::compat::hidden_raster(),
                _ => CompatAnswer::Native,
            });
        };
        let data = std::fs::read(&file);
        let _ = std::fs::remove_file(&file);
        let data = data.map_err(|e| e.to_string())?;
        if data.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(format!(
                "The extension's {width}×{height} layer arrived with {} bytes",
                data.len()
            ));
        }
        Ok(crate::compat::override_raster(
            width, height, data, size, anchor, uv,
        ))
    }
    /// The parent's reply, waited for until it comes, the export is
    /// cancelled or the parent ends.
    fn wait(&self, rx: &std::sync::mpsc::Receiver<Answers>) -> Option<Answers> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while std::time::Instant::now() < deadline {
            if self.stop.load(Ordering::Acquire) {
                return None;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(answers) => return Some(answers),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
        *self.error.lock().unwrap() = Some("The extension did not draw the layer in time".into());
        None
    }
}
impl pm_render::compat::CompatRasters for JobRasters {
    fn producing(&self, source: pm_protocol::snapshot::CompatSource) -> bool {
        source == pm_protocol::snapshot::CompatSource::RasterOverride
    }
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: pm_protocol::snapshot::CompatSource,
        _crop: Option<pm_protocol::common::PixelRect>,
    ) -> pm_render::compat::CompatAnswer {
        use pm_render::compat::CompatAnswer;
        if source != pm_protocol::snapshot::CompatSource::RasterOverride {
            return CompatAnswer::Native;
        }
        let key = (
            composition.to_owned(),
            layer.to_owned(),
            time.to_bits(),
            scale.to_bits(),
        );
        let needs = {
            let mut answered = self.answered.lock().unwrap();
            if let Some(answer) = answered.map.get(&key) {
                return answer.clone();
            }
            // A new frame: only its answers are kept (an export never comes back).
            if answered.time != key.2 {
                answered.map.clear();
                answered.time = key.2;
            }
            let needs = vec![CompatJobNeed {
                composition: composition.to_owned(),
                layer: layer.to_owned(),
                seconds: time,
                scale,
            }];
            needs
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let id = {
            let mut waiting = self.waiting.lock().unwrap();
            let id = self.next.fetch_add(1, Ordering::AcqRel);
            waiting.insert(id, tx);
            id
        };
        let asked = send(
            &mut std::io::stdout().lock(),
            &ChildMessage::Compat {
                id,
                needs: needs.clone(),
            },
        );
        let answers = asked.ok().and_then(|()| self.wait(&rx));
        self.waiting.lock().unwrap().remove(&id);
        let items = match answers {
            None => return CompatAnswer::Missing,
            Some(Err(error)) => {
                *self.error.lock().unwrap() = Some(error);
                return CompatAnswer::Missing;
            }
            Some(Ok(items)) if items.len() != needs.len() => {
                *self.error.lock().unwrap() = Some(format!(
                    "The app answered {} layers for {}",
                    items.len(),
                    needs.len()
                ));
                return CompatAnswer::Missing;
            }
            Some(Ok(items)) => items,
        };
        let mut answered = self.answered.lock().unwrap();
        let mut mine = CompatAnswer::Missing;
        for (need, item) in needs.iter().zip(items) {
            let answer = match Self::raster(item) {
                Ok(answer) => answer,
                Err(error) => {
                    *self.error.lock().unwrap() = Some(error);
                    continue;
                }
            };
            let k = (
                need.composition.clone(),
                need.layer.clone(),
                key.2,
                need.scale.to_bits(),
            );
            if k == key {
                mine = answer.clone();
            }
            answered.map.insert(k, answer);
        }
        mine
    }
}
/// Renders `fill`'s frames as the export draws them (its request, opaque, at
/// the composition's size and the project's motion blur, RGBA16F) into the
/// render cache folder
/// captured when the fill started, stopping before the volume or the Render
/// Files Limit is full (`crate::render_cache`).
///
/// With a budget (background rendering), each chunk's probe frames come
/// first, drawn as playback draws them and measured (GPU time, warm): a
/// chunk renders and stores all its frames, exactly, only when a probe
/// costs the budget or more, and is left alone otherwise, so ranges that
/// play in real time take no disk. A frame that can't be drawn exactly (a
/// layer failed) is never stored.
#[allow(clippy::too_many_arguments)]
fn fill_cache(
    renderer: &mut pm_render::Renderer,
    doc: Arc<pm_model::Document>,
    composition: &str,
    media: pm_export::render::MediaPaths,
    fill: &crate::render_cache::CacheFill,
    gate: &ChildGate,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
    report: &mut dyn FnMut(crate::render_cache::FrameReport),
    deferred: &AtomicU64,
    export_request: &pm_render::RenderRequest,
) -> Result<PathBuf> {
    use crate::render_cache::{DISK_FULL, FrameReport, LIMIT_FULL};
    if fill.budget_us.is_some() {
        // Background work never competes with the app for the CPU.
        let _ = crate::qos::set_job_qos(crate::qos::JobQos::Utility);
    }
    let cache = pm_promedia::cache::RenderCache::open(&fill.dir, fill.quota)?;
    let scene = crate::render::Scene {
        doc: doc.clone(),
        comp: composition.to_owned(),
        media: Arc::new(media),
        content: 0,
    };
    let size = pm_protocol::common::Size::new(fill.width, fill.height);
    let output = crate::policy::Output {
        width: size.width,
        height: size.height,
        display: 0,
        headroom: 1.,
        crop: None,
    };
    let pass = pm_export::render::pass_key(2, size);
    renderer.set_isolate_layers(true);
    // One frame drawn as `spec` asks, with its GPU time, or `None` when it
    // can't be drawn exactly (a layer failed): it is left out and the fill
    // goes on. An error (often passing: an override answer that didn't
    // come) stops the fill.
    let render_as = |renderer: &mut pm_render::Renderer,
                     frame: &crate::render_cache::CacheFrame,
                     spec: crate::render::FrameSpec,
                     request: Option<&pm_render::RenderRequest>|
     -> Result<Option<(pm_export::render::ReadFrame, Sample)>> {
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!("cancelled");
        }
        let time = pm_protocol::common::Ticks::new(frame.ticks)?;
        let request = match request {
            Some(request) => request.clone(),
            None => spec.request(&scene)?,
        };
        gate.take_measured();
        gate.take_waited();
        let deferred_before = deferred.load(Ordering::Acquire);
        let (started, cpu) = (std::time::Instant::now(), crate::trace::thread_cpu_ms());
        let read = match pm_export::render::render_frame_reusing(
            renderer,
            &doc,
            composition,
            time,
            &request,
            &scene.media,
            gate,
            pass,
            cancel,
        ) {
            Ok(read) => read,
            Err(pm_export::render::JobError::Cancelled) => anyhow::bail!("cancelled"),
            Err(e) => anyhow::bail!("{e}"),
        };
        let (gpu_us, submissions, untimed) = gate.take_measured();
        let cost = Sample {
            gpu_us,
            wall_us: own_time(
                started.elapsed().as_secs_f64() * 1e6,
                gate.take_waited(),
                deferred.load(Ordering::Acquire) - deferred_before,
            ),
            cpu_us: (crate::trace::thread_cpu_ms() - cpu) * 1e3,
            submissions,
            untimed,
        };
        Ok(read.failures.is_empty().then_some((read, cost)))
    };
    let time_of = |frame: &crate::render_cache::CacheFrame| {
        pm_protocol::common::Ticks::new(frame.ticks).map_err(anyhow::Error::from)
    };
    // The exported frame (the export's own request), coded for the cache.
    let draw = |renderer: &mut pm_render::Renderer,
                frame: &crate::render_cache::CacheFrame|
     -> Result<Option<Vec<u8>>> {
        let spec = crate::render::FrameSpec::exact(time_of(frame)?, output);
        Ok(
            render_as(renderer, frame, spec, Some(export_request))?.map(|(read, _)| {
                let bytes: Vec<u8> = read
                    .rgba
                    .iter()
                    .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
                    .collect();
                crate::frame_codec::encode(&bytes, fill.width, fill.height)
            }),
        )
    };
    // What the frame costs as playback draws it (its motion samples and
    // resampler), at the composition's size: whether it plays in real time.
    let measure = |renderer: &mut pm_render::Renderer,
                   frame: &crate::render_cache::CacheFrame|
     -> Result<Option<Sample>> {
        let spec = crate::render::FrameSpec {
            moment: crate::policy::Moment::Playing,
            exact: false,
            scale: fill.probe_scale,
            ..crate::render::FrameSpec::exact(time_of(frame)?, output)
        };
        Ok(render_as(renderer, frame, spec, None)?.map(|(_, cost)| cost))
    };
    // A probe as playback meets it: the frame before it drawn first, so
    // programs, decoders and caches are warm, then the probe measured.
    let probe = |renderer: &mut pm_render::Renderer,
                 frame: &crate::render_cache::CacheFrame|
     -> Result<Option<(Sample, Sample)>> {
        let Some(cold) = measure(renderer, frame)? else {
            return Ok(None);
        };
        let before = crate::render_cache::CacheFrame {
            ticks: (frame.ticks - fill.frame_ticks).max(0),
            ..frame.clone()
        };
        if fill.frame_ticks > 0 && before.ticks != frame.ticks {
            measure(renderer, &before)?;
        }
        Ok(measure(renderer, frame)?.map(|warm| (cold, warm)))
    };
    let diagnostics = std::env::var_os("PM_ENGINE_TRACE").map(|t| {
        let mut path = std::path::PathBuf::from(t).into_os_string();
        path.push(".fill.jsonl");
        path
    });
    let store = |frame: &crate::render_cache::CacheFrame, bytes: &[u8]| -> Result<()> {
        if free_bytes(&fill.dir)? < DISK_RESERVE + bytes.len() as u64 {
            anyhow::bail!(DISK_FULL);
        }
        cache
            .put(&frame.key, bytes)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::StorageFull {
                    anyhow::anyhow!(DISK_FULL)
                } else if e.to_string().contains("quota") {
                    anyhow::anyhow!(LIMIT_FULL)
                } else {
                    anyhow::Error::from(e)
                }
            })
            .inspect(|()| {
                // Which file holds which time (diagnostics only).
                if let Some(path) = &diagnostics
                    && let Ok(file) = frame.key.digest()
                {
                    let line = serde_json::json!({"stored": frame.ticks, "file": file});
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|mut f| writeln!(f, "{line}"));
                }
            })
    };
    let frames = &fill.frames;
    let total = frames.len().max(1) as f64;
    let mut at = 0;
    while at < frames.len() {
        let (first, chunk) = (at, frames[at].chunk);
        let end = at + frames[at..].iter().take_while(|f| f.chunk == chunk).count();
        // Probes first: measured as playback draws them, never stored.
        let mut probes = Vec::new();
        while at < end && frames[at].probe {
            let frame = &frames[at];
            match probe(renderer, frame)? {
                Some((cold, warm)) => {
                    if let Some(path) = &diagnostics {
                        let line = serde_json::json!({"ticks": frame.ticks, "cold": cold, "warm": warm,
                            "budget_us": fill.budget_us});
                        let _ = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .and_then(|mut f| writeln!(f, "{line}"));
                    }
                    probes.push((at, Some(warm.cost_us())));
                }
                None => probes.push((at, None)),
            }
            at += 1;
            progress(at as f64 / total);
        }
        // Heavy when every measured probe is at the budget or over (the
        // parent's rule, `render_cache::Known::need`).
        let heavy = match fill.budget_us {
            None => true,
            Some(budget) => {
                let measured: Vec<f64> = probes.iter().filter_map(|p| p.1).collect();
                probes.is_empty() || (!measured.is_empty() && measured.iter().all(|c| *c >= budget))
            }
        };
        let refused = |i: usize| probes.iter().any(|p| p.0 == i && p.1.is_none());
        for &(i, cost) in &probes {
            report(FrameReport {
                at: i as u32,
                cost_us: cost,
                stored: false,
                refused: cost.is_none(),
            });
        }
        if !heavy {
            at = end;
            progress(at as f64 / total);
            continue;
        }
        // Every frame of a chunk that can't play, its probes too, exactly.
        for (i, frame) in frames.iter().enumerate().take(end).skip(first) {
            if refused(i) {
                continue;
            }
            let drawn = draw(renderer, frame)?;
            if let Some(bytes) = &drawn {
                store(frame, bytes)?;
            }
            report(FrameReport {
                at: i as u32,
                cost_us: None,
                stored: drawn.is_some(),
                refused: drawn.is_none(),
            });
            if i >= at {
                at = i + 1;
                progress(at as f64 / total);
            }
        }
        at = end;
    }
    renderer.release_frame_reuse();
    Ok(fill.dir.clone())
}

/// A probe's own time in microseconds: its whole draw (`wall_us`) less the
/// time it waited for GPU grants and for override answers the parent held
/// for viewer activity: deferral for the person is never a frame's cost.
fn own_time(wall_us: f64, waited_us: f64, deferred_us: u64) -> f64 {
    (wall_us - waited_us - deferred_us as f64).max(0.)
}

/// Whether a GPU grant admits the rest of a frame: an export's, once the
/// viewer has been quiet for [`QUIET`]; background work's never: it is
/// granted right after the viewer is seen idle and only in bounded units
/// (a whole frame only where bands cost one), so a play that starts waits
/// behind one unit.
fn grants_whole_frames(background: bool, active_at: Option<std::time::Instant>) -> bool {
    !background && active_at.is_none_or(|t| t.elapsed() >= QUIET)
}

/// How long the viewer stays quiet before an export submits whole frames
/// again; until then submissions are bounded by [`BAND_BUDGET_US`].
const QUIET: std::time::Duration = std::time::Duration::from_secs(1);
/// GPU time one submission may take within [`QUIET`] of viewer activity
/// (half a 60 Hz frame): a scrub or play that resumes waits behind at most
/// this, or one indivisible unit (Metal cannot preempt a submission).
const BAND_BUDGET_US: f64 = 8_000.;

/// What a job's submissions measured: per unit, and per whole frame.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Costs {
    /// Units in a whole frame (the most a frame's first admission asked).
    frame_units: u32,
    whole_us: Option<f64>,
    unit_us: Option<f64>,
    /// Bands cost about a whole frame (a precomp or a multi-pass effect
    /// renders whole for any region): splitting buys no latency.
    indivisible: bool,
    /// One unit already costs the budget: [`SCALING_UNITS`] units were
    /// asked once, to see whether cost grows with the band at all.
    tested: bool,
}
/// Units of a scaling test: as many bands as this costing under twice one
/// band means bands cost a frame each.
const SCALING_UNITS: u32 = 4;
impl Costs {
    fn record(&mut self, units: u32, us: f64) {
        if units == 0 || !us.is_finite() {
            return;
        }
        if units >= self.frame_units {
            self.whole_us = Some(us);
        } else if self.whole_us.is_some_and(|whole| us >= 0.75 * whole)
            || (units >= SCALING_UNITS && self.unit_us.is_some_and(|unit| us < 2. * unit))
        {
            self.indivisible = true;
        }
        self.unit_us = Some(us / f64::from(units));
    }
    /// Units of `remaining` to submit within the budget: one unit to probe
    /// a job not measured yet, as many as fit otherwise, and the rest of the
    /// frame when bands cost a whole frame anyway. Where one unit already
    /// costs the budget and no whole frame was measured (background work
    /// never gets one), a few units are asked once to find out.
    fn bounded(&mut self, remaining: u32) -> u32 {
        if self.indivisible {
            return remaining;
        }
        match self.unit_us {
            None => 1,
            Some(unit) if unit >= BAND_BUDGET_US && self.whole_us.is_none() && !self.tested => {
                self.tested = true;
                SCALING_UNITS.min(remaining)
            }
            Some(unit) => ((BAND_BUDGET_US / unit.max(1.)).floor() as u32).clamp(1, remaining),
        }
    }
}

/// What drawing one probe frame took.
#[derive(Clone, Copy, Debug, Serialize)]
struct Sample {
    /// GPU time the gate measured (span timestamps, or wall time from
    /// submission to completion for `untimed` submissions).
    gpu_us: f64,
    /// The whole draw, CPU and GPU.
    wall_us: f64,
    /// This thread's CPU time in the draw.
    cpu_us: f64,
    submissions: u32,
    untimed: u32,
}
impl Sample {
    /// The whole draw (with what it waits on: override answers, exact
    /// video frames, other threads), less the part playback overlaps with
    /// the frame before (this frame's CPU work runs during the last one's
    /// GPU work). Calibrated on Paper against the frames live playback
    /// dropped: GPU or CPU time alone missed the override waits.
    fn cost_us(&self) -> f64 {
        self.wall_us - self.gpu_us.min(self.cpu_us)
    }
}
/// Inherited-pipe admission: a whole-frame grant admits the rest of the frame
/// (the viewer has been quiet for [`QUIET`]), any other a submission bounded
/// by [`Costs::bounded`]. Cancellation and parent EOF interrupt a deferred job.
struct ChildGate {
    permits: Mutex<std::sync::mpsc::Receiver<bool>>,
    costs: Mutex<Costs>,
    /// Microseconds measured since [`ChildGate::take_measured`] last ran,
    /// the submissions they came from, and how many had no GPU timestamps.
    measured: Mutex<(f64, u32, u32)>,
    /// Microseconds spent waiting for grants (the viewer's turn) since
    /// [`ChildGate::take_waited`] last ran: not a frame's own time.
    waited_us: Mutex<f64>,
}
impl ChildGate {
    fn new(permits: std::sync::mpsc::Receiver<bool>) -> Self {
        Self {
            permits: Mutex::new(permits),
            costs: Mutex::default(),
            measured: Mutex::default(),
            waited_us: Mutex::default(),
        }
    }
    /// Microseconds waited for grants since the last call.
    fn take_waited(&self) -> f64 {
        std::mem::take(&mut *self.waited_us.lock().unwrap())
    }
    /// The measured time of the submissions finished since the last call
    /// (one frame's GPU time when taken around it), their count, and how
    /// many were wall time for want of GPU timestamps.
    fn take_measured(&self) -> (f64, u32, u32) {
        std::mem::take(&mut *self.measured.lock().unwrap())
    }
}
impl pm_export::render::GpuGate for ChildGate {
    fn admit(&self, _: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        {
            let mut costs = self.costs.lock().unwrap();
            costs.frame_units = costs.frame_units.max(units);
        }
        send(&mut std::io::stdout().lock(), &ChildMessage::Gpu).ok()?;
        let asked = std::time::Instant::now();
        let permits = self.permits.lock().unwrap();
        let admitted = || {
            *self.waited_us.lock().unwrap() += asked.elapsed().as_secs_f64() * 1e6;
        };
        while !cancel.load(Ordering::Acquire) {
            match permits.recv_timeout(std::time::Duration::from_millis(5)) {
                Ok(true) => {
                    admitted();
                    return Some(units);
                }
                Ok(false) => {
                    admitted();
                    return Some(self.costs.lock().unwrap().bounded(units));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return None,
            }
        }
        None
    }
    fn finished(&self, _: u64, units: u32, measured_us: f64) {
        self.finished_timed(0, units, measured_us, true);
    }
    fn finished_timed(&self, _: u64, units: u32, measured_us: f64, gpu_timed: bool) {
        self.costs.lock().unwrap().record(units, measured_us);
        let mut measured = self.measured.lock().unwrap();
        measured.0 += measured_us;
        measured.1 += 1;
        measured.2 += u32::from(!gpu_timed);
    }
    fn abandoned(&self, _: u64) {}
    /// Whole frames (a quiet viewer) overlap: the next frame's CPU work runs
    /// during this one's GPU work. A banded frame waits for the one before.
    fn overlaps(&self) -> bool {
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Within the quiet period a submission fits the budget, or is one
    /// unit; bands that cost a whole frame give way to whole frames.
    #[test]
    fn bounded_admission_fits_the_budget_unless_bands_cost_a_frame() {
        let mut costs = Costs {
            frame_units: 68,
            ..Costs::default()
        };
        assert_eq!(costs.bounded(68), 1, "an unmeasured job probes one unit");
        costs.record(68, 34_000.);
        assert_eq!(costs.bounded(68), 16, "0.5 ms units: 16 fit 8 ms");
        costs.record(1, 30_000.);
        assert!(costs.indivisible);
        assert_eq!(costs.bounded(67), 67, "a band costing a frame: no split");
        let mut cheap = Costs {
            frame_units: 12,
            ..Costs::default()
        };
        cheap.record(12, 1_200.);
        assert_eq!(cheap.bounded(12), 12, "a cheap frame fits whole");
    }
    /// Without a whole-frame measurement (background work), a band costing
    /// the budget asks a few bands once: when they cost about one band,
    /// bands cost a frame each and frames go whole; when cost grows with
    /// the band, bands stay bounded.
    #[test]
    fn bands_that_cost_a_frame_are_found_without_a_whole_frame() {
        let mut heavy = Costs {
            frame_units: 68,
            ..Costs::default()
        };
        heavy.record(1, 40_000.);
        assert_eq!(heavy.bounded(67), SCALING_UNITS);
        heavy.record(SCALING_UNITS, 42_000.);
        assert_eq!(heavy.bounded(63), 63, "bands cost a frame: whole");
        let mut divisible = Costs {
            frame_units: 68,
            ..Costs::default()
        };
        divisible.record(1, 10_000.);
        assert_eq!(divisible.bounded(67), SCALING_UNITS);
        divisible.record(SCALING_UNITS, 40_000.);
        assert_eq!(divisible.bounded(63), 1, "cost grows with the band");
        assert_eq!(divisible.bounded(62), 1, "tested once");
    }
    /// A draw that waited for its grant (the viewer's turn), or for override
    /// answers held for the person, doesn't count that wait as its own time.
    #[test]
    fn a_probe_never_counts_its_wait_for_the_viewer() {
        let (permit, permits) = std::sync::mpsc::channel();
        let gate = ChildGate::new(permits);
        let late = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            permit.send(true).unwrap();
        });
        use pm_export::render::GpuGate;
        assert_eq!(gate.admit(0, 4, &AtomicBool::new(false)), Some(4));
        late.join().unwrap();
        let waited = gate.take_waited();
        assert!(waited >= 290_000., "{waited}");
        assert_eq!(gate.take_waited(), 0.);
        // A 450 ms draw that waited 300 ms for its grant and had 100 ms of
        // answers held: 50 ms of its own.
        assert_eq!(own_time(450_000., 300_000., 100_000), 50_000.);
    }
    /// An activity that lands while a background grant is being checked
    /// and sent waits for it; one that landed first stops it, so nothing
    /// is sent once the person has acted.
    #[test]
    fn no_background_grant_is_sent_after_an_activity() {
        let activity = Activity::default();
        let seen = activity.epoch();
        // Play lands first: nothing is sent.
        activity.set(true);
        activity.set(false);
        let mut sent = false;
        assert!(!activity.grant_if_idle(seen, || sent = true));
        assert!(!sent);
        // Play lands during the send: it takes effect after it.
        let seen = activity.epoch();
        let player = activity.clone();
        let mut epoch_during_send = 0;
        assert!(activity.grant_if_idle(seen, || {
            let play = std::thread::spawn(move || player.set(true));
            std::thread::sleep(std::time::Duration::from_millis(50));
            epoch_during_send = activity.epoch();
            drop(play);
        }));
        assert_eq!(
            epoch_during_send, seen,
            "no activity landed inside the grant"
        );
        let start = std::time::Instant::now();
        while activity.epoch() == seen {
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            std::thread::yield_now();
        }
        assert!(activity.busy());
    }
    /// Background work is never granted whole frames, however long the
    /// viewer has been idle; an export is, once the viewer is quiet.
    #[test]
    fn background_grants_are_always_bounded() {
        let long_ago = std::time::Instant::now() - 2 * QUIET;
        assert!(grants_whole_frames(false, None));
        assert!(grants_whole_frames(false, Some(long_ago)));
        assert!(!grants_whole_frames(false, Some(std::time::Instant::now())));
        assert!(!grants_whole_frames(true, None));
        assert!(!grants_whole_frames(true, Some(long_ago)));
    }
    /// A job waiting for the viewer wakes when it goes quiet, or when the job
    /// is cancelled while it is still busy.
    #[test]
    fn a_waiting_job_wakes_when_the_viewer_goes_quiet_or_it_is_cancelled() {
        let activity = Activity::default();
        activity.set(true);
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting = |activity: &Activity, cancel: &Arc<AtomicBool>| {
            let (activity, cancel) = (activity.clone(), cancel.clone());
            std::thread::spawn(move || activity.wait_quiet(&cancel))
        };
        let quiet = waiting(&activity, &cancel);
        std::thread::sleep(std::time::Duration::from_millis(20));
        activity.set(false);
        assert!(quiet.join().unwrap(), "it waited for the viewer");
        activity.set(true);
        let cancelled = waiting(&activity, &cancel);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cancel.store(true, Ordering::Release);
        activity.wake();
        assert!(cancelled.join().unwrap());
        assert!(activity.busy());
    }
    /// An offline refusal names the first three clips and counts the rest.
    #[test]
    fn an_offline_refusal_names_three_clips_and_counts_the_rest() {
        let clips = |n: usize| {
            OfflineRefusal(
                (0..n)
                    .map(|i| pm_export::render::OfflineClip {
                        asset: format!("A{i}"),
                        name: format!("take {i}.mov"),
                    })
                    .collect(),
            )
        };
        assert_eq!(
            clips(5).to_string(),
            "5 clips are offline: take 0.mov, take 1.mov, take 2.mov and 2 more. \
             Relink the originals, or export without them."
        );
        assert_eq!(
            clips(2).to_string(),
            "2 clips are offline: take 0.mov, take 1.mov. Relink the originals, or export without them."
        );
    }
    /// A disk that fills during an export stops it; a finished export is
    /// left alone.
    #[test]
    fn a_disk_filling_mid_export_stops_it() {
        let left = std::sync::atomic::AtomicU64::new(DISK_RESERVE + (64 << 20));
        let (finished, low, cancel) = (
            AtomicBool::new(false),
            AtomicBool::default(),
            AtomicBool::default(),
        );
        let tick = std::time::Duration::from_millis(1);
        watch_disk(
            || Ok(left.fetch_sub(16 << 20, Ordering::AcqRel)),
            DISK_RESERVE,
            tick,
            &finished,
            &low,
            &cancel,
        );
        assert!(low.load(Ordering::Acquire) && cancel.load(Ordering::Acquire));
        assert!(left.load(Ordering::Acquire) < DISK_RESERVE);
        let (finished, low, cancel) = (
            AtomicBool::new(true),
            AtomicBool::default(),
            AtomicBool::default(),
        );
        watch_disk(|| Ok(0), DISK_RESERVE, tick, &finished, &low, &cancel);
        assert!(!cancel.load(Ordering::Acquire) && !low.load(Ordering::Acquire));
    }
    #[test]
    fn free_space_is_read_for_a_folder_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_bytes(&dir.path().join("frames/not-yet")).unwrap() > 0);
    }
    #[test]
    fn simultaneous_exports_never_share_a_pixel_file() {
        let answer = |job: &str| {
            let raster = CompatReply::Raster {
                width: 1,
                height: 1,
                rgba: job.as_bytes()[..4].to_vec(),
                size: [1., 1.],
                anchor: [0., 0.],
                uv: [0., 0., 1., 1.],
            };
            wire_answers(job, 0, Ok(vec![raster])).unwrap().remove(0)
        };
        let (a, b) = (answer("export-aaaa"), answer("export-bbbb"));
        let file = |a: &WireAnswer| match a {
            WireAnswer::Raster { file, .. } => file.clone(),
            other => panic!("{other:?}"),
        };
        assert_ne!(file(&a), file(&b));
        for item in [a, b] {
            match JobRasters::raster(item).unwrap() {
                pm_render::compat::CompatAnswer::Raster(r) => assert_eq!(&r.data[..], b"expo"),
                other => panic!("{other:?}"),
            }
        }
    }
    #[test]
    fn a_cancelled_export_stops_waiting_for_an_extension() {
        let rasters = JobRasters::default();
        let (_tx, rx) = std::sync::mpsc::channel();
        rasters.stop.store(true, Ordering::Release);
        let started = std::time::Instant::now();
        assert!(rasters.wait(&rx).is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(matches!(
            JobRasters::raster(WireAnswer::Hidden),
            Ok(pm_render::compat::CompatAnswer::Raster(_))
        ));
    }
    /// An answer to a request the child never made is the parent's mistake;
    /// one to a request whose frame stopped waiting is late, and dropped.
    #[test]
    fn an_answer_to_a_request_never_made_fails_the_job() {
        let rasters = JobRasters::default();
        assert!(rasters.reply(0, Ok(vec![])).is_err());
        rasters.next.store(1, Ordering::Release);
        assert_eq!(rasters.reply(0, Ok(vec![])), Ok(()));
        assert!(rasters.reply(7, Ok(vec![])).is_err());
    }
    /// A layer answered with the wrong number of answers, or pixels that
    /// don't fill it, fails with the reason instead of drawing.
    #[test]
    fn a_short_answer_fails_the_layer() {
        use pm_protocol::snapshot::CompatSource;
        use pm_render::compat::{CompatAnswer, CompatRasters};
        let rasters = Arc::new(JobRasters::default());
        let parent = rasters.clone();
        let replied = std::thread::spawn(move || {
            while parent.waiting.lock().unwrap().is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            parent.reply(0, Ok(vec![])).unwrap();
        });
        let answer = rasters.answer("c", "l", 1., 1., CompatSource::RasterOverride, None);
        replied.join().unwrap();
        assert!(matches!(answer, CompatAnswer::Missing));
        let error = rasters.error.lock().unwrap().take();
        assert_eq!(error.as_deref(), Some("The app answered 0 layers for 1"));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), [0; 3]).unwrap();
        let short = WireAnswer::Raster {
            width: 1,
            height: 1,
            size: [1., 1.],
            anchor: [0., 0.],
            uv: [0., 0., 1., 1.],
            file: file.path().to_owned(),
        };
        assert!(JobRasters::raster(short).is_err());
    }
}
