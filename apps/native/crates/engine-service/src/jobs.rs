//! Same-executable --engine-job child. The parent and child talk in
//! [`ParentMessage`] and [`ChildMessage`] lines on inherited pipes; pixels go
//! through private files. A line either end cannot read fails the job.
use anyhow::{Result, ensure};
use pm_protocol::{
    common::JobId,
    error::{EngineError, ErrorCode, ErrorScope},
    priority::{JobSpec, JobState, JobStatus, PriorityClass},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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

/// The viewer's activity, as exports and cache fills see it: whether it is
/// drawing or playing now, and an epoch that rises every time it starts, so
/// an activity shorter than any sampling interval is still seen.
#[derive(Clone, Default)]
pub struct Activity {
    busy: Arc<AtomicBool>,
    epoch: Arc<std::sync::atomic::AtomicU64>,
}
impl Activity {
    pub fn set(&self, busy: bool) {
        if busy {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        self.busy.store(busy, Ordering::Release);
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
        }
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
/// Ends the refusal of an export with offline clips; the Export sheet offers
/// Export Without Them on it.
pub const OFFLINE_ADVICE: &str = "Relink the originals, or export without them.";
/// An export refused because clips are offline (not silent audio, which still fails).
pub fn is_offline_refusal(message: &str) -> bool {
    message.ends_with(OFFLINE_ADVICE)
}
pub struct ExportHandle {
    child: Option<Child>,
    input: Arc<Mutex<ChildStdin>>,
    cancel: Cancel,
    status: Arc<Mutex<JobStatus>>,
    /// Ends with the child's output; true if it stopped at a line it could
    /// not use, with the child still running.
    reader: Option<std::thread::JoinHandle<bool>>,
    submissions: Arc<std::sync::atomic::AtomicU64>,
    asks: Arc<std::sync::atomic::AtomicU64>,
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
            cancel,
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
        if let Err(e) = send(&mut input, &ParentMessage::Start(Box::new(request))) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e.into());
        }
        let input = Arc::new(Mutex::new(input));
        let child_input = input.clone();
        let gate_cancel = cancel.0.clone();
        let output = child.stdout.take().unwrap();
        let state = Arc::new(Mutex::new(status(&id, JobState::Queued, 0., None, None)));
        let events = state.clone();
        let submissions = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let granted = submissions.clone();
        let asks = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let asked = asks.clone();
        let reader = std::thread::spawn(move || {
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
                    Ok(ChildMessage::Status(s)) => *events.lock().unwrap() = *s,
                    Ok(ChildMessage::Compat { id: ask, needs }) => {
                        let answered = match &overrides {
                            Some(answer) => answer(&needs),
                            None => Err("No window has this project's extensions open".into()),
                        };
                        let answers = wire_answers(id.as_str(), ask, answered);
                        reply(&ParentMessage::Compat { id: ask, answers });
                    }
                    Ok(ChildMessage::Gpu) => {
                        asked.fetch_add(1, Ordering::Release);
                        while interactive.busy() && !gate_cancel.load(Ordering::Acquire) {
                            active_at = Some(std::time::Instant::now());
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        if !gate_cancel.load(Ordering::Acquire) {
                            let epoch = interactive.epoch();
                            if epoch != seen {
                                seen = epoch;
                                active_at = Some(std::time::Instant::now());
                            }
                            let whole_frame = active_at.is_none_or(|t| t.elapsed() >= QUIET);
                            reply(&ParentMessage::Grant { whole_frame });
                            granted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            };
            let mut last = events.lock().unwrap();
            if !last.state.is_terminal() {
                let error = broken
                    .clone()
                    .unwrap_or_else(|| "engine job child exited without a terminal status".into());
                *last = status(&id, JobState::Failed, last.progress, None, Some(error));
            }
            // The job is over: what its overrides pinned goes.
            gate_cancel.store(true, Ordering::Release);
            broken.is_some()
        });
        Ok(Self {
            child: Some(child),
            input,
            cancel,
            status: state,
            reader: Some(reader),
            submissions,
            asks,
        })
    }
    pub fn status(&self) -> JobStatus {
        self.status.lock().unwrap().clone()
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
    pub fn cancel(&mut self) -> Result<()> {
        self.cancel.0.store(true, Ordering::Release);
        send(&mut *self.input.lock().unwrap(), &ParentMessage::Cancel)?;
        Ok(())
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
        self.cancel.0.store(true, Ordering::Release);
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
                Ok(ParentMessage::Compat { id, answers }) => match replies.reply(id, answers) {
                    Ok(()) => continue,
                    Err(e) => e,
                },
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
    // This windowless host pumps the broker below. A registered wake admits
    // requests; without it layout fails immediately and can lose text glyphs.
    broker.set_wake(Arc::new(|| {}));
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
    let worker = std::thread::spawn(move || -> Result<PathBuf> {
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
        // would hear still stops it by name, as the shipped app's exports did.
        let tick = |t: pm_protocol::common::Ticks| {
            t.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64
        };
        if request.without_offline && settings.audio && !settings.format.is_image() {
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
            let names: Vec<_> = offline.iter().take(3).map(|c| c.name.as_str()).collect();
            let more = match offline.len().saturating_sub(3) {
                0 => String::new(),
                n => format!(" and {n} more"),
            };
            let count = match offline.len() {
                1 => "1 clip is".to_owned(),
                n => format!("{n} clips are"),
            };
            anyhow::bail!(
                "{count} offline: {}{more}. {OFFLINE_ADVICE}",
                names.join(", ")
            );
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
            anyhow::bail!(
                "{} isn't installed ({uses}{others}). Install the extension that provides it, or turn the effect off, then export again.",
                missing.effect
            );
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
    while !worker.is_finished() {
        pm_text::host::serve(&mut font_service, broker.take_asks());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let result = worker
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("export worker panicked")));
    finished.store(true, Ordering::Release);
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
/// Renders `fill`'s frames as the paused viewer draws them (the viewer's
/// request at the composition's size, RGBA16F) into the render cache folder
/// captured when the fill started, stopping before the volume or the Render
/// Files Limit is full (`crate::render_cache`).
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
) -> Result<PathBuf> {
    use crate::render_cache::{DISK_FULL, LIMIT_FULL};
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
    let total = fill.frames.len().max(1) as f64;
    for (i, frame) in fill.frames.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!("cancelled");
        }
        let time = pm_protocol::common::Ticks::new(frame.ticks)?;
        let request = crate::render::FrameSpec::exact(time, output).request(&scene)?;
        let read = pm_export::render::render_frame_reusing(
            renderer,
            &doc,
            composition,
            time,
            &request,
            &scene.media,
            gate,
            pass,
            cancel,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let bytes: Vec<u8> = read
            .rgba
            .iter()
            .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
            .collect();
        let bytes = crate::frame_codec::encode(&bytes, fill.width, fill.height);
        if free_bytes(&fill.dir)? < DISK_RESERVE + bytes.len() as u64 {
            anyhow::bail!(DISK_FULL);
        }
        cache.put(&frame.key, &bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::StorageFull {
                anyhow::anyhow!(DISK_FULL)
            } else if e.to_string().contains("quota") {
                anyhow::anyhow!(LIMIT_FULL)
            } else {
                anyhow::Error::from(e)
            }
        })?;
        progress((i + 1) as f64 / total);
    }
    renderer.release_frame_reuse();
    Ok(fill.dir.clone())
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
}
impl Costs {
    fn record(&mut self, units: u32, us: f64) {
        if units == 0 || !us.is_finite() {
            return;
        }
        if units >= self.frame_units {
            self.whole_us = Some(us);
        } else if self.whole_us.is_some_and(|whole| us >= 0.75 * whole) {
            self.indivisible = true;
        }
        self.unit_us = Some(us / f64::from(units));
    }
    /// Units of `remaining` to submit within the budget: one unit to probe
    /// a job not measured yet, as many as fit otherwise, and the rest of the
    /// frame when bands cost a whole frame anyway.
    fn bounded(&self, remaining: u32) -> u32 {
        if self.indivisible {
            return remaining;
        }
        match self.unit_us {
            None => 1,
            Some(unit) => ((BAND_BUDGET_US / unit.max(1.)).floor() as u32).clamp(1, remaining),
        }
    }
}

/// Inherited-pipe admission: a whole-frame grant admits the rest of the frame
/// (the viewer has been quiet for [`QUIET`]), any other a submission bounded
/// by [`Costs::bounded`]. Cancellation and parent EOF interrupt a deferred job.
struct ChildGate {
    permits: Mutex<std::sync::mpsc::Receiver<bool>>,
    costs: Mutex<Costs>,
}
impl ChildGate {
    fn new(permits: std::sync::mpsc::Receiver<bool>) -> Self {
        Self {
            permits: Mutex::new(permits),
            costs: Mutex::default(),
        }
    }
}
impl pm_export::render::GpuGate for ChildGate {
    fn admit(&self, _: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        {
            let mut costs = self.costs.lock().unwrap();
            costs.frame_units = costs.frame_units.max(units);
        }
        send(&mut std::io::stdout().lock(), &ChildMessage::Gpu).ok()?;
        let permits = self.permits.lock().unwrap();
        while !cancel.load(Ordering::Acquire) {
            match permits.recv_timeout(std::time::Duration::from_millis(5)) {
                Ok(true) => return Some(units),
                Ok(false) => return Some(self.costs.lock().unwrap().bounded(units)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return None,
            }
        }
        None
    }
    fn finished(&self, _: u64, units: u32, measured_us: f64) {
        self.costs.lock().unwrap().record(units, measured_us);
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
