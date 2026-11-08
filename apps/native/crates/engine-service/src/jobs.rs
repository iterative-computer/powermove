//! Same-executable --engine-job child. Payloads stay on inherited pipes.
//! JobSpec, ExportSettings and JobStatus are the existing engine protocol.
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
}
/// An export frame's raster override answer from the window that has the
/// document open: `None` passes the layer through; `Some` is premultiplied RGBA8
/// with the quad's size and anchor (layer pixels).
pub type CompatReply = Option<(u32, u32, Vec<u8>, [f64; 2], [f64; 2])>;
/// One layer an export frame needs an override answer for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompatJobNeed {
    pub composition: String,
    pub layer: String,
    pub seconds: f64,
    pub scale: f64,
}
/// Answers a batch of export needs, in order, on the job's reader thread.
pub type CompatAnswerer =
    Arc<dyn Fn(&[CompatJobNeed]) -> std::result::Result<Vec<CompatReply>, String> + Send + Sync>;
struct CompatProducer {
    producing: Arc<dyn Fn() -> bool + Send + Sync>,
    answer: CompatAnswerer,
}
/// Each open document's producer, by kernel `doc_id` (the job's session).
static COMPAT: Mutex<Option<std::collections::HashMap<u64, CompatProducer>>> = Mutex::new(None);
/// Whose extensions wrap `raster` for document `doc_id` (none: its exports
/// draw natively).
pub fn set_compat_answerer(
    doc_id: u64,
    producer: Option<(Arc<dyn Fn() -> bool + Send + Sync>, CompatAnswerer)>,
) {
    let mut all = COMPAT.lock().unwrap();
    let all = all.get_or_insert_with(Default::default);
    match producer {
        Some((producing, answer)) => {
            all.insert(doc_id, CompatProducer { producing, answer });
        }
        None => {
            all.remove(&doc_id);
        }
    }
}
/// A compat need's reply on the child's stdin. Pixels go through a private
/// file the child reads and removes, never through the pipe.
fn compat_reply(id: u64, answered: std::result::Result<Vec<CompatReply>, String>) -> String {
    let answer = |(i, reply): (usize, CompatReply)| match reply {
        None => serde_json::json!({"native": true}),
        Some((width, height, rgba, size, anchor)) => {
            let path = std::env::temp_dir().join(format!(
                "powermove-compat-{}-{id}-{i}.rgba",
                std::process::id()
            ));
            match std::fs::write(&path, &rgba) {
                Ok(()) => serde_json::json!({"width": width, "height": height,
                    "size": size, "anchor": anchor, "file": path}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
    };
    let body = match answered {
        Ok(replies) => serde_json::json!({"id": id,
            "answers": replies.into_iter().enumerate().map(answer).collect::<Vec<_>>()}),
        Err(error) => serde_json::json!({"id": id, "error": error}),
    };
    format!("{}\n", serde_json::json!({ "compatAnswer": body }))
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
const LOW_DISK: &str = "The export stopped because the destination disk is almost full. Free some space and export again.";
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
pub struct ExportHandle {
    child: Option<Child>,
    input: Arc<Mutex<ChildStdin>>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<JobStatus>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl ExportHandle {
    /// Call from a job worker: process spawn and snapshot serialization are
    /// deliberately excluded from the UI event path.
    pub fn start(executable: &Path, request: ExportRequest) -> Result<Self> {
        Self::start_scheduled(executable, request, Arc::default())
    }
    /// Interactive service activity takes precedence at every export band.
    /// One already admitted band cannot be preempted by Metal.
    pub fn start_scheduled(
        executable: &Path,
        mut request: ExportRequest,
        interactive: Arc<AtomicBool>,
    ) -> Result<Self> {
        // An export draws what the viewer draws: a raster override's layers too.
        let session = match &request.spec {
            JobSpec::Export { session, .. } => Some(session.get()),
            _ => None,
        };
        let answerer = COMPAT.lock().unwrap().as_ref().and_then(|all| {
            let c = all.get(&session?)?;
            (c.producing)().then(|| c.answer.clone())
        });
        if answerer.is_some()
            && let JobSpec::Export { settings, .. } = &mut request.spec
            && !settings
                .delivery
                .compat
                .contains(&pm_protocol::snapshot::CompatSource::RasterOverride)
        {
            settings
                .delivery
                .compat
                .push(pm_protocol::snapshot::CompatSource::RasterOverride);
        }
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
        if let Err(e) = serde_json::to_writer(&mut input, &request)
            .and_then(|_| input.write_all(b"\n").map_err(serde_json::Error::io))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e.into());
        }
        input.flush()?;
        let input = Arc::new(Mutex::new(input));
        let gate_input = input.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let gate_cancel = cancel.clone();
        let output = child.stdout.take().unwrap();
        let state = Arc::new(Mutex::new(status(
            &request.job,
            JobState::Queued,
            0.,
            None,
            None,
        )));
        let events = state.clone();
        let id = request.job;
        let compat_input = input.clone();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if let Ok(text) = line.as_deref()
                    && text.starts_with("{\"compat\"")
                {
                    let asked: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
                    let id = asked["compat"]["id"].as_u64().unwrap_or(0);
                    let needs: Vec<CompatJobNeed> =
                        serde_json::from_value(asked["compat"]["needs"].clone())
                            .unwrap_or_default();
                    let answered = match &answerer {
                        Some(answer) => answer(&needs),
                        None => Err("No window has this project's extensions open".into()),
                    };
                    let mut input = compat_input.lock().unwrap();
                    if input
                        .write_all(compat_reply(id, answered).as_bytes())
                        .and_then(|_| input.flush())
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                if matches!(line.as_deref(), Ok("\"gpu\"")) {
                    while interactive.load(Ordering::Acquire)
                        && !gate_cancel.load(Ordering::Acquire)
                    {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    if !gate_cancel.load(Ordering::Acquire) {
                        let mut input = gate_input.lock().unwrap();
                        if input
                            .write_all(b"grant\n")
                            .and_then(|_| input.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                    continue;
                }
                match line
                    .ok()
                    .and_then(|s| serde_json::from_str::<JobStatus>(&s).ok())
                {
                    Some(s) if s.job == id => *events.lock().unwrap() = s,
                    _ => break,
                }
            }
            let mut last = events.lock().unwrap();
            if !last.state.is_terminal() {
                *last = status(
                    &id,
                    JobState::Failed,
                    last.progress,
                    None,
                    Some("engine job child exited without a terminal status".into()),
                );
            }
        });
        Ok(Self {
            child: Some(child),
            input,
            cancel,
            status: state,
            reader: Some(reader),
        })
    }
    pub fn status(&self) -> JobStatus {
        self.status.lock().unwrap().clone()
    }
    pub fn cancel(&mut self) -> Result<()> {
        self.cancel.store(true, Ordering::Release);
        let mut input = self.input.lock().unwrap();
        input.write_all(b"cancel\n")?;
        input.flush()?;
        Ok(())
    }
    /// Worker-only wait. Drop also reaps the child (never a detached export).
    pub fn wait(mut self) -> Result<JobStatus> {
        if let Some(mut child) = self.child.take() {
            child.wait()?;
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        Ok(self.status())
    }
}
impl Drop for ExportHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
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
    let request: ExportRequest = serde_json::from_str(
        &lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing export request"))??,
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    request.resources.validate()?;
    let (grant, permits) = std::sync::mpsc::channel();
    let stopped = cancel.clone();
    let rasters = Arc::new(JobRasters::default());
    let replies = rasters.clone();
    std::thread::spawn(move || {
        for line in lines {
            if let Ok(text) = line.as_deref()
                && text.starts_with("{\"compatAnswer\"")
            {
                replies.reply(text);
                continue;
            }
            if matches!(line.as_deref(), Ok("grant")) {
                let _ = grant.send(());
                continue;
            }
            if matches!(line.as_deref(), Ok("cancel")) {
                stopped.store(true, Ordering::Release);
                return;
            }
        }
        stopped.store(true, Ordering::Release); // Parent died/closed the pipe.
    });
    let emit = |s: JobStatus| -> Result<()> {
        let mut out = std::io::stdout().lock();
        serde_json::to_writer(&mut out, &s)?;
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(())
    };
    emit(status(&request.job, JobState::Running, 0., None, None))?;
    // CoreText registry stays on main; rendering waits only on this serviced
    // broker, never on GPUI or a Node event loop.
    let mut font_service = pm_node::fonts::FontService::new().map_err(anyhow::Error::msg)?;
    let broker = pm_node::fonts::render::FontBroker::new();
    // This windowless host pumps the broker below. A registered wake admits
    // requests; without it layout fails immediately and can lose text glyphs.
    broker.set_wake(Arc::new(|| {}));
    broker.set_generation(font_service.generation());
    let fonts = pm_node::fonts::render::RenderFonts::new(broker.clone());
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
        let gate = ChildGate(Mutex::new(permits));
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
        };
        let media = catalog.export_paths(settings.draft_from_proxies);
        let mut doc = pm_model::Document::new(
            Some(pm_model::json::parse(&request.document)?),
            request.revision,
        );
        doc.set_saving_app_version(request.saving_app_version);
        ensure!(
            pm_export::render::offline_media(&doc, composition.as_str(), &media).is_empty(),
            "export originals are offline; relink originals or explicitly choose draft"
        );
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!("cancelled");
        }
        let mut renderer = pm_render::Renderer::headless()?;
        if overriding {
            renderer.set_compat_rasters(Some(rasters.clone()));
        }
        // An export never ships silently without an effect the project uses.
        let failures = renderer
            .set_document_effects(&doc, &request.resources.definitions)
            .map_err(anyhow::Error::msg)?;
        if let Some(failure) = failures.iter().find(|failure| {
            matches!(&failure.scope, pm_protocol::error::ErrorScope::Definition { definition }
                if request.document.contains(&format!("\"{}\"", definition.as_str())))
        }) {
            anyhow::bail!("{}", failure.message);
        }
        #[cfg(feature = "kernel")]
        crate::studio::attach(&mut renderer, Arc::new(doc.clone()), &media);
        fonts.sync(&mut renderer, &mut None);
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
        pm_node::fonts::render::serve(&mut font_service, broker.take_asks());
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
        Err(_) if cancel.load(Ordering::Acquire) => {
            emit(status(&job_id, JobState::Cancelled, 0., None, None))?
        }
        Err(e) => emit(status(
            &job_id,
            JobState::Failed,
            0.,
            None,
            Some(e.to_string()),
        ))?,
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
    next: std::sync::atomic::AtomicU64,
    waiting: Mutex<std::collections::HashMap<u64, std::sync::mpsc::Sender<serde_json::Value>>>,
    answered: Mutex<JobAnswers>,
}
type JobKey = (String, String, u64, u64);
#[derive(Default)]
struct JobAnswers {
    map: std::collections::HashMap<JobKey, pm_render::compat::CompatAnswer>,
    time: u64,
}
impl JobRasters {
    fn reply(&self, text: &str) {
        let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let body = value["compatAnswer"].clone();
        if let Some(id) = body["id"].as_u64()
            && let Some(tx) = self.waiting.lock().unwrap().remove(&id)
        {
            let _ = tx.send(body);
        }
    }
    fn raster(item: &serde_json::Value) -> pm_render::compat::CompatAnswer {
        use pm_render::compat::CompatAnswer;
        if item["native"] == true {
            return CompatAnswer::Native;
        }
        let Some(file) = item["file"].as_str() else {
            return CompatAnswer::Missing;
        };
        let data = std::fs::read(file);
        let _ = std::fs::remove_file(file);
        let number = |v: &serde_json::Value, i: usize| v[i].as_f64().unwrap_or(0.);
        match data {
            Ok(data) => crate::compat::override_raster(
                item["width"].as_u64().unwrap_or(0) as u32,
                item["height"].as_u64().unwrap_or(0) as u32,
                data,
                [number(&item["size"], 0), number(&item["size"], 1)],
                [number(&item["anchor"], 0), number(&item["anchor"], 1)],
            ),
            Err(_) => CompatAnswer::Missing,
        }
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
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        let line = serde_json::json!({"compat": {"id": id, "needs": needs}});
        {
            let mut out = std::io::stdout().lock();
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                return CompatAnswer::Missing;
            }
        }
        let Ok(body) = rx.recv_timeout(std::time::Duration::from_secs(120)) else {
            return CompatAnswer::Missing;
        };
        let items = body["answers"].as_array().cloned().unwrap_or_default();
        let mut answered = self.answered.lock().unwrap();
        let mut mine = CompatAnswer::Missing;
        for (need, item) in needs.iter().zip(&items) {
            let answer = Self::raster(item);
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
/// Inherited-pipe admission. Exactly one renderer band per grant; never a
/// whole-frame burst. Cancellation and parent EOF interrupt a deferred export.
struct ChildGate(Mutex<std::sync::mpsc::Receiver<()>>);
impl pm_export::render::GpuGate for ChildGate {
    fn admit(&self, _: u64, _: u32, cancel: &AtomicBool) -> Option<u32> {
        {
            let mut out = std::io::stdout().lock();
            if out
                .write_all(b"\"gpu\"\n")
                .and_then(|_| out.flush())
                .is_err()
            {
                return None;
            }
        }
        let permits = self.0.lock().unwrap();
        while !cancel.load(Ordering::Acquire) {
            match permits.recv_timeout(std::time::Duration::from_millis(5)) {
                Ok(()) => return Some(1),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return None,
            }
        }
        None
    }
    fn finished(&self, _: u64, _: u32, _: f64) {}
    fn abandoned(&self, _: u64) {}
}
#[cfg(test)]
mod tests {
    use super::*;
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
}
