//! Production hosts behind MCP's service traits. Everything here runs on MCP or
//! export workers. Exports and frames use the same child job as the Export panel.
use crate::mcp::Tools;
use crate::raster_overrides::OverrideWindows;
use anyhow::{Context, Result as AnyResult};
use pm_engine_service::jobs::{ExportHandle, ExportRequest, JobSession};
use pm_kernel::media_archive::Archive;
use pm_kernel::{KernelHandle, Published};
use pm_mcp::{
    CaptureRequest, EngineHook, Error, Result,
    export_jobs::Progress,
    media::{MediaRegistry, MediaState},
    media_tools::{
        FrameRequest, MediaSource, Probe, SheetOutput, SheetPlan, SheetRequest, Waveform,
        WaveformRequest,
    },
    services::{
        ExportChild, FrozenProject, Image, ImportedMedia, MediaImport, MediaImporter, MediaRequest,
        ThreadArchive, VideoExportRequest,
    },
};
use pm_media::preview::PreviewReader;
use pm_model::json::to_canonical_string;
use pm_protocol::{
    color::{OutputColor, Precision},
    common::{FrameRate, SessionId, Size, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, QualityTier},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

type Resources = Arc<pm_engine_service::resources::ResourceSnapshot>;
/// Each open document's extension definitions (its window's), for the
/// captures and exports agents start; a request copies them when it starts.
static RESOURCES: Mutex<Option<HashMap<u64, Resources>>> = Mutex::new(None);
static SETTLED: std::sync::Condvar = std::sync::Condvar::new();
/// How long a capture or export waits for its window's extensions to load
/// before it goes ahead with what is registered (and says what is missing).
const EXTENSIONS_WAIT: Duration = Duration::from_secs(120);
/// Document `doc_id`'s definitions now (none: its window has no extensions).
pub(crate) fn set_resources(doc_id: u64, resources: Option<Resources>) {
    let mut all = RESOURCES.lock().unwrap();
    let all = all.get_or_insert_with(HashMap::new);
    match resources {
        Some(resources) => all.insert(doc_id, resources),
        None => all.remove(&doc_id),
    };
    SETTLED.notify_all();
}
/// Document `doc_id`'s definitions once its extensions have loaded: an
/// effect still loading is never exported or captured as not installed.
/// `wait: false` (an extension's own capture, which its realm may be loading
/// for) takes them as they are.
fn resources_of(doc_id: u64, wait: bool) -> Option<Resources> {
    let deadline = Instant::now() + EXTENSIONS_WAIT;
    let mut all = RESOURCES.lock().unwrap();
    loop {
        let resources = all.as_ref()?.get(&doc_id).cloned()?;
        let left = deadline.saturating_duration_since(Instant::now());
        if !wait || !resources.pending || left.is_zero() {
            return Some(resources);
        }
        all = SETTLED.wait_timeout(all, left).unwrap().0;
    }
}
/// A capture draws a missing effect bypassed, as the viewer does, but never
/// one its window's extensions may still register: past the wait it fails
/// saying the effect is still loading instead of showing the frame without it.
fn bypasses_missing(
    look: bool,
    resources: &pm_engine_service::resources::ResourceSnapshot,
) -> bool {
    look && !resources.pending
}
/// Install the production engine hosts. Call on a worker before serving.
pub fn connect_engine(
    tools: &Tools,
    activity: pm_engine_service::jobs::Activity,
    overrides: OverrideWindows,
) -> AnyResult<()> {
    let jobs = EngineJobs::new(
        tools.host.kernel.clone(),
        std::env::current_exe()?,
        activity,
        tools.host.profile.clone(),
        overrides,
    );
    let mut automation = tools.automation.lock().unwrap();
    automation.export_child = jobs.clone();
    automation.media_importer = Some(jobs.clone());
    automation.speech = Some(crate::agent_speech::AgentSpeech::new(
        tools.host.kernel.clone(),
        tools.host.profile.clone(),
    ));
    automation.media_registry = Some(jobs.clone());
    automation.threads = Some(jobs.clone());
    automation.engine = Some(jobs);
    let extensions = crate::extension_authoring::AppExtensions::new(tools.host.profile.clone());
    // Before any window scans: finish or undo a promotion a crash interrupted.
    if let Err(error) = pm_mcp::extensions::recover_all(&extensions) {
        eprintln!("Extension change recovery failed: {error}");
    }
    automation.extensions = Some(Arc::new(extensions));
    automation.store_reads = Some(Arc::new(StoreHttp));
    Ok(())
}
/// The Store's public read routes over the system HTTP stack, for
/// `store_search`, `store_extension` and `store_source` (runs on MCP workers).
struct StoreHttp;
impl pm_mcp::store::StoreRead for StoreHttp {
    fn get(&self, request: pm_mcp::store::Request) -> Result<pm_mcp::store::Response> {
        let failed = |message: &str| Error {
            code: "store_error".into(),
            message: message.into(),
            backing: Some("native Store transport".into()),
        };
        let origin = url::Url::parse(pm_compat_runner::store_install::ORIGIN)
            .map_err(|_| Error::invalid("Invalid Store origin"))?;
        let mut url = origin
            .join(&request.path)
            .map_err(|_| Error::invalid("Invalid Store route"))?;
        if !request.path.starts_with("/v1/store/")
            || request.path.contains(['?', '#'])
            || url.origin() != origin.origin()
            || request.max_bytes > pm_mcp::store::JSON_LIMIT
        {
            return Err(Error::invalid("Invalid native Store read request"));
        }
        if !request.query.is_empty() {
            url.query_pairs_mut().extend_pairs(request.query);
        }
        // Never leaves the Store's origin, even by redirect. Transport errors
        // can carry hosts and system detail: none crosses MCP.
        let response = pm_compat_runner::net::get_within(
            pm_compat_runner::store_install::ORIGIN,
            url.as_str(),
            request.max_bytes,
            30,
        )
        .map_err(|e| match e {
            pm_runtime_v8::Error::Host(m) if m.contains("sent elsewhere") => {
                failed("The Store redirected the read to another origin")
            }
            _ => failed("The Store didn't answer. Check the connection and try again."),
        })?;
        Ok(pm_mcp::store::Response {
            status: response.status,
            body: response.body,
        })
    }
}
pub fn headless(profile: PathBuf, exports: PathBuf, launch: Option<PathBuf>) -> anyhow::Result<()> {
    std::thread::spawn(move || -> anyhow::Result<()> {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
            journal_root: Some(profile.join("journals")),
            backup_root: Some(profile.join("backups")),
            write_coordinator: Some(crate::file_coordination::system()),
            trash: Some(crate::file_coordination::trash()),
            ..Default::default()
        });
        let host = crate::mcp::Host::new(actor.handle(), profile.clone());
        crate::mcp::attach(&host.kernel, &profile)?;
        if let Some(path) = launch {
            host.open(
                crate::session::stable_id(&path),
                pm_kernel::session::SessionSource::File(path),
            )?;
        }
        let tools = Tools::new(host, exports);
        connect_engine(&tools, Default::default(), Default::default())?;
        let shared = tools.clone();
        let server =
            pm_mcp::transport::LoopbackServer::start_with(&profile, move || shared.connect())?;
        let result = pm_mcp::transport::serve_stdio(
            std::io::stdin().lock(),
            std::io::stdout().lock(),
            tools.connect(),
        );
        drop(server);
        actor.shutdown();
        result.map_err(Into::into)
    })
    .join()
    .map_err(|_| anyhow::anyhow!("MCP worker panicked"))?
}
/// Starts engine child jobs from the running executable. `activity` lets the
/// viewer keep precedence over export bands, as it does for panel exports.
pub struct EngineJobs {
    kernel: KernelHandle,
    executable: PathBuf,
    activity: pm_engine_service::jobs::Activity,
    profile: PathBuf,
    cache: PathBuf,
    progress: Mutex<HashMap<u64, Progress>>,
    /// Whose extensions answer a project's raster overrides.
    overrides: OverrideWindows,
}
impl EngineJobs {
    pub fn new(
        kernel: KernelHandle,
        executable: PathBuf,
        activity: pm_engine_service::jobs::Activity,
        profile: PathBuf,
        overrides: OverrideWindows,
    ) -> Arc<Self> {
        Arc::new(Self {
            kernel,
            executable,
            activity,
            cache: profile.join("cache/mcp"),
            profile,
            progress: Mutex::default(),
            overrides,
        })
    }
    fn published(&self, frozen: &FrozenProject) -> Result<Arc<Published>> {
        self.kernel
            .published(&frozen.project_id)
            .filter(|p| p.snapshot.doc_id == frozen.document_id && p.revision() == frozen.revision)
            .ok_or_else(|| Error::conflict("The composition changed during engine work"))
    }
    fn start(
        &self,
        published: &Arc<Published>,
        settings: ExportSettings,
        destination: PathBuf,
        look: bool,
        preview: bool,
        resources: Option<Resources>,
    ) -> AnyResult<ExportHandle> {
        let media = crate::media::for_session(
            &published.snapshot,
            &published.media,
            published.path.as_deref(),
            &crate::media::cache_root(&self.profile),
        )?
        .catalog;
        let mut request = ExportRequest::new(
            pm_kernel::new_id("export").as_str().into(),
            &published.snapshot,
            JobSpec::Export {
                session: SessionId::new(published.snapshot.doc_id)?,
                composition: published.snapshot.render_comp().as_str().into(),
                settings,
            },
            media,
            destination,
        )?;
        request.preview = preview;
        // An agent's capture draws the effects its project's window defines,
        // as its caller pinned them (`resources_of`).
        if let Some(resources) = resources {
            request.resources = (*resources).clone();
        }
        // A capture shows the project as the viewer does; a video is a delivery.
        request.bypass_missing_effects = bypasses_missing(look, &request.resources);
        // As the shipped app's agent exports: silent offline clips are left
        // out, not asked about; missing audio still fails by name. Captures
        // refuse offline media unless they draw the card (agent stills).
        request.without_offline = !look;
        let mut session = JobSession::new(request);
        if let Some(overrides) = self.overrides.of(published.snapshot.doc_id) {
            // A look at the window's project now may ask its realm; a frame of
            // a frozen publication (an export realm's capture) is that realm's.
            let current = self
                .kernel
                .published(&published.project)
                .is_some_and(|now| Arc::ptr_eq(&now, published));
            overrides.pin(&mut session, published, look && current);
        }
        ExportHandle::start_scheduled(&self.executable, session, self.activity.clone())
    }
}
fn failed(error: impl std::fmt::Display) -> Error {
    Error {
        code: "export_failed".into(),
        message: error.to_string(),
        backing: Some("pm-engine-service".into()),
    }
}
fn finish(handle: ExportHandle) -> Result<PathBuf> {
    let status = handle.wait().map_err(failed)?;
    match status.state {
        JobState::Done => status
            .path
            .map(PathBuf::from)
            .ok_or_else(|| failed("Export completed without an output path")),
        _ => Err(failed(status.error.map(|e| e.message).unwrap_or_else(
            || "Powermove lost contact with the export engine. Try again.".into(),
        ))),
    }
}
/// The published name made safe for a file, with a unique suffix.
fn file_name(name: &str, extension: &str) -> String {
    let stem = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect::<String>();
    format!("{stem}-{}.{extension}", pm_kernel::new_id("export"))
}
impl ExportChild for EngineJobs {
    fn video(&self, request: &VideoExportRequest) -> Result<PathBuf> {
        let frozen = &request.snapshot;
        let published = self.published(frozen)?;
        let project = &frozen.project;
        let options = pm_builtins::delivery::options(&json!({
            "format": request.options.format,
            "scale": 1,
            "width": project["w"],
            "height": project["h"],
            "fps": project["fps"],
            "range": request.options.range,
            "quality": "high",
            "mblur": request.options.motion_blur,
            "audio": request.options.audio,
            "alpha": request.options.alpha,
            "captionsBurn": false,
            "captionsSidecar": "none",
        }))
        .map_err(Error::invalid)?;
        let time = project["time"].as_f64().unwrap_or(0.);
        let plan = pm_builtins::delivery::plan(&published.snapshot, &options, time)
            .map_err(Error::invalid)?;
        let settings = plan
            .settings
            .clone()
            .ok_or_else(|| Error::invalid("Missing render settings"))?;
        std::fs::create_dir_all(&request.directory)?;
        let name = request
            .options
            .name
            .clone()
            .or_else(|| project["name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Untitled".into());
        let destination = request
            .directory
            .join(file_name(&name, options.format.extension()));
        // Listed in Background Tasks beside the person's exports, where it can
        // be watched and cancelled.
        let task =
            crate::export_coordinator::AgentExport::start(&name, &frozen.project_id, plan.frames)
                .map_err(failed)?;
        let resources = resources_of(published.snapshot.doc_id, true);
        let handle = match self.start(&published, settings, destination, false, false, resources) {
            Ok(handle) => handle,
            Err(error) => {
                task.finish(Err(error.to_string()));
                return Err(failed(error));
            }
        };
        let started = Instant::now();
        while !matches!(
            handle.status().state,
            JobState::Done | JobState::Failed | JobState::Cancelled
        ) {
            if task.cancelled() {
                handle.cancel();
                break;
            }
            let frames = (handle.status().progress * plan.frames as f64).round() as u64;
            task.progress(frames, plan.frames);
            self.progress.lock().unwrap().insert(
                frozen.document_id,
                Progress {
                    frames,
                    total: Some(plan.frames),
                    encoder: None,
                    seconds: started.elapsed().as_secs_f64(),
                },
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        self.progress.lock().unwrap().remove(&frozen.document_id);
        if task.cancelled() {
            let _ = handle.wait();
            task.finish(Err("cancelled".into()));
            return Err(failed(
                "The person cancelled this export in Background Tasks.",
            ));
        }
        let result = finish(handle);
        task.finish(result.as_ref().map(|_| ()).map_err(|e| e.message.clone()));
        result
    }
    fn video_progress(&self, frozen: &FrozenProject) -> Option<Progress> {
        self.progress
            .lock()
            .unwrap()
            .get(&frozen.document_id)
            .cloned()
    }
}
/// One composition frame as PNG bytes, through the production export renderer.
/// The extension realm's `render.renderFrameTo`/`snapshot`: the open
/// composition of `project`, rendered by the engine as one PNG frame.
pub(crate) fn frame_renderer(
    kernel: KernelHandle,
    project: String,
    profile: PathBuf,
    overrides: OverrideWindows,
    capture: Capture,
) -> AnyResult<pm_compat_runner::FrameRenderer> {
    let jobs = EngineJobs::new(
        kernel.clone(),
        std::env::current_exe()?,
        Default::default(),
        profile,
        overrides,
    );
    let epochs = jobs.overrides.clone();
    Ok(cached_frames(
        kernel,
        project,
        capture,
        move |doc_id| epochs.of(doc_id).and_then(|o| o.epoch()),
        move |published, request, resources| {
            let root = published.snapshot.root().ok_or("No open project")?;
            let open = published.snapshot.open_comp();
            let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
            let fps = comp
                .get("fps")
                .and_then(|n| n.to_value().as_f64())
                .unwrap_or(30.);
            std::fs::create_dir_all(&jobs.cache).map_err(|e| e.to_string())?;
            let look = Look {
                alpha: AlphaOutput::None,
                motion_blur: request.motion_blur,
            };
            // An extension's snapshot (renderFrameTo, PM.Export.snapshot) is a
            // delivery: offline media refuses, so no Media Offline card is ever
            // cached here or saved by an extension. Home's poster is a look at
            // the project and draws the card (its renderer is its own).
            frame(
                &jobs,
                published,
                fps,
                request.time,
                Size::new(request.width, request.height),
                look,
                request.stop,
                capture,
                resources,
            )
            .map_err(|e| e.to_string())
        },
    ))
}
/// A frame renderer that keeps the last few frames it drew: a panel that asks
/// for the same frame again (thumbnails, a check on every redraw) gets it
/// without a render. A kept frame is the same document revision drawn by the
/// same override wrappers (`epoch`) and the same extension definitions: the
/// window's resources are pinned before the lookup and `render` draws with
/// those, so a frame drawn before an extension reloaded is never the answer
/// after it. While extensions load nothing is kept or served: a frame drawn
/// then lacks, or is about to gain, an effect.
fn cached_frames(
    kernel: KernelHandle,
    project: String,
    capture: Capture,
    epoch: impl Fn(u64) -> Option<u64> + Send + Sync + 'static,
    render: impl Fn(
        &Arc<Published>,
        &pm_compat_runner::FrameRequest,
        Option<Resources>,
    ) -> std::result::Result<Vec<u8>, String>
    + Send
    + Sync
    + 'static,
) -> pm_compat_runner::FrameRenderer {
    let cache: Mutex<std::collections::VecDeque<(String, Arc<Vec<u8>>)>> = Mutex::default();
    Arc::new(move |request: &pm_compat_runner::FrameRequest| {
        let published = match &request.published {
            Some(frozen) => frozen.clone(),
            None => kernel.published(&project).ok_or("No open project")?,
        };
        let doc_id = published.snapshot.doc_id;
        let resources = resources_of(doc_id, capture != Capture::ExtensionSnapshot);
        let key = resources.as_ref().is_none_or(|r| !r.pending).then(|| {
            format!(
                "{doc_id}:{}:{:?}:{:?}:{}:{}x{}:{}",
                published.revision(),
                epoch(doc_id),
                resources.as_ref().map(|r| r.generation),
                request.time.to_bits(),
                request.width,
                request.height,
                request.motion_blur
            )
        });
        if let Some(key) = &key
            && let Some((_, png)) = cache.lock().unwrap().iter().find(|(k, _)| k == key)
        {
            return Ok(png.as_ref().clone());
        }
        let png = render(&published, request, resources)?;
        if let Some(key) = key {
            let mut cache = cache.lock().unwrap();
            if cache.len() >= 4 {
                cache.pop_front();
            }
            cache.push_back((key, Arc::new(png.clone())));
        }
        Ok(png)
    })
}
/// How a captured frame looks: MCP captures keep alpha; an extension's
/// `renderFrameTo` is opaque over black with motion blur, as main draws it.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    alpha: AlphaOutput,
    motion_blur: bool,
}
/// An agent's still and contact sheet: opaque over the composition's
/// background, as the viewer shows it (v1's snapshots were JPEGs).
const MCP_LOOK: Look = Look {
    alpha: AlphaOutput::None,
    motion_blur: false,
};
fn never() -> bool {
    false
}
/// Renders one frame; `stop` cancels the export (a stopped task, a deadline).
/// Who asked for a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Capture {
    /// An agent's render_frames: a look at the project.
    AgentStill,
    /// An extension's renderFrameTo or PM.Export.snapshot: a delivery.
    ExtensionSnapshot,
    /// A composition sheet.
    Sheet,
    /// Home's poster frame of a project: a look at it.
    Poster,
}
impl Capture {
    /// A look at the project (an agent's still, Home's poster) draws the
    /// Media Offline card; a delivery refuses offline media, as an export
    /// does.
    pub(crate) fn draws_offline_cards(self) -> bool {
        matches!(self, Capture::AgentStill | Capture::Poster)
    }
}
#[allow(clippy::too_many_arguments)]
fn frame(
    jobs: &EngineJobs,
    published: &Arc<Published>,
    fps: f64,
    time: f64,
    size: Size,
    look: Look,
    stop: &(dyn Fn() -> bool + Sync),
    capture: Capture,
    resources: Option<Resources>,
) -> AnyResult<Vec<u8>> {
    let rate = pm_time::rate_from_document(None, fps)?.rate;
    let start = pm_time::ticks_from_seconds(time)?;
    let span = rate.den() as f64 / rate.num() as f64;
    let end = pm_time::ticks_from_seconds(time + span)?;
    let output = tempfile::tempdir_in(jobs.cache.as_path())?;
    let destination = output.path().join("frame");
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: false },
        alpha: look.alpha,
        quality: QualityTier::High,
        size,
        frame_rate: FrameRate::new(u32::try_from(rate.num())?, u32::try_from(rate.den())?),
        range: TickRange {
            start: Ticks::new(start)?,
            end: Ticks::new(end)?,
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: look.motion_blur,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        dither: false,
        // A look at the project, as the viewer shows it: proxy-only media
        // draws from its proxy. Exports still refuse it without a choice.
        draft_from_proxies: true,
        delivery: Default::default(),
    };
    let handle = jobs.start(
        published,
        settings,
        destination,
        true,
        capture.draws_offline_cards(),
        resources,
    )?;
    while matches!(
        handle.status().state,
        JobState::Queued | JobState::Running | JobState::WaitingForMedia
    ) {
        if stop() {
            handle.cancel();
            let _ = handle.wait();
            anyhow::bail!("The frame capture was stopped");
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    let directory = finish(handle)?;
    first_png(&directory)
}
fn first_png(directory: &Path) -> AnyResult<Vec<u8>> {
    let mut frames = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "png"))
        .collect::<Vec<_>>();
    frames.sort();
    Ok(std::fs::read(
        frames.first().context("Render produced no frame")?,
    )?)
}
fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
const IMPORT_LIMIT: u64 = 512 << 20;
/// Import Media's kinds and types, plus the 3D models only agents import.
fn media_type(path: &Path) -> Option<(&'static str, String)> {
    if let Some(found) = pm_builtins::media_import::classify(path) {
        return Some(found);
    }
    let extension = path.extension()?.to_str()?.to_lowercase();
    let mime = match extension.as_str() {
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        "obj" => "model/obj",
        _ => return None,
    };
    Some(("model", mime.into()))
}
/// Main's identity: stable across renames, sampled so it never reads a whole file.
fn fingerprint(path: &Path, size: u64) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    const SAMPLE: u64 = 64 << 10;
    let sample = SAMPLE.min(size);
    let mut digest = Sha256::new();
    digest.update(format!("powermove-media-v2\n{size}\n"));
    let mut file = std::fs::File::open(path)?;
    let mut offsets = vec![0, (size - sample) / 2, size - sample];
    offsets.dedup();
    let mut buffer = vec![0; sample as usize];
    for offset in offsets {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buffer)?;
        digest.update(&buffer);
    }
    let hex = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("v2:{size}:{hex}"))
}
impl MediaImporter for EngineJobs {
    fn import(&self, request: &MediaImport) -> Result<ImportedMedia> {
        let path = &request.path;
        if !path.is_absolute() {
            return Err(Error::invalid("Provide an absolute local media file path."));
        }
        let size = std::fs::metadata(path)
            .map_err(|_| {
                Error::invalid(
                    "The source is unavailable locally. Download cloud files first; no media was changed.",
                )
            })?
            .len();
        if size > IMPORT_LIMIT {
            return Err(Error::invalid("Import local files up to 512 MiB."));
        }
        let (kind, mime) =
            media_type(path).ok_or_else(|| Error::invalid("Unsupported media file."))?;
        let fingerprint = fingerprint(path, size).map_err(failed)?;
        if request
            .expected_fingerprint
            .as_ref()
            .is_some_and(|expected| *expected != fingerprint)
        {
            return Err(Error::invalid(
                "The chosen file does not match expectedFingerprint; no media was changed.",
            ));
        }
        let project = &request.frozen.project;
        if let Some(asset) = &request.replace
            && project["assets"][asset]["kind"] != kind
        {
            return Err(Error::invalid(
                "Replace media with a file of the same kind.",
            ));
        }
        let probe = (kind != "model")
            .then(|| pm_media::probe::probe(path).map(|probed| probed.probe))
            .transpose()
            .map_err(|_| Error::invalid("This file could not be read as media."))?;
        let video = probe.as_ref().and_then(|probe| probe.video.as_ref());
        let width = video.map(|video| f64::from(video.width));
        let height = video.map(|video| f64::from(video.height));
        // The clip's own length (first frame start to last frame end), as
        // the Project panel's import records it.
        // A relinked asset keeps reading its file's times as it did, so its
        // lengths are measured that way.
        let timing = match &request.replace {
            Some(asset) => pm_builtins::media_import::asset_timing(project["assets"].get(asset)),
            None => pm_media::clip::ClipTiming::Presentation,
        };
        let duration = probe
            .as_ref()
            .and_then(|probe| probe.duration_as(timing))
            .filter(|_| kind != "image");
        let has_audio = probe
            .as_ref()
            .is_some_and(|probe| probe.streams.iter().any(|stream| stream.audio.is_some()));
        let archive = Archive::open(&self.profile).map_err(failed)?;
        let original = archive
            .retain_path(path, &mime, &AtomicBool::new(false))
            .map_err(failed)?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let imported = |status, asset: String, revision| ImportedMedia {
            status,
            asset,
            name: name.clone(),
            kind: kind.into(),
            width,
            height,
            duration,
            has_audio,
            fingerprint: fingerprint.clone(),
            revision,
        };
        let published = self.published(&request.frozen)?;
        let same_bytes = |entry: &pm_kernel::project_io::MediaEntry| {
            use pm_kernel::project_io::MediaSource::{Embedded, File, Memory};
            match &entry.source {
                File { sha256, .. } | Embedded { sha256, .. } => {
                    sha256.as_deref() == Some(original.hash())
                }
                Memory(_) => false,
            }
        };
        if request.replace.is_none()
            && let Some(entry) = published.media.iter().find(|entry| {
                same_bytes(entry) && project["assets"][entry.id.to_string()].is_object()
            })
        {
            return Ok(imported(
                "already_imported",
                entry.id.to_string(),
                request.frozen.revision,
            ));
        }
        let id = request
            .replace
            .clone()
            .unwrap_or_else(|| pm_kernel::new_id("asset"));
        let previous = request
            .replace
            .as_ref()
            .and_then(|asset| published.media.get(asset))
            .map(|entry| archive.retain_entry(entry))
            .transpose()
            .map_err(failed)?;
        let mut metadata = json!({"name":name,"kind":kind,"mimeType":mime,"fingerprint":fingerprint,
            "sourcePath":path,"size":size,"w":width.unwrap_or(0.),"h":height.unwrap_or(0.),
            "dur":duration.unwrap_or(0.),"hasAudio":has_audio,"persisted":true});
        if kind == "video" {
            metadata["audioDur"] = json!(
                probe
                    .as_ref()
                    .and_then(|probe| pm_media::facts::audio_duration(probe, timing))
                    .unwrap_or(0.)
            );
        }
        // Every audio stream of a multi-track file is listed and heard, as
        // when a person imports it; a relink keeps what was heard.
        let streams = probe
            .as_ref()
            .map(pm_media::facts::audio_streams)
            .unwrap_or_default();
        if let Some(asset) = &request.replace {
            let old = published
                .snapshot
                .root()
                .and_then(|root| root.get("assets")?.get(asset)?.get("audioStreams"))
                .and_then(|streams| {
                    serde_json::from_str(&pm_model::json::to_canonical_string(&streams.to_value()))
                        .ok()
                });
            pm_builtins::media_import::relink_streams(
                &mut metadata,
                old.as_ref(),
                &streams,
                previous.as_ref(),
                &original,
            );
        } else if let Some(streams) = pm_media::facts::asset_audio_streams(&streams) {
            metadata["audioStreams"] = streams;
        }
        // As a person's import: a new clip's times address the file's
        // presentation; a replaced asset keeps the reading it was edited with.
        if request.replace.is_none() && matches!(kind, "video" | "audio") {
            metadata["timing"] = json!(pm_media::clip::ClipTiming::imported());
        }
        let catalog =
            pm_panel_media::Catalog::published(&published).map_err(|e| failed(format!("{e:?}")))?;
        let edit = catalog
            .prepare_import(
                &catalog.stamp,
                vec![pm_panel_media::StoredAsset {
                    id: id.clone(),
                    metadata: pm_model::json::parse(&metadata.to_string()).map_err(failed)?,
                    original_version: original.version().into(),
                }],
                request.folder.clone(),
                request.replace.clone(),
            )
            .map_err(|e| match e {
                pm_panel_media::Error::MissingItem => {
                    Error::invalid("Choose an existing media folder.")
                }
                other => failed(format!("{other:?}")),
            })?;
        let expected = published
            .snapshot
            .root()
            .and_then(|root| root.get("assets"))
            .and_then(|assets| assets.get(&id))
            .map(pm_model::Node::to_value);
        let writer = pm_kernel::Caller::realm(request.writer, 1, pm_kernel::Origin::Mcp);
        let ack = self
            .kernel
            .call(
                &request.frozen.project_id,
                edit.request_with_media(
                    writer,
                    vec![pm_kernel::MediaBinding {
                        asset: id.clone(),
                        expected,
                        original,
                        previous,
                    }],
                    vec![],
                ),
            )
            .wait()
            .ok_or_else(|| Error::unavailable("The app is closing."))?
            .map_err(Error::kernel)?;
        if let pm_kernel::Reply::Throw(error) = &ack.reply {
            return Err(Error::kernel(error));
        }
        let status = if request.replace.is_some() {
            "replaced"
        } else {
            "imported"
        };
        Ok(imported(status, id, ack.revision))
    }
}
/// The last thing the agent said, which is what a caller wants at a glance.
fn thread_summary(thread: &Value) -> Value {
    let messages = thread["conversation"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    pm_agents::conversation::for_agent(messages)
        .into_iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map_or(Value::Null, |message| json!(message.text))
}
fn describe_thread(thread: &Value) -> Value {
    let settings = |key: &str| thread[key].clone();
    let status = thread["lastRunStatus"].as_str().unwrap_or("idle");
    json!({"threadId":thread["id"],"title":thread["title"],"updatedAt":thread["updatedAt"],
        "provider":settings("provider"),"model":settings("model"),
        "reasoningEffort":settings("reasoningEffort"),"access":thread["access"].as_str().unwrap_or("editor"),
        "busy":matches!(status, "queued" | "running" | "waiting"),"runId":thread["lastRunId"],
        "status":status,"summary":thread_summary(thread)})
}
impl ThreadArchive for EngineJobs {
    fn read(&self, tool: &str, project: Option<&str>, args: &Value) -> Result<Value> {
        let state = self.kernel.state();
        let decode = |value: &pm_model::Value| {
            serde_json::from_str::<Value>(&to_canonical_string(value)).ok()
        };
        // The application Agent panel (no project) nests its archive.
        let archive = match project {
            Some(project) => state
                .settings
                .get(&format!("agentThreads.{project}"))
                .and_then(decode),
            None => state
                .settings
                .get(pm_builtins::APPLICATION_AGENT_STORE)
                .and_then(decode)
                .map(|fields| fields["agentThreads.builtin-application"].clone())
                .filter(Value::is_object),
        }
        .unwrap_or_else(|| json!({"threads":[]}));
        let threads = archive["threads"].as_array().map_or(&[][..], Vec::as_slice);
        let count = |key: &str, default: usize| args[key].as_u64().map_or(default, |n| n as usize);
        let offset = count("offset", 0);
        if tool == "thread_list" {
            let limit = count("limit", 30);
            let page = threads.iter().skip(offset).take(limit);
            return Ok(
                json!({"threads":page.map(describe_thread).collect::<Vec<_>>(),
                "total":threads.len(),
                "nextOffset":(offset + limit < threads.len()).then_some(offset + limit)}),
            );
        }
        let thread = threads
            .iter()
            .find(|thread| thread["id"] == args["threadId"])
            .ok_or_else(|| Error::invalid("This thread does not exist in the current project."))?;
        let conversation = thread["conversation"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let limit = count("limit", 20);
        let start = count("textOffset", 0);
        let width = count("maxCharsPerMessage", 12_000).min(160_000 / limit);
        let messages = conversation
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(position, message)| {
                let source = pm_agents::conversation::message_text(message)
                    .encode_utf16()
                    .collect::<Vec<_>>();
                let end = (start + width).min(source.len());
                let text = String::from_utf16_lossy(source.get(start..end).unwrap_or_default());
                let role = if message["role"] == "trace" {
                    json!("assistant")
                } else {
                    message["role"].clone()
                };
                json!({"position":position,"role":role,"text":text,"truncated":end < source.len(),
                "nextTextOffset":(end < source.len()).then_some(end)})
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"thread":describe_thread(thread),"messages":messages,"activity":"","questions":[],
            "nextOffset":(offset + limit < conversation.len()).then_some(offset + limit)}),
        )
    }
}
impl MediaRegistry for EngineJobs {
    fn state(&self, project: &str, asset: &str) -> MediaState {
        let Some(published) = self.kernel.published(project) else {
            return MediaState::default();
        };
        let linked = || {
            let root = published.snapshot.root()?;
            let record = root.get("assets")?.get(asset)?.to_value();
            let path = record.get("sourcePath").or_else(|| record.get("path"))?;
            Some(Path::new(&*path.as_js_string()?.to_string_lossy()).is_file())
        };
        MediaState {
            available: published.media.get(asset).is_some() || linked() == Some(true),
            ..Default::default()
        }
    }
}
impl EngineHook for EngineJobs {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        Err(Error::invalid("Frame capture needs a frozen project"))
    }
    fn capture_snapshot(&self, request: &CaptureRequest, frozen: &FrozenProject) -> Result<Value> {
        let published = self.published(frozen)?;
        let project = &frozen.project;
        let (w, h) = (
            project["w"].as_f64().unwrap_or(1920.),
            project["h"].as_f64().unwrap_or(1080.),
        );
        let fps = project["fps"].as_f64().unwrap_or(30.);
        let width = request.width.clamp(16, 8192);
        let height = ((width as f64 * h / w).round() as u32).max(16);
        std::fs::create_dir_all(&self.cache)?;
        let mut images = vec![];
        for time in &request.times {
            let size = Size::new(width, height);
            let bytes =
                // render_frames is a look at the project: offline media draws
                // the Media Offline card. Nothing caches these stills.
                frame(
                    self,
                    &published,
                    fps,
                    *time,
                    size,
                    MCP_LOOK,
                    &never,
                    Capture::AgentStill,
                    resources_of(published.snapshot.doc_id, true),
                )
                .map_err(failed)?;
            images.push(format!("data:image/png;base64,{}", base64(&bytes)));
        }
        Ok(json!({"times":request.times,"images":images}))
    }
    fn resolve_media(&self, request: &MediaRequest) -> Result<MediaSource> {
        if request.layer.is_some() {
            return Err(Error::unavailable(
                "evaluated clip timing for layer targets; pass the assetId instead",
            ));
        }
        let path = self.original(request)?;
        let timing = pm_media::probe::probe_timing(&path).map_err(failed)?;
        Ok(MediaSource {
            asset: request.asset.clone().unwrap_or_default(),
            duration: seconds(timing.duration_us),
            proxy: false,
            layer: None,
        })
    }
    fn probe(&self, request: &MediaRequest, _: &MediaSource) -> Result<Probe> {
        let probe = pm_media::probe::probe(&self.original(request)?)
            .map_err(failed)?
            .probe;
        let fps = probe.video.as_ref().and_then(|video| {
            video
                .nominal_rate
                .as_ref()
                .map(|rate| rate.rate.num() as f64 / rate.rate.den() as f64)
        });
        let video = probe
            .video
            .iter()
            .map(|video| {
                json!({"index":probe.video_stream,"codec":video.codec.codec,"width":video.width,
                    "height":video.height,"fps":fps,"alpha":video.alpha.is_some()})
            })
            .collect();
        let mut audio = vec![];
        let mut other = 0;
        for stream in &probe.streams {
            match &stream.audio {
                Some(a) => audio.push(json!({"index":stream.index,"codec":a.codec,
                    "sampleRate":a.sample_rate,"channels":a.channels})),
                None if Some(stream.index) != probe.video_stream => other += 1,
                None => {}
            }
        }
        let duration = seconds(probe.duration_us);
        Ok(Probe {
            format: Some(probe.container.clone()),
            duration,
            start: seconds(probe.start_us).unwrap_or(0.),
            bitrate_kbps: duration
                .filter(|d| *d > 0.)
                .map(|d| probe.size as f64 * 8. / d / 1000.),
            video,
            audio,
            other_streams: other,
        })
    }
    fn sample(&self, request: &FrameRequest) -> Result<Vec<Image>> {
        let mut reader = self.reader(&request.request)?;
        request
            .frames
            .iter()
            .map(|frame| {
                let rgba = source_frame(&mut reader, frame.source).map_err(failed)?;
                let pixels = fit(&rgba, request.width, request.height);
                image(&pixels, request.width, request.height)
            })
            .collect()
    }
    fn sheet(&self, request: &SheetRequest) -> Result<Image> {
        let mut reader = self.reader(&request.request)?;
        let cells = request
            .cells
            .iter()
            .map(|cell| source_frame(&mut reader, cell.source).map_err(failed))
            .collect::<Result<Vec<_>>>()?;
        sheet(&cells, &request.plan)
    }
    /// Each cell is a full engine job, so a long sheet returns a finished prefix.
    fn composition_sheet(&self, request: &SheetRequest) -> Result<SheetOutput> {
        let frozen = &request.request.snapshot;
        let published = self.published(frozen)?;
        let fps = frozen.project["fps"].as_f64().unwrap_or(30.);
        let size = Size::new(request.plan.cell_width, request.plan.cell_height);
        std::fs::create_dir_all(&self.cache)?;
        let started = Instant::now();
        let mut cells = vec![];
        for cell in &request.cells {
            if !cells.is_empty() && started.elapsed() > Duration::from_secs(45) {
                break;
            }
            let bytes = frame(
                self,
                &published,
                fps,
                cell.source,
                size,
                MCP_LOOK,
                &never,
                Capture::Sheet,
                resources_of(published.snapshot.doc_id, true),
            )
            .map_err(failed)?;
            cells.push(decode_png(&bytes).map_err(failed)?);
        }
        let aspect = request.plan.cell_width as f64 / request.plan.cell_height as f64;
        let plan = pm_mcp::media_tools::plan_sheet(
            cells.len(),
            aspect,
            Some(request.plan.columns).filter(|_| cells.len() == request.cells.len()),
        );
        Ok(SheetOutput {
            image: sheet(&cells, &plan)?,
            completed: cells.len(),
        })
    }
    /// Loudness and silences of what the mix plays from the asset: its heard
    /// streams on its clip's timeline (`pm_audio::waveform::clip_envelope`).
    fn waveform(&self, request: &WaveformRequest) -> Result<Waveform> {
        const BUCKETS_PER_SECOND: f64 = 100.;
        let path = self.original(&request.request)?;
        let published = self.published(&request.request.snapshot)?;
        let record = request
            .request
            .asset
            .as_ref()
            .and_then(|asset| asset["id"].as_str())
            .and_then(|id| published.snapshot.root()?.get("assets")?.get(id));
        let streams = pm_audio::graph::selected_streams(record);
        if streams.is_empty() {
            return Err(Error::invalid("This media's audio streams are all off."));
        }
        let (envelope, rate) = pm_audio::waveform::clip_envelope(
            &path,
            &streams,
            pm_audio::graph::clip_timing(record),
            BUCKETS_PER_SECOND,
        )
        .map_err(|_| Error::invalid("This media has no audio."))?;
        let envelope: Vec<f64> = envelope.into_iter().map(f64::from).collect();
        let bucket_at = |time: f64| ((time * rate) as usize).min(envelope.len());
        let span = &envelope[bucket_at(request.start)..bucket_at(request.end)];
        let peak = span.iter().copied().fold(0., f64::max);
        let threshold = 10f64.powf(request.threshold_db / 20.);
        let mut silences = vec![];
        let mut quiet = None;
        for (index, level) in span.iter().chain([&f64::MAX]).enumerate() {
            let time = request.start + index as f64 / rate;
            match (quiet, *level < threshold) {
                (None, true) => quiet = Some(time),
                (Some(start), false) => {
                    if time - start >= request.min_silence {
                        silences.push([start, time.min(request.end)]);
                    }
                    quiet = None;
                }
                _ => {}
            }
        }
        let picture = if request.image && !span.is_empty() {
            const HEIGHT: u32 = 120;
            let mut pixels = vec![0u8; (request.width * HEIGHT * 4) as usize];
            for x in 0..request.width as usize {
                let level = span[x * span.len() / request.width as usize].min(1.);
                let reach = (level * HEIGHT as f64 / 2.) as i64;
                for offset in -reach..=reach {
                    let y = (HEIGHT as i64 / 2 + offset).clamp(0, HEIGHT as i64 - 1) as usize;
                    let at = (y * request.width as usize + x) * 4;
                    pixels[at..at + 4].copy_from_slice(&[255; 4]);
                }
            }
            Some(image(&pixels, request.width, HEIGHT)?)
        } else {
            None
        };
        Ok(Waveform {
            loudness: json!({"peakDb":decibels(peak),"clipping":peak >= 1.}),
            silences,
            image: picture,
        })
    }
}

