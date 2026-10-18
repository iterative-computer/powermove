//! The engines' Blender backend (contract 11 over the host port; contract
//! 14's Blender supervision runs in host services): the job engine's
//! exports, and the interactive engine's paused viewer
//! (`crate::present::blender`).
//!
//! For each Blender frame the stage needs, the backend writes the
//! snapshot's canonical JSON into the Blender Surfaces cache area
//! (`snapshots/<blake3>.json`) and any converted texture into
//! `assets/<sha256>.<ext>`, names every other asset by a media handle, and
//! sends `blenderRender` with one surface request per layer. Host services
//! run the pinned worker, write each surface as `<key>.png` in the same
//! area, send one `blenderSurface` event per surface (and progress), and
//! answer the request once the job ends: `ack`, or the error that ended it
//! (`blenderFailed`, `blenderMissing`, `cancelled`). Cancelling the export
//! sends `cancelJobs` for the Blender job, which stops Blender.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pm_blender::BlenderError;
use pm_blender::job::{BlenderBackend, RenderJob, RenderedFrame, RenderedSurface, SurfacePng};
use pm_blender::key::hex;
use pm_blender::snapshot::AssetBytes;
use pm_protocol::blender::{
    BlenderAsset, BlenderJobRequest, BlenderSurface, ShutterSample, SnapshotEncoding, SnapshotRef,
    SurfaceRequest, limits,
};
use pm_protocol::common::{ByteSize, ContentHash, JobId, SessionId, Size, Ticks};
use pm_engine::jobs::HostRequest;
use pm_protocol::envelope::{Event, Request, Response};
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope};
use pm_protocol::host::{
    CacheKind, CapabilityHandle, CapabilityKind, HandleAccess, MintHandle, ResolveHandle,
};
use pm_protocol::priority::{CancelJobs, PriorityClass};

/// How long the job engine waits for one Blender frame. Host services own
/// the 10-minute run timeout, which does not count the time Blender is
/// paused during playback (contract 13), and answer every request; this
/// bound only covers a host that never answers.
const FRAME_TIMEOUT: Duration = Duration::from_secs(6 * 3600);
/// Inputs older than the longest a job may wait and run are no job's
/// (`prune`).
const INPUT_HORIZON: Duration = FRAME_TIMEOUT;

/// How long an ordinary host call (handles, status) may take.
const HOST_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The host port as a Blender backend uses it: calls made from a thread
/// off the engine's, answered by the engine's host connection, and the
/// `blenderSurface` events of the Blender jobs it waits for.
pub(crate) trait HostPort: Send + Sync {
    fn call(&self, request: Request, session: Option<SessionId>) -> Result<Response, EngineError>;
    /// One call answered by `deadline`, or `None` once `cancel` is set.
    fn call_until(
        &self,
        request: Request,
        session: Option<SessionId>,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Option<Result<Response, EngineError>>;
    /// Surfaces of `job` are wanted from now on (before it is sent).
    fn begin_blender_job(&self, job: &JobId);
    /// The surfaces delivered for `job`; later ones are dropped.
    fn take_blender_surfaces(&self, job: &JobId) -> Vec<BlenderSurface>;
}

/// The Blender backend of one job lane, or of the paused viewer.
pub(crate) struct HostBackend<P: HostPort + ?Sized> {
    pub port: Arc<P>,
    /// The work this backend renders for now (set per job).
    pub job: Arc<Mutex<JobParams>>,
    area: Option<(PathBuf, CapabilityHandle, SessionId)>,
    next: u64,
}

/// What a Blender backend works for.
#[derive(Debug, Clone)]
pub(crate) struct JobParams {
    /// The session (handles are bound to it) and class.
    pub session: SessionId,
    pub priority: PriorityClass,
    /// The export job (or the viewer's preview), to name Blender jobs
    /// after it.
    pub export: JobId,
}

impl<P: HostPort + ?Sized> HostBackend<P> {
    pub fn new(port: Arc<P>, job: Arc<Mutex<JobParams>>) -> HostBackend<P> {
        HostBackend {
            port,
            job,
            area: None,
            next: 0,
        }
    }

