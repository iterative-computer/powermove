//! Production hosts behind MCP's service traits. Everything here runs on MCP or
//! export workers. Exports and frames use the same child job as the Export panel.
use crate::mcp::Tools;
use anyhow::{Context, Result as AnyResult};
use pm_engine_service::jobs::{ExportHandle, ExportRequest};
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
/// Document `doc_id`'s definitions now (none: its window has no extensions).
pub(crate) fn set_resources(doc_id: u64, resources: Option<Resources>) {
    let mut all = RESOURCES.lock().unwrap();
    let all = all.get_or_insert_with(HashMap::new);
    match resources {
        Some(resources) => all.insert(doc_id, resources),
        None => all.remove(&doc_id),
    };
}
fn resources_of(doc_id: u64) -> Option<Resources> {
    RESOURCES.lock().unwrap().as_ref()?.get(&doc_id).cloned()
}
/// Install the production engine hosts. Call on a worker before serving.
pub fn connect_engine(tools: &Tools, activity: Arc<AtomicBool>) -> AnyResult<()> {
    let jobs = EngineJobs::new(
        tools.host.kernel.clone(),
        std::env::current_exe()?,
        activity,
        tools.host.profile.clone(),
    );
    let mut automation = tools.automation.lock().unwrap();
    automation.export_child = jobs.clone();
    automation.media_importer = Some(jobs.clone());
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
        // Transport errors can carry hosts and system detail: none crosses MCP.
        let response = pm_compat_runner::net::get(url.as_str(), request.max_bytes, 30)
            .map_err(|_| failed("The Store didn't answer. Check the connection and try again."))?;
        // A redirect may not take a Store read to another origin.
        if url::Url::parse(&response.url).ok().map(|u| u.origin()) != Some(origin.origin()) {
            return Err(failed("The Store answered from another origin"));
        }
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
            ..Default::default()
        });
        let (host, _) = crate::mcp::Host::new(actor.handle(), profile.clone());
        crate::mcp::attach(&host.kernel, &profile)?;
        if let Some(path) = launch {
            host.open(
                crate::session::stable_id(&path),
                pm_kernel::session::SessionSource::File(path),
            )?;
        }
        let tools = Tools::new(host, exports);
        connect_engine(&tools, Arc::default())?;
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
    activity: Arc<AtomicBool>,
    profile: PathBuf,
    cache: PathBuf,
    progress: Mutex<HashMap<u64, Progress>>,
}
impl EngineJobs {
    pub fn new(
        kernel: KernelHandle,
        executable: PathBuf,
        activity: Arc<AtomicBool>,
        profile: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            kernel,
            executable,
            activity,
            cache: profile.join("cache/mcp"),
            profile,
            progress: Mutex::default(),
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
        published: &Published,
        settings: ExportSettings,
        destination: PathBuf,
        look: bool,
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
        // An agent's capture draws the effects its project's window defines.
        if let Some(resources) = resources_of(published.snapshot.doc_id) {
            request.resources = (*resources).clone();
        }
        // A capture shows the project as the viewer does; a video is a delivery.
        request.bypass_missing_effects = look;
        // As the shipped app's agent exports: silent offline clips are left
        // out, not asked about; missing audio still fails by name.
        request.without_offline = true;
        ExportHandle::start_scheduled(&self.executable, request, self.activity.clone())
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
        let handle = self
            .start(&published, settings, destination, false)
            .map_err(failed)?;
        let started = Instant::now();
        while !matches!(
            handle.status().state,
            JobState::Done | JobState::Failed | JobState::Cancelled
        ) {
            self.progress.lock().unwrap().insert(
                frozen.document_id,
                Progress {
                    frames: (handle.status().progress * plan.frames as f64).round() as u64,
                    total: Some(plan.frames),
                    encoder: None,
                    seconds: started.elapsed().as_secs_f64(),
                },
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        self.progress.lock().unwrap().remove(&frozen.document_id);
        finish(handle)
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
) -> AnyResult<pm_compat_runner::FrameRenderer> {
    let jobs = EngineJobs::new(
        kernel.clone(),
        std::env::current_exe()?,
        Arc::new(AtomicBool::new(false)),
        profile,
    );
    // The last few frames by document revision: a panel that asks for the same
    // frame again (thumbnails, a check on every redraw) gets it without a render.
    let cache: Mutex<std::collections::VecDeque<(String, Arc<Vec<u8>>)>> = Mutex::default();
    Ok(Arc::new(move |request: &pm_compat_runner::FrameRequest| {
        let published = kernel.published(&project).ok_or("No open project")?;
        // The same frame drawn by other override wrappers is another frame.
        let key = format!(
            "{}:{}:{:?}:{}:{}x{}:{}",
            published.snapshot.doc_id,
            published.revision(),
            pm_engine_service::jobs::compat_epoch(published.snapshot.doc_id),
            request.time.to_bits(),
            request.width,
            request.height,
            request.motion_blur
        );
        if let Some((_, png)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
            return Ok(png.as_ref().clone());
        }
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
        let png = frame(
            &jobs,
            &published,
            fps,
            request.time,
            Size::new(request.width, request.height),
            look,
            request.stop,
        )
        .map_err(|e| e.to_string())?;
        let mut cache = cache.lock().unwrap();
        if cache.len() >= 4 {
            cache.pop_front();
        }
        cache.push_back((key, Arc::new(png.clone())));
        Ok(png)
    }))
}
/// How a captured frame looks: MCP captures keep alpha; an extension's
/// `renderFrameTo` is opaque over black with motion blur, as main draws it.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    alpha: AlphaOutput,
    motion_blur: bool,
}
const MCP_LOOK: Look = Look {
    alpha: AlphaOutput::Straight,
    motion_blur: false,
};
fn never() -> bool {
    false
}
/// Renders one frame; `stop` cancels the export (a stopped task, a deadline).
fn frame(
    jobs: &EngineJobs,
    published: &Published,
    fps: f64,
    time: f64,
    size: Size,
    look: Look,
    stop: &(dyn Fn() -> bool + Sync),
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
    let mut handle = jobs.start(published, settings, destination, true)?;
    while matches!(
        handle.status().state,
        JobState::Queued | JobState::Running | JobState::WaitingForMedia
    ) {
        if stop() {
            let _ = handle.cancel();
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
        let duration = probe
            .as_ref()
            .and_then(|probe| seconds(probe.duration_us))
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
        let metadata = json!({"name":name,"kind":kind,"mimeType":mime,"fingerprint":fingerprint,
            "sourcePath":path,"size":size,"w":width.unwrap_or(0.),"h":height.unwrap_or(0.),
            "dur":duration.unwrap_or(0.),"hasAudio":has_audio,"persisted":true});
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
                frame(self, &published, fps, *time, size, MCP_LOOK, &never).map_err(failed)?;
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
            let bytes = frame(self, &published, fps, cell.source, size, MCP_LOOK, &never)
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
    fn waveform(&self, request: &WaveformRequest) -> Result<Waveform> {
        const BUCKETS_PER_SECOND: f64 = 100.;
        let path = self.original(&request.request)?;
        let rate = pm_media::probe::probe_timing(&path)
            .map_err(failed)?
            .streams
            .iter()
            .find_map(|stream| stream.audio.as_ref().map(|audio| audio.sample_rate))
            .ok_or_else(|| Error::invalid("This media has no audio."))?;
        let per_peak = (rate as f64 / BUCKETS_PER_SECOND).round() as usize;
        let peaks =
            pm_media::probe::peaks::peaks(&path, None, per_peak.max(1), 0).map_err(failed)?;
        let channels = peaks.channels.max(1);
        let envelope = peaks
            .data
            .chunks(channels)
            .map(|bucket| {
                bucket
                    .iter()
                    .map(|[low, high]| low.abs().max(high.abs()) as f64)
                    .fold(0., f64::max)
            })
            .collect::<Vec<f64>>();
        let bucket_at = |time: f64| {
            ((time * peaks.sample_rate as f64 / per_peak as f64) as usize).min(envelope.len())
        };
        let span = &envelope[bucket_at(request.start)..bucket_at(request.end)];
        let peak = span.iter().copied().fold(0., f64::max);
        let threshold = 10f64.powf(request.threshold_db / 20.);
        let mut silences = vec![];
        let mut quiet = None;
        for (index, level) in span.iter().chain([&f64::MAX]).enumerate() {
            let time = request.start + index as f64 / BUCKETS_PER_SECOND;
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
    fn reader(&self, request: &MediaRequest) -> Result<PreviewReader> {
        PreviewReader::open(&self.original(request)?).map_err(failed)
    }
}
fn seconds(micros: Option<i64>) -> Option<f64> {
    micros.map(|us| us as f64 / 1_000_000.)
}
fn decibels(level: f64) -> f64 {
    (20. * level.max(1e-9).log10() * 10.).round() / 10.
}