type Rgba = (Vec<u8>, u32, u32);
pub(crate) fn png(pixels: &[u8], width: u32, height: u32) -> AnyResult<Vec<u8>> {
    let mut bytes = vec![];
    let mut encoder = ::png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(::png::ColorType::Rgba);
    encoder.set_depth(::png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(bytes)
}
fn decode_png(bytes: &[u8]) -> AnyResult<Rgba> {
    let mut decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::ALPHA | ::png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().context("Frame too large")?];
    let info = reader.next_frame(&mut pixels)?;
    pixels.truncate(info.buffer_size());
    Ok((pixels, info.width, info.height))
}
/// Nearest-neighbour fit. These frames are for an agent to look at, not deliver.
fn fit(source: &Rgba, width: u32, height: u32) -> Vec<u8> {
    let (pixels, w, h) = source;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let sx = (x as u64 * *w as u64 / width as u64) as usize;
            let sy = (y as u64 * *h as u64 / height as u64) as usize;
            let at = (sy * *w as usize + sx) * 4;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    out
}
fn image(pixels: &[u8], width: u32, height: u32) -> Result<Image> {
    let bytes = png(pixels, width, height).map_err(failed)?;
    Ok(Image {
        data: base64(&bytes),
        mime_type: "image/png".into(),
    })
}
fn sheet(cells: &[Rgba], plan: &SheetPlan) -> Result<Image> {
    let (cell_w, cell_h) = (plan.cell_width, plan.cell_height);
    let (width, height) = (cell_w * plan.columns as u32, cell_h * plan.rows as u32);
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    for (index, cell) in cells.iter().enumerate() {
        let (column, row) = (index % plan.columns, index / plan.columns);
        let fitted = fit(cell, cell_w, cell_h);
        for line in 0..cell_h as usize {
            let start =
                ((row * cell_h as usize + line) * width as usize + column * cell_w as usize) * 4;
            let source = line * cell_w as usize * 4;
            canvas[start..start + cell_w as usize * 4]
                .copy_from_slice(&fitted[source..source + cell_w as usize * 4]);
        }
    }
    image(&canvas, width, height)
}
fn source_frame(reader: &mut PreviewReader, time: f64) -> AnyResult<Rgba> {
    let frame = reader.frame_at(time)?;
    let decoded = frame.decoded();
    let (width, height) = (decoded.format.width as u32, decoded.format.height as u32);
    let unit = pm_media::image::imageio::rgba_unit(decoded)?;
    let pixels = unit
        .iter()
        .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
        .collect();
    Ok((pixels, width, height))
}
impl EngineJobs {
    fn original(&self, request: &MediaRequest) -> Result<PathBuf> {
        let asset = request
            .asset
            .as_ref()
            .and_then(|asset| asset["id"].as_str())
            .ok_or_else(|| Error::invalid("Supply assetId or layerId."))?;
        let published = self.published(&request.snapshot)?;
        crate::media::for_session(
            &published.snapshot,
            &published.media,
            published.path.as_deref(),
            &crate::media::cache_root(&self.profile),
        )
        .map_err(failed)?
        .catalog
        .originals
        .remove(asset)
        .ok_or_else(|| Error::invalid("This media's original is offline. Relink it first."))
    }
    /// Source times read as the asset's layers read them (its `timing`).
    fn reader(&self, request: &MediaRequest) -> Result<PreviewReader> {
        let timing = pm_media::clip::ClipTiming::of_asset(
            request
                .asset
                .as_ref()
                .and_then(|asset| asset["timing"].as_f64()),
        );
        Ok(PreviewReader::open(&self.original(request)?)
            .map_err(failed)?
            .with_timing(timing))
    }
}
fn seconds(micros: Option<i64>) -> Option<f64> {
    micros.map(|us| us as f64 / 1_000_000.)
}
fn decibels(level: f64) -> f64 {
    (20. * level.max(1e-9).log10() * 10.).round() / 10.
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_engine_service::resources::ResourceSnapshot;
    fn snapshot(generation: u64, pending: bool) -> Option<Resources> {
        Some(Arc::new(ResourceSnapshot {
            generation,
            pending,
            ..Default::default()
        }))
    }
    /// An export, poster or agent still of a document whose window is still
    /// loading extensions waits for them; an extension's own capture (its
    /// realm may be the one loading) takes what is registered.
    /// A capture taken after the wait while an extension still loads (one
    /// that hangs) is refused, never drawn without that extension's effect.
    #[test]
    fn a_capture_never_bypasses_an_effect_still_loading() {
        let loaded = ResourceSnapshot::default();
        let loading = ResourceSnapshot {
            pending: true,
            ..Default::default()
        };
        assert!(bypasses_missing(true, &loaded));
        assert!(!bypasses_missing(true, &loading));
        assert!(!bypasses_missing(false, &loaded));
    }
    /// A frame an extension captured (a GLSL-only effect: no override
    /// wrapper, so no epoch moves) is not the answer after that effect's
    /// extension reloaded with other code, at the same document revision:
    /// the kept frame is of the definitions it was drawn with, nothing is
    /// kept or served while the reload runs, and each render draws with the
    /// resources pinned for its lookup.
    #[test]
    fn a_kept_capture_is_never_served_after_its_effect_reloaded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
        let kernel = actor.handle();
        let project = json!({"id":"p","name":"Film","compId":"c","w":64,"h":36,"fps":25,"dur":1,
            "bg":"#000000","comps":{},"layers":[],"assets":{}});
        kernel
            .open(
                "capture-cache",
                pm_kernel::OpenSource::Untitled(
                    pm_model::json::parse(&project.to_string()).unwrap(),
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        let doc = kernel.published("capture-cache").unwrap().snapshot.doc_id;
        let renders = Arc::new(AtomicUsize::new(0));
        let count = renders.clone();
        // Stands in for the engine child: the "pixels" are the generation of
        // the definitions the frame was drawn with; a frame drawn while they
        // load is refused, as the engine refuses it.
        let capture = cached_frames(
            kernel.clone(),
            "capture-cache".into(),
            Capture::ExtensionSnapshot,
            |_| None,
            move |_, _, resources| {
                count.fetch_add(1, Ordering::Relaxed);
                let resources = resources.expect("the window's resources were pinned");
                if resources.pending {
                    return Err("still loading".into());
                }
                Ok(resources.generation.to_le_bytes().to_vec())
            },
        );
        let ask = || {
            capture(&pm_compat_runner::FrameRequest {
                time: 0.5,
                width: 64,
                height: 36,
                motion_blur: true,
                stop: &never,
                published: None,
            })
        };
        let drawn = |generation: u64| Ok(generation.to_le_bytes().to_vec());
        set_resources(doc, snapshot(1, false));
        assert_eq!(ask(), drawn(1));
        assert_eq!(ask(), drawn(1));
        assert_eq!(renders.load(Ordering::Relaxed), 1, "the same frame is kept");
        // The reload starts: the old frame is not the answer, and what is
        // drawn meanwhile is not kept.
        set_resources(doc, snapshot(2, true));
        assert_eq!(ask(), Err("still loading".into()));
        assert_eq!(ask(), Err("still loading".into()));
        assert_eq!(renders.load(Ordering::Relaxed), 3);
        // The reloaded code: drawn again with it, at the same revision.
        set_resources(doc, snapshot(3, false));
        assert_eq!(ask(), drawn(3));
        assert_eq!(ask(), drawn(3));
        assert_eq!(renders.load(Ordering::Relaxed), 4);
        set_resources(doc, None);
        actor.shutdown();
    }
    #[test]
    fn captures_wait_for_their_window_extensions_to_load() {
        let doc = 0x5e77_1ed0;
        set_resources(doc, snapshot(1, true));
        assert_eq!(resources_of(doc, false).map(|r| r.pending), Some(true));
        let waiter = std::thread::spawn(move || resources_of(doc, true).map(|r| r.generation));
        std::thread::sleep(Duration::from_millis(150));
        assert!(!waiter.is_finished(), "a capture went ahead while loading");
        set_resources(doc, snapshot(2, true));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!waiter.is_finished(), "still loading");
        set_resources(doc, snapshot(3, false));
        assert_eq!(waiter.join().unwrap(), Some(3));
        set_resources(doc, None);
        assert!(resources_of(doc, true).is_none());
    }
}