    fn params(&self) -> JobParams {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The Blender Surfaces cache area and a handle to it for this session.
    pub fn area(&mut self) -> Result<(PathBuf, CapabilityHandle), BlenderError> {
        let session = self.params().session;
        if let Some((path, handle, s)) = &self.area
            && *s == session
        {
            return Ok((path.clone(), handle.clone()));
        }
        let failed = |e: EngineError| BlenderError::Failed(format!("Blender cache: {}", e.message));
        let grant = match self.port.call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::Cache {
                    cache: CacheKind::BlenderSurfaces,
                },
                session,
            }),
            Some(session),
        ) {
            Ok(Response::Handle(g)) => g,
            Ok(_) => {
                return Err(BlenderError::Failed(
                    "Blender cache: unexpected answer".into(),
                ));
            }
            Err(e) => return Err(failed(e)),
        };
        let resolved = match self.port.call(
            Request::ResolveHandle(ResolveHandle {
                handle: grant.handle.clone(),
                session,
            }),
            Some(session),
        ) {
            Ok(Response::HandleResolved(r)) if r.access == HandleAccess::ReadWrite => r,
            Ok(_) => return Err(BlenderError::Failed("Blender cache: not writable".into())),
            Err(e) => return Err(failed(e)),
        };
        let path = PathBuf::from(resolved.path);
        for sub in ["snapshots", "assets", "surfaces"] {
            pm_blender::supervisor::create_private_dir(&path.join(sub))
                .map_err(|e| BlenderError::Failed(format!("Blender cache: {e}")))?;
        }
        prune(&path);
        self.area = Some((path.clone(), grant.handle.clone(), session));
        Ok((path, grant.handle))
    }

    fn media_handle(&self, asset: &str) -> Result<CapabilityHandle, BlenderError> {
        let session = self.params().session;
        match self.port.call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::MediaRead {
                    asset: asset.into(),
                },
                session,
            }),
            Some(session),
        ) {
            Ok(Response::Handle(g)) => Ok(g.handle),
            Ok(_) => Err(BlenderError::Failed(format!("Missing media for {asset}"))),
            Err(e) => Err(BlenderError::Failed(e.message)),
        }
    }
}

