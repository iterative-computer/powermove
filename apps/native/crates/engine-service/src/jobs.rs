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
    pub resources: crate::resources::ResourceSnapshot,
}
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
            resources: Default::default(),
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
        request: ExportRequest,
        interactive: Arc<AtomicBool>,
    ) -> Result<Self> {
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
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
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
    std::thread::spawn(move || {
        for line in lines {
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
            settings.delivery.compat.is_empty(),
            "compat export requires the P2 runtime host"
        );
        let catalog = crate::MediaCatalog {
            originals: request.originals,
            proxies: request.proxies,
        };
        let media = catalog.export_paths(settings.draft_from_proxies);
        let doc = pm_model::Document::new(
            Some(pm_model::json::parse(&request.document)?),
            request.revision,
        );
        ensure!(
            pm_export::render::offline_media(&doc, composition.as_str(), &media).is_empty(),
            "export originals are offline; relink originals or explicitly choose draft"
        );
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!("cancelled");
        }
        let mut renderer = pm_render::Renderer::headless()?;
        renderer.set_definitions(&pm_render::all_builtin_definitions())?;
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
    match result {
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