#[cfg(test)]
mod relink_tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::*};

    const RATE: usize = 48_000;
    const CLICKS: [usize; 3] = [4_800, 24_000, 14_400];

    fn ffmpeg_path() -> PathBuf {
        std::env::var_os("PM_FFMPEG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var("HOME").unwrap())
                    .join("Library/Caches/powermove-engine/ffmpeg/out")
            })
            .join("bin/ffmpeg")
    }

    /// A one-second clip with a picture and one mono PCM stream per click.
    fn clip(root: &Path, name: &str, clicks: &[usize]) -> PathBuf {
        let path = root.join(name);
        let mut command = std::process::Command::new(ffmpeg_path());
        command.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"]);
        command.args(["-i", "testsrc2=size=64x64:rate=25:duration=1"]);
        for at in clicks {
            command
                .args(["-f", "lavfi", "-i"])
                .arg(format!("aevalsrc='0.9*eq(n\\,{at})':s={RATE}:d=1"));
        }
        command.args(["-map", "0:v"]);
        for k in 0..clicks.len() {
            command.args(["-map", &format!("{}:a", k + 1)]);
        }
        command.args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"]);
        assert!(command.arg(&path).status().unwrap().success());
        path
    }

    fn root_of(kernel: &KernelHandle) -> (Arc<Published>, Value) {
        let published = kernel.published("p").unwrap();
        let root = serde_json::from_str(&to_canonical_string(
            &published.snapshot.root().unwrap().to_value(),
        ))
        .unwrap();
        (published, root)
    }

    /// The clicks heard from `asset` as the only clip, through the mix graph.
    fn heard(kernel: &KernelHandle, asset: &str) -> Vec<usize> {
        let (_, mut root) = root_of(kernel);
        root["layers"] = json!([{"id":"V","type":"video","name":"V","from":0,"dur":1,"on":true,
            "solo":false,"group":null,"parent":null,"p":{},"fx":[],"masks":[],
            "d":{"asset":asset,"trim":0,"speed":1,"embeddedAudio":true}}]);
        let source = PathBuf::from(root["assets"][asset]["sourcePath"].as_str().unwrap());
        let doc =
            pm_model::Document::new(Some(pm_model::json::parse(&root.to_string()).unwrap()), 1);
        let cache = pm_audio::SourceCache::new(64 << 20, 0);
        let resolver = move |_: &str| Some(source.clone());
        let graph =
            pm_audio::MixGraph::build(&doc, "c", &mut pm_eval::Evaluator::new(), &cache, &resolver)
                .unwrap();
        let mut mixer = pm_audio::Mixer::new(
            pm_audio::MixFormat::stereo(RATE as u32),
            Arc::new(doc),
            Arc::new(graph),
            pm_eval::Evaluator::new(),
            cache.clone(),
        );
        let mut left = vec![];
        let summary = pm_audio::offline::render(&mut mixer, 0, RATE as i64, 4096, |_, inter| {
            left.extend(inter.iter().step_by(2).copied());
            Ok(())
        })
        .unwrap();
        cache.shutdown();
        assert!(summary.errors.is_empty(), "{:?}", summary.errors);
        CLICKS
            .iter()
            .enumerate()
            .filter(|(_, at)| left[*at - 2..*at + 3].iter().any(|v| v.abs() > 0.2))
            .map(|(k, _)| k)
            .collect()
    }

    /// An agent's relink keeps which audio streams are heard, exactly as a
    /// person's does (`pm_builtins` tests the full table).
    #[test]
    fn an_agent_relink_keeps_which_audio_streams_are_heard() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(root.join("journals")),
            ..Default::default()
        });
        let kernel = actor.handle();
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: root.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let entry =
            |index: u32, on: bool| json!({"index":index,"channels":1,"sampleRate":48000,"on":on});
        let project = json!({"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":25,"dur":1,
            "bg":"#000000","comps":{},"layers":[],"assets":{
                "older":{"id":"older","name":"Older.mov","kind":"video","dur":1},
                "muted":{"id":"muted","name":"Muted.mov","kind":"video","dur":1,
                    "audioStreams":[entry(1, true), entry(2, false)]},
                "first_off":{"id":"first_off","name":"First off.mov","kind":"video","dur":1,
                    "audioStreams":[entry(1, false), entry(2, true)]},
                "late":{"id":"late","name":"Late.mp4","kind":"video","dur":1}}});
        kernel
            .shell(ShellCall::Open {
                id: "p".into(),
                source: SessionSource::New(pm_model::json::parse(&project.to_string()).unwrap()),
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let jobs = EngineJobs::new(
            kernel.clone(),
            PathBuf::new(),
            Default::default(),
            root.clone(),
            OverrideWindows::default(),
        );
        let ffmpeg = ffmpeg_path();
        let recorder = clip(&root, "recorder.mov", &CLICKS[..2]);
        let mono = clip(&root, "mono.mov", &CLICKS[2..]);
        let relink = |asset: &str, path: &Path| {
            let (published, project) = root_of(&kernel);
            let replaced = jobs
                .import(&MediaImport {
                    frozen: FrozenProject {
                        project_id: "p".into(),
                        document_id: published.snapshot.doc_id,
                        revision: published.revision(),
                        project,
                    },
                    path: path.to_owned(),
                    folder: None,
                    replace: Some(asset.into()),
                    expected_fingerprint: None,
                    writer: 1,
                })
                .unwrap();
            assert_eq!(replaced.status, "replaced");
        };
        relink("older", &recorder);
        assert_eq!(heard(&kernel, "older"), [0]);
        relink("muted", &recorder);
        assert_eq!(heard(&kernel, "muted"), [0]);
        relink("first_off", &mono);
        assert_eq!(heard(&kernel, "first_off"), Vec::<usize>::new());
        assert_eq!(
            root_of(&kernel).1["assets"]["first_off"]["audioStreams"],
            json!([entry(1, false)])
        );
        // An older asset measures its clip as it reads it: twenty frames
        // 0.4 s into the file end 1.2 s in.
        let late = root.join("late.mp4");
        let status = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args([
                "-i",
                "testsrc2=size=64x64:rate=25:duration=0.8",
                "-c:v",
                "mpeg4",
            ])
            .args(["-output_ts_offset", "0.4"])
            .arg(&late)
            .status()
            .unwrap();
        assert!(status.success());
        relink("late", &late);
        let dur = root_of(&kernel).1["assets"]["late"]["dur"]
            .as_f64()
            .unwrap();
        assert!((dur - 1.2).abs() < 1e-3, "{dur}");
    }
}