/// Snapshots and converted textures are inputs of running jobs only:
/// those older than the longest a job may run are removed (surfaces stay,
/// under the surface cache's own bound).
fn prune(area: &std::path::Path) {
    let horizon = INPUT_HORIZON;
    for sub in ["snapshots", "assets", ""] {
        let Ok(entries) = std::fs::read_dir(area.join(sub)) else {
            continue;
        };
        // In the area itself, only surfaces host services wrote and no job
        // took (a failed or cancelled one).
        let only_png = sub.is_empty();
        for e in entries.flatten() {
            if only_png && e.path().extension().is_none_or(|x| x != "png") {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > horizon);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn of_engine(e: EngineError) -> BlenderError {
    match e.code {
        ErrorCode::BlenderMissing => BlenderError::Missing(e.message),
        ErrorCode::Cancelled => BlenderError::Cancelled,
        _ => BlenderError::Failed(e.message),
    }
}

impl<P: HostPort + ?Sized> BlenderBackend for HostBackend<P> {
    fn version(&mut self) -> Result<String, BlenderError> {
        let session = self.params().session;
        match self.port.call(Request::BlenderStatus, Some(session)) {
            Ok(Response::BlenderStatus(s)) if s.supported() => Ok(s.version.unwrap_or_default()),
            Ok(Response::BlenderStatus(s)) => Err(BlenderError::Missing(
                s.error
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| pm_blender::BLENDER_MISSING_MESSAGE.to_owned()),
            )),
            Ok(_) => Err(BlenderError::Failed(
                "unexpected Blender status answer".into(),
            )),
            Err(e) => Err(of_engine(e)),
        }
    }

    fn render(
        &mut self,
        job: &RenderJob<'_>,
        cancel: &AtomicBool,
    ) -> Result<RenderedFrame, BlenderError> {
        let (area, handle) = self.area()?;
        let params = self.params();
        let io = |e: std::io::Error| BlenderError::Failed(format!("Blender cache: {e}"));
        // The snapshot, addressed by its hash.
        let bytes = job.snapshot.encode();
        let hash = job.snapshot.hash();
        let file = area.join("snapshots").join(format!("{}.json", hex(&hash)));
        // Written again even when present: a fresh modification time keeps
        // it out of `prune` while the job runs.
        let tmp = file.with_extension(format!("{}.{}.part", std::process::id(), self.next));
        pm_blender::supervisor::write_private(&tmp, &bytes).map_err(io)?;
        std::fs::rename(&tmp, &file).map_err(io)?;
        // Assets: media handles, or converted bytes in the area.
        let mut assets = Vec::new();
        for a in &job.snapshot.assets {
            let handle = match &a.bytes {
                AssetBytes::File(_) => self.media_handle(&a.id)?,
                AssetBytes::Bytes(b) => {
                    let ext = std::path::Path::new(&a.name)
                        .extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "png".into());
                    let path = area
                        .join("assets")
                        .join(format!("{}.{ext}", hex(&a.hash())));
                    pm_blender::supervisor::write_private(&path, b).map_err(io)?;
                    handle.clone()
                }
            };
            assets.push(BlenderAsset {
                asset: a.id.as_str().into(),
                hash: a.hash(),
                handle,
            });
        }
        self.next += 1;
        let id = JobId::new(format!("{}-blender-{}", params.export.0, self.next));
        let (w, h) = job.snapshot.size();
        let frame = &job.snapshot.frame;
        let surfaces: Vec<SurfaceRequest> = job
            .layers
            .iter()
            .zip(
                job.keys
                    .iter()
                    .chain(std::iter::repeat(&ContentHash([0; 32]))),
            )
            .map(|(layer, key)| SurfaceRequest {
                layer: layer.as_str().into(),
                composition_path: frame
                    .composition_path
                    .iter()
                    .map(|c| c.as_str().into())
                    .collect(),
                time: Ticks::new(
                    (frame.time * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
                )
                .unwrap_or(Ticks::ZERO),
                shutter: frame
                    .shutter
                    .map(|(index, count)| ShutterSample { index, count }),
                size: Size::new(w, h),
                key: *key,
            })
            .collect();
        let request = BlenderJobRequest {
            job: id.clone(),
            priority: params.priority,
            snapshot: SnapshotRef {
                hash,
                encoding: SnapshotEncoding::RenderSnapshotJsonV1,
                handle: handle.clone(),
                bytes: ByteSize::new(bytes.len() as u64)
                    .map_err(|_| BlenderError::Failed("snapshot size".into()))?,
            },
            worker_script_hash: pm_blender::worker::worker_script_hash(),
            assets,
            settings: job.settings.clone(),
            surfaces,
            batch_layers: job.batch,
            output: handle,
        };
        pm_protocol::validate::Validate::validate(&request)
            .map_err(|v| BlenderError::Failed(v.to_string()))?;
        self.port.begin_blender_job(&id);
        let answer = self.port.call_until(
            Request::BlenderRender(request),
            Some(params.session),
            Instant::now() + FRAME_TIMEOUT,
            cancel,
        );
        let delivered = self.port.take_blender_surfaces(&id);
        match answer {
            Some(Ok(Response::Ack)) => {}
            Some(Ok(_)) => return Err(BlenderError::Failed("unexpected Blender answer".into())),
            Some(Err(e)) => return Err(of_engine(e)),
            None => {
                // Cancelled: stop Blender.
                let _ = self.port.call(
                    Request::CancelJobs(CancelJobs::Job { job: id }),
                    Some(params.session),
                );
                return Err(BlenderError::Cancelled);
            }
        }
        let device = delivered
            .first()
            .map(|s| s.device.clone())
            .ok_or_else(|| BlenderError::Failed("Blender delivered no surfaces".into()))?;
        let mut out = Vec::new();
        for s in delivered {
            pm_protocol::validate::Validate::validate(&s)
                .map_err(|v| BlenderError::Failed(v.to_string()))?;
            let Some(layer) = job.layers.get(s.request_index as usize) else {
                return Err(BlenderError::Failed(
                    "a Blender surface for no request".into(),
                ));
            };
            out.push(RenderedSurface {
                layer: layer.clone(),
                png: SurfacePng::File(area.join(format!("{}.png", hex(&s.key)))),
            });
        }
        Ok(RenderedFrame {
            surfaces: out,
            device,
        })
    }
}

/// Surfaces received for each Blender job that is waiting for them. Events
/// for a job nobody waits for (it was cancelled, or failed, before they
/// arrived) are dropped, so nothing accumulates across cancelled jobs.
#[derive(Default)]
pub(crate) struct Delivered {
    open: HashSet<JobId>,
    by_job: HashMap<JobId, Vec<BlenderSurface>>,
}

impl Delivered {
    pub fn begin(&mut self, job: &JobId) {
        self.open.insert(job.clone());
    }

    pub fn push(&mut self, s: BlenderSurface) {
        if !self.open.contains(&s.job) {
            return;
        }
        // Bounded: a job has at most the nested-frame limit of surfaces.
        let list = self.by_job.entry(s.job.clone()).or_default();
        if list.len() < limits::MAX_NESTED_FRAMES {
            list.push(s);
        }
    }

    /// The job's surfaces; the job is closed.
    pub fn take(&mut self, job: &JobId) -> Vec<BlenderSurface> {
        self.open.remove(job);
        self.by_job.remove(job).unwrap_or_default()
    }

    /// Jobs waiting and surfaces held (tests).
    #[cfg(test)]
    pub fn held(&self) -> (usize, usize) {
        (self.open.len(), self.by_job.values().map(Vec::len).sum())
    }
}

/// A host-call channel for threads off the engine's (the paused viewer's
/// Blender worker): calls wait in an outbox until the engine drains it
/// (`FramePresenter::drain_host_requests`), answers come back by token,
/// and `blenderSurface` events are kept for the job waiting for them.
pub(crate) struct HostLink {
    state: Mutex<LinkState>,
    answered: Condvar,
    wake: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct LinkState {
    next_token: u64,
    outbox: Vec<HostRequest>,
    answers: HashMap<u64, Result<Response, EngineError>>,
    /// Calls sent and not yet answered.
    waiting: HashSet<u64>,
    blender: Delivered,
    shutdown: bool,
}

impl HostLink {
    /// `wake` asks the engine's thread to drain the outbox soon.
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Arc<HostLink> {
        Arc::new(HostLink {
            state: Mutex::new(LinkState::default()),
            answered: Condvar::new(),
            wake,
        })
    }

    fn lock(&self) -> MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The calls to make now.
    pub fn drain(&self) -> Vec<HostRequest> {
        std::mem::take(&mut self.lock().outbox)
    }

    /// The answer to a drained call.
    pub fn answer(&self, token: u64, answer: Result<Response, EngineError>) {
        let mut st = self.lock();
        if st.waiting.remove(&token) {
            st.answers.insert(token, answer);
        }
        drop(st);
        self.answered.notify_all();
    }

    /// A host event: Blender surfaces of the jobs this link waits for.
    pub fn event(&self, event: &Event) {
        if let Event::BlenderSurface(s) = event {
            self.lock().blender.push(s.clone());
        }
    }

    /// The engine is going away: every wait ends.
    pub fn shutdown(&self) {
        self.lock().shutdown = true;
        self.answered.notify_all();
    }

    fn send(&self, request: Request, session: Option<SessionId>) -> u64 {
        let mut st = self.lock();
        st.next_token += 1;
        let token = st.next_token;
        st.waiting.insert(token);
        st.outbox.push(HostRequest {
            token,
            request,
            session,
        });
        drop(st);
        (self.wake)();
        token
    }

    fn wait(
        &self,
        token: u64,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
    ) -> Option<Result<Response, EngineError>> {
        let mut st = self.lock();
        loop {
            if let Some(answer) = st.answers.remove(&token) {
                return Some(answer);
            }
            let cancelled = cancel.is_some_and(|c| c.load(Ordering::Relaxed));
            if cancelled || st.shutdown {
                st.waiting.remove(&token);
                st.outbox.retain(|r| r.token != token);
                return if cancelled {
                    None
                } else {
                    Some(Err(EngineError::new(
                        ErrorCode::Cancelled,
                        ErrorScope::Connection,
                        "The engine is shutting down.",
                    )))
                };
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                st.waiting.remove(&token);
                return Some(Err(EngineError::new(
                    ErrorCode::Timeout,
                    ErrorScope::Connection,
                    "Host services did not answer in time.",
                )));
            }
            st = self
                .answered
                .wait_timeout(st, left.min(Duration::from_millis(100)))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Calls waiting, answers held, and Blender jobs open (tests).
    #[cfg(test)]
    pub fn pending(&self) -> (usize, usize, usize) {
        let st = self.lock();
        (st.waiting.len(), st.answers.len(), st.blender.held().0)
    }
}

impl HostPort for HostLink {
    fn call(&self, request: Request, session: Option<SessionId>) -> Result<Response, EngineError> {
        let token = self.send(request, session);
        self.wait(token, Instant::now() + HOST_CALL_TIMEOUT, None)
            .expect("only a cancel ends a wait without an answer")
    }

    fn call_until(
        &self,
        request: Request,
        session: Option<SessionId>,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Option<Result<Response, EngineError>> {
        let token = self.send(request, session);
        self.wait(token, deadline, Some(cancel))
    }

    fn begin_blender_job(&self, job: &JobId) {
        self.lock().blender.begin(job);
    }

    fn take_blender_surfaces(&self, job: &JobId) -> Vec<BlenderSurface> {
        self.lock().blender.take(job)
    }
}
