//! The job engine's render and export service (plan sections 17 and 20;
//! contracts 12, 13, 14).
//!
//! [`JobEngineService`] is the [`JobService`] the job engine installs. It runs
//! three lanes, each a thread at utility QoS with its own [`Renderer`] on one
//! shared GPU device:
//!
//! | Lane | Work | Class |
//! |---|---|---|
//! | Snapshots | `agentFrames` batches | agent snapshot |
//! | Agent exports | `export_video` jobs | agent export |
//! | User jobs | export sheet and render queue jobs, background renders | user job, background |
//!
//! Lanes run concurrently; their GPU submissions pass one [`gate::Gate`],
//! which keeps one submission in flight, lets the higher class go first, and
//! slices everything while the interactive engine plays (contract 13). Work
//! queues instead of failing for being busy (contract 12 invariant 3).
//!
//! Exports (section 17): media handles are minted and resolved through host
//! services, the capability check routes anything pm-render or pm-audio
//! cannot do (`unsupported`, so the host sends it to the legacy exporter),
//! host services create the output's temporary file (`open(wx, 0600)`), the
//! export is encoded, muxed, and verified beside it and renamed over it, and
//! `commitOutput` moves it into place (unique names for agents) and returns
//! the real path, which the final `jobStatus` carries. A failed or cancelled
//! export asks the host to remove its temporary file.
//!
//! Everything the service sends waits in an outbox the engine drains on its
//! own thread ([`pm_engine::Engine::pump`]); `wake` asks the utility process
//! to pump.

// Host answers carry protocol errors by value, as the engine hands them over.
#![allow(clippy::result_large_err)]

pub(crate) mod blender;
pub(crate) mod compat;
mod compile;
mod definitions;
mod fallback;
pub mod gate;
mod svg;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_engine::jobs::{Drained, HostRequest, JobDocument, JobService, WorkNote};
use pm_engine::priority::Scheduler;
use pm_export::render::{
    ExportJob, JobError, MediaPaths, OfflineClip, Progress, StillRouting, asset_records,
    encode_jpeg, encode_png, offline_media, plan_export_for, render_still, run_export_with_blender,
    sink_for, to_rgba8,
};
use pm_protocol::common::{AgentRunId, JobId, RequestId, SessionId};
use pm_protocol::envelope::{Envelope, Event, Message, Request, Response};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::host::{
    CapabilityHandle, CapabilityKind, CommitOutput, HandleAccess, MintHandle, OutputChoice,
    OutputDestinationRequest, OutputVerification, OverwritePolicy, PortId, ResolveHandle,
};
use pm_protocol::priority::{
    AgentFrameOutcome, AgentFrameRequest, AgentFrameResult, AgentImageFormat, BlenderUse,
    CancelJobs, ExportFormat, ImagePayload, JobAccepted, JobPreview, JobSpec, JobState, JobStatus,
    JobsCancelled, PriorityClass, SubmitJob,
};
use pm_protocol::slicing::{DeadlineSignal, SlicingPolicy};
use pm_render::Renderer;

use gate::{AGENT_EXPORT_LANE, Gate, LaneGate, SNAPSHOT_LANE, USER_LANE};

/// How long a lane waits for host services to answer one call.
pub const HOST_CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The most image bytes one `agentFrames` response carries; later frames of a
/// larger batch fail with `budgetExceeded` (the message limit is 16 MiB).
pub const AGENT_FRAMES_MAX_BYTES: usize = 14 << 20;
/// Progress events at most this often per job.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Unix mode of exported files (contract 14 invariant 4: owner-only).
const OUTPUT_MODE: u32 = 0o600;
/// How often a queued export waiting for offline media asks the host again
/// (section 14.8: queued jobs wait).
pub const OFFLINE_RECHECK: Duration = Duration::from_secs(2);

/// Asks the engine's host thread to pump the engine (pm-node posts an empty
/// action list through the utility process's threadsafe callback).
pub type Wake = Arc<dyn Fn() + Send + Sync>;

enum Work {
    Frames {
        port: PortId,
        id: RequestId,
        document: JobDocument,
        request: AgentFrameRequest,
        queued: Instant,
        cancel: Arc<AtomicBool>,
    },
    Export {
        job: JobId,
        host: PortId,
        submit: Box<SubmitJob>,
        document: JobDocument,
        cancel: Arc<AtomicBool>,
    },
}

impl Work {
    /// Whose font cache seeds the work's text measures with.
    fn seed_scope(&self) -> pm_render::text::served::SeedScope {
        match self {
            Work::Frames { port, document, .. } => pm_render::text::served::SeedScope {
                port: Some(port.0),
                session: Some(document.session.get()),
            },
            Work::Export { document, .. } => pm_render::text::served::SeedScope {
                port: None,
                session: Some(document.session.get()),
            },
        }
    }
}

struct JobRecord {
    class: PriorityClass,
    run: Option<AgentRunId>,
    host: PortId,
    state: JobState,
    progress: f64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    snapshots: VecDeque<Work>,
    agent_exports: VecDeque<Work>,
    user: Option<Scheduler<Work>>,
    jobs: HashMap<JobId, JobRecord>,
    /// Ended jobs, oldest first: their records are kept for the last
    /// `FINISHED_KEPT` (cancels and resubmissions of recent jobs answer
    /// correctly), older ones are dropped.
    finished: VecDeque<JobId>,
    outbox: Drained,
    next_token: u64,
    answers: HashMap<u64, Result<Response, EngineError>>,
    /// Media paths resolved per session (handles are bound to the session),
    /// each with the digest of the asset record it was resolved for.
    media: HashMap<SessionId, HashMap<String, ([u8; 32], PathBuf)>>,
    /// The run of each queued or running frame batch, for run cancels.
    frame_runs: Vec<(AgentRunId, Arc<AtomicBool>)>,
    /// `probeMedia` requests, answered in order by one probe lane (each
    /// opens and indexes a file; main sends at most one per asset at a
    /// time).
    probes: VecDeque<(PortId, RequestId, pm_protocol::media::ProbeMedia)>,
    shutdown: bool,
    stats: ServiceStats,
    /// Blender surfaces received for the lanes' Blender requests.
    blender: blender::Delivered,
    /// Definition pushes waiting for the compile worker's answer.
    compiles: compile::Queue,
}

/// Ended job records kept.
pub const FINISHED_KEPT: usize = 256;

impl State {
    /// `job` ended: keep its record among the last `FINISHED_KEPT`.
    fn retire(&mut self, job: JobId) {
        self.finished.push_back(job);
        while self.finished.len() > FINISHED_KEPT {
            if let Some(old) = self.finished.pop_front()
                && self.jobs.get(&old).is_some_and(|r| r.state.is_terminal())
            {
                self.jobs.remove(&old);
            }
        }
    }
}

/// Counters for status and tests.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStats {
    pub frames_rendered: u64,
    pub frame_batches: u64,
    pub exports_done: u64,
    pub exports_failed: u64,
    pub exports_routed: u64,
    pub exports_cancelled: u64,
    /// Agent frame batches answered while the interactive engine played.
    pub batches_during_playback: u64,
    /// Longest wait of a frame batch behind other work, in ms.
    pub max_queued_ms: u64,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    answers: Condvar,
    gate: Arc<Gate>,
    wake: Wake,
    gpu: Mutex<Option<(wgpu::Device, wgpu::Queue)>>,
    /// The engine's font service: lanes' renderers lay text out through it.
    fonts: Option<Arc<crate::fonts::hub::FontHub>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, port: PortId, envelope: Envelope) {
        self.lock().outbox.messages.push((port, envelope));
        (self.wake)();
    }

    fn status(&self, host: PortId, job: &JobId, update: impl FnOnce(&mut JobRecord)) {
        self.status_with(host, job, None, None, update);
    }

    /// A status event, with a preview of the latest encoded frame or an
    /// error that does not end the job (waiting for media).
    fn status_with(
        &self,
        host: PortId,
        job: &JobId,
        preview: Option<JobPreview>,
        note: Option<EngineError>,
        update: impl FnOnce(&mut JobRecord),
    ) {
        let mut st = self.lock();
        let Some(record) = st.jobs.get_mut(job) else {
            return;
        };
        update(record);
        let mut status = status_of(job, record, None, None);
        status.preview = preview;
        if record.state == JobState::WaitingForMedia {
            status.error = note;
        }
        st.outbox.messages.push((host, status_event(status)));
        drop(st);
        (self.wake)();
    }

    /// Makes host calls and waits for every answer (in order).
    fn call_many(
        &self,
        calls: Vec<(Request, Option<SessionId>)>,
    ) -> Vec<Result<Response, EngineError>> {
        if calls.is_empty() {
            return vec![];
        }
        let mut st = self.lock();
        let tokens: Vec<u64> = calls
            .into_iter()
            .map(|(request, session)| {
                st.next_token += 1;
                let token = st.next_token;
                st.outbox.host_requests.push(HostRequest {
                    token,
                    request,
                    session,
                });
                token
            })
            .collect();
        drop(st);
        (self.wake)();
        let deadline = Instant::now() + HOST_CALL_TIMEOUT;
        let mut st = self.lock();
        loop {
            if tokens.iter().all(|t| st.answers.contains_key(t)) || st.shutdown {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            st = self
                .answers
                .wait_timeout(st, left)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        tokens
            .iter()
            .map(|t| {
                st.answers.remove(t).unwrap_or_else(|| {
                    Err(EngineError::new(
                        ErrorCode::Timeout,
                        ErrorScope::Connection,
                        "Host services did not answer in time.",
                    ))
                })
            })
            .collect()
    }

    /// One host call answered by `deadline`, or `None` once `cancel` is set
    /// (Blender jobs: long, and cancelled with the export).
    fn call_until(
        &self,
        request: Request,
        session: Option<SessionId>,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Option<Result<Response, EngineError>> {
        let token = {
            let mut st = self.lock();
            st.next_token += 1;
            let token = st.next_token;
            st.outbox.host_requests.push(HostRequest {
                token,
                request,
                session,
            });
            token
        };
        (self.wake)();
        let mut st = self.lock();
        loop {
            if let Some(answer) = st.answers.remove(&token) {
                return Some(answer);
            }
            if cancel.load(Ordering::Relaxed) || st.shutdown {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Some(Err(EngineError::new(
                    ErrorCode::Timeout,
                    ErrorScope::Connection,
                    "Blender did not finish in time.",
                )));
            }
            st = self
                .answers
                .wait_timeout(st, left.min(Duration::from_millis(100)))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn call(&self, request: Request, session: Option<SessionId>) -> Result<Response, EngineError> {
        self.call_many(vec![(request, session)])
            .pop()
            .expect("one answer per call")
    }

    fn renderer(&self) -> Result<Renderer, String> {
        let (device, queue) = self.gpu()?;
        let mut renderer = Renderer::new(device, queue);
        if let Some(hub) = &self.fonts {
            renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
                hub.clone(),
                hub.clone(),
            ))));
        }
        Ok(renderer)
    }

    /// The service's device (one per process, shared by the lanes).
    fn gpu(&self) -> Result<(wgpu::Device, wgpu::Queue), String> {
        let mut gpu = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
        if gpu.is_none() {
            let g = pm_gpu::Gpu::new("pm-job").map_err(|e| e.to_string())?;
            *gpu = Some((g.device, g.queue));
        }
        let (device, queue) = gpu.as_ref().expect("set above");
        Ok((device.clone(), queue.clone()))
    }

    fn device(&self) -> Result<wgpu::Device, String> {
        self.gpu().map(|(device, _)| device)
    }
}

impl compat::CompatHost for Shared {
    fn call(&self, request: Request, session: SessionId) -> Result<Response, String> {
        Shared::call(self, request, Some(session)).map_err(|e| e.message)
    }
}

impl blender::HostPort for Shared {
    fn call(&self, request: Request, session: Option<SessionId>) -> Result<Response, EngineError> {
        Shared::call(self, request, session)
    }

    fn call_until(
        &self,
        request: Request,
        session: Option<SessionId>,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Option<Result<Response, EngineError>> {
        Shared::call_until(self, request, session, deadline, cancel)
    }

    fn begin_blender_job(&self, job: &JobId) {
        self.lock().blender.begin(job);
    }

    /// The surfaces host services delivered for Blender job `job`.
    fn take_blender_surfaces(&self, job: &JobId) -> Vec<pm_protocol::blender::BlenderSurface> {
        self.lock().blender.take(job)
    }
}

fn status_of(
    job: &JobId,
    record: &JobRecord,
    path: Option<String>,
    error: Option<EngineError>,
) -> JobStatus {
    JobStatus {
        job: job.clone(),
        class: record.class,
        state: record.state,
        progress: record.progress.clamp(0.0, 1.0),
        path: path.filter(|_| record.state == JobState::Done),
        error: error.filter(|_| record.state == JobState::Failed),
        preview: None,
    }
}

fn status_event(status: JobStatus) -> Envelope {
    Envelope::new(Message::Event {
        body: Event::JobStatus(status),
    })
}

fn job_error(job: &JobId, code: ErrorCode, message: impl Into<String>) -> EngineError {
    EngineError::new(code, ErrorScope::Job { job: job.clone() }, message)
}

/// The engine error for a failed render job: routing failures keep
/// `unsupported` so the host sends the work to the legacy engine.
fn engine_error(job: &JobId, e: &JobError) -> EngineError {
    let code = match e {
        JobError::Unsupported(_) => ErrorCode::Unsupported,
        JobError::Cancelled => ErrorCode::Cancelled,
        JobError::Invalid(_) => ErrorCode::InvalidMessage,
        JobError::Render(_) | JobError::Audio(_) => ErrorCode::ExportFailed,
        JobError::Export(_) => ErrorCode::ExportFailed,
        JobError::Blender(pm_blender::BlenderError::Missing(_)) => ErrorCode::BlenderMissing,
        JobError::Blender(pm_blender::BlenderError::Cancelled) => ErrorCode::Cancelled,
        JobError::Blender(pm_blender::BlenderError::Failed(_)) => ErrorCode::BlenderFailed,
    };
    job_error(job, code, e.to_string())
}

/// The job engine's service.
pub struct JobEngineService {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl JobEngineService {
    /// Starts the three lanes. `wake` must make the engine's host call
    /// [`pm_engine::Engine::pump`] soon.
    pub fn start(wake: Wake) -> Result<JobEngineService, String> {
        Self::start_with_fonts(wake, None)
    }

    /// [`JobEngineService::start`] with the engine's font hub: lanes draw
    /// text and captions (`crate::fonts::hub`).
    pub fn start_with_fonts(
        wake: Wake,
        fonts: Option<Arc<crate::fonts::hub::FontHub>>,
    ) -> Result<JobEngineService, String> {
        let state = State {
            user: Some(Scheduler::new(EngineProcess::Job)),
            ..State::default()
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            work: Condvar::new(),
            answers: Condvar::new(),
            gate: Arc::new(Gate::new()),
            wake,
            gpu: Mutex::new(None),
            fonts,
        });
        let mut threads = Vec::new();
        for (lane, name) in [
            (SNAPSHOT_LANE, "pm-job-snapshots"),
            (AGENT_EXPORT_LANE, "pm-job-agent-exports"),
            (USER_LANE, "pm-job-user"),
        ] {
            let shared = shared.clone();
            let handle = pm_engine::qos::spawn_engine_thread(EngineProcess::Job, name, move || {
                run_lane(&shared, lane)
            })
            .map_err(|e| format!("could not start the {name} lane: {e}"))?;
            threads.push(handle);
        }
        let probes = shared.clone();
        let handle =
            pm_engine::qos::spawn_engine_thread(EngineProcess::Job, "pm-job-probes", move || {
                run_probes(&probes)
            })
            .map_err(|e| format!("could not start the pm-job-probes lane: {e}"))?;
        threads.push(handle);
        let compiles = shared.clone();
        let handle =
            pm_engine::qos::spawn_engine_thread(EngineProcess::Job, "pm-job-compile", move || {
                compile::run(&compiles)
            })
            .map_err(|e| format!("could not start the pm-job-compile worker: {e}"))?;
        threads.push(handle);
        Ok(JobEngineService { shared, threads })
    }

    pub fn stats(&self) -> ServiceStats {
        self.shared.lock().stats.clone()
    }

    pub fn gate_stats(&self) -> gate::GateStats {
        self.shared.gate.stats()
    }

    /// A handle for status reads after the service moved into the engine.
    pub fn stats_handle(&self) -> StatsHandle {
        StatsHandle(self.shared.clone())
    }
}

/// Reads the service's counters from another owner.
#[derive(Clone)]
pub struct StatsHandle(Arc<Shared>);

impl StatsHandle {
    pub fn json(&self) -> serde_json::Value {
        let st = self.0.lock();
        serde_json::json!({
            "service": st.stats,
            "gate": self.0.gate.stats(),
            "queued": {
                "snapshots": st.snapshots.len(),
                "agentExports": st.agent_exports.len(),
                "user": st.user.as_ref().map_or(0, |s| s.len()),
            },
            "jobs": st.jobs.len(),
        })
    }
}

impl Drop for JobEngineService {
    fn drop(&mut self) {
        {
            let mut st = self.shared.lock();
            st.shutdown = true;
            for record in st.jobs.values() {
                record.cancel.store(true, Ordering::Relaxed);
            }
            for (_, cancel) in &st.frame_runs {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        self.shared.work.notify_all();
        self.shared.answers.notify_all();
        self.shared.gate.poke();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl JobService for JobEngineService {
    fn submit(
        &mut self,
        host: PortId,
        submit: SubmitJob,
        document: Option<JobDocument>,
    ) -> Result<JobAccepted, Box<EngineError>> {
        let job = submit.job.clone();
        let JobSpec::Export { .. } = &submit.spec else {
            return Err(Box::new(job_error(
                &job,
                ErrorCode::Unsupported,
                "Proxy generation is not in the job engine yet (Phase 4a).",
            )));
        };
        let Some(document) = document else {
            return Err(Box::new(job_error(
                &job,
                ErrorCode::StaleRevision,
                "The export has no document snapshot.",
            )));
        };
        let mut st = self.shared.lock();
        if st.jobs.get(&job).is_some_and(|r| !r.state.is_terminal()) {
            return Err(Box::new(job_error(
                &job,
                ErrorCode::InvalidMessage,
                "A job with that id is running.",
            )));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let class = submit.class;
        st.jobs.insert(
            job.clone(),
            JobRecord {
                class,
                run: submit.run.clone(),
                host,
                state: JobState::Queued,
                progress: 0.0,
                cancel: cancel.clone(),
            },
        );
        let work = Work::Export {
            job: job.clone(),
            host,
            submit: Box::new(submit),
            document,
            cancel,
        };
        let position = match class {
            PriorityClass::AgentExport => {
                st.agent_exports.push_back(work);
                st.agent_exports.len() as u32 - 1
            }
            _ => {
                let scheduler = st.user.as_mut().expect("present");
                scheduler
                    .push(class, work, pm_engine::clock::host_now())
                    .map_err(|e| {
                        Box::new(job_error(&job, ErrorCode::InvalidMessage, e.to_string()))
                    })?;
                scheduler.len() as u32 - 1
            }
        };
        drop(st);
        self.shared.work.notify_all();
        Ok(JobAccepted {
            job,
            state: JobState::Queued,
            position,
        })
    }

    fn cancel(&mut self, cancel: &CancelJobs) -> JobsCancelled {
        let mut st = self.shared.lock();
        let hit: Vec<JobId> = st
            .jobs
            .iter()
            .filter(|(id, r)| {
                !r.state.is_terminal()
                    && match cancel {
                        CancelJobs::Job { job } => *id == job,
                        CancelJobs::Run { run } => r.run.as_ref() == Some(run),
                    }
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &hit {
            st.jobs[id].cancel.store(true, Ordering::Relaxed);
        }
        if let CancelJobs::Run { run } = cancel {
            for (r, flag) in &st.frame_runs {
                if r == run {
                    flag.store(true, Ordering::Relaxed);
                }
            }
        }
        // Queued jobs end now; running ones stop at their next frame or band.
        let mut queued: Vec<(PortId, JobId)> = vec![];
        let is_hit = |w: &Work| matches!(w, Work::Export { job, .. } if hit.contains(job));
        let mut removed: Vec<Work> = vec![];
        let keep: VecDeque<Work> = st
            .agent_exports
            .drain(..)
            .filter_map(|w| {
                if is_hit(&w) {
                    removed.push(w);
                    None
                } else {
                    Some(w)
                }
            })
            .collect();
        st.agent_exports = keep;
        if let Some(user) = st.user.as_mut() {
            removed.extend(user.remove_where(is_hit));
        }
        for w in removed {
            if let Work::Export { job, host, .. } = w {
                queued.push((host, job));
            }
        }
        for (host, job) in queued {
            if let Some(record) = st.jobs.get_mut(&job) {
                record.state = JobState::Cancelled;
                let status = status_of(&job, record, None, None);
                st.outbox.messages.push((host, status_event(status)));
                st.stats.exports_cancelled += 1;
                st.retire(job);
            }
        }
        drop(st);
        self.shared.gate.poke();
        self.shared.work.notify_all();
        (self.shared.wake)();
        JobsCancelled { jobs: hit }
    }

    fn agent_frames(
        &mut self,
        port: PortId,
        id: RequestId,
        document: JobDocument,
        request: AgentFrameRequest,
    ) -> Result<(), Box<EngineError>> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut st = self.shared.lock();
        st.frame_runs.push((request.run.clone(), cancel.clone()));
        st.snapshots.push_back(Work::Frames {
            port,
            id,
            document,
            request,
            queued: Instant::now(),
            cancel,
        });
        drop(st);
        self.shared.work.notify_all();
        Ok(())
    }

    fn probe_media(
        &mut self,
        port: PortId,
        id: RequestId,
        probe: pm_protocol::media::ProbeMedia,
    ) -> Result<(), Box<EngineError>> {
        // Probing opens and indexes the file: off the engine thread, on the
        // probe lane.
        self.shared.lock().probes.push_back((port, id, probe));
        self.shared.work.notify_all();
        Ok(())
    }

    fn signal(&mut self, signal: &DeadlineSignal) {
        self.shared.gate.signal(signal);
        if let DeadlineSignal::PlaybackStarted { .. } | DeadlineSignal::PlaybackStopped { .. } =
            signal
        {
            let playing = matches!(signal, DeadlineSignal::PlaybackStarted { .. });
            if let Some(user) = self.shared.lock().user.as_mut() {
                user.set_playing(playing);
            }
            self.shared.work.notify_all();
        }
    }

    fn policy(&mut self, policy: &SlicingPolicy) {
        self.shared.gate.policy(policy);
    }

    fn host_answer(&mut self, token: u64, answer: Result<Response, EngineError>) {
        self.shared.lock().answers.insert(token, answer);
        self.shared.answers.notify_all();
    }

    fn drain(&mut self) -> Drained {
        std::mem::take(&mut self.shared.lock().outbox)
    }

    fn host_event(&mut self, event: &pm_protocol::envelope::Event) {
        if let pm_protocol::envelope::Event::BlenderSurface(s) = event {
            self.shared.lock().blender.push(s.clone());
        }
    }

    fn session_closed(&mut self, session: SessionId) {
        let mut st = self.shared.lock();
        st.media.remove(&session);
        // Queued frame batches for the session are answered as unknown.
        let mut answers = vec![];
        let keep: VecDeque<Work> = st
            .snapshots
            .drain(..)
            .filter_map(|w| match w {
                Work::Frames {
                    port, id, document, ..
                } if document.session == session => {
                    answers.push((port, id));
                    None
                }
                w => Some(w),
            })
            .collect();
        st.snapshots = keep;
        for (port, id) in answers {
            let error = EngineError::new(
                ErrorCode::UnknownSession,
                ErrorScope::Request { id },
                "The session closed before its frames rendered.",
            );
            st.outbox
                .messages
                .push((port, Envelope::new(Message::Error { id, error })));
        }
        drop(st);
        (self.shared.wake)();
    }

    fn definitions_pushed(
        &mut self,
        port: PortId,
        id: RequestId,
        session: SessionId,
        update: &pm_protocol::definitions::DefinitionsUpdate,
    ) -> bool {
        self.shared.lock().compiles.0.push_back(compile::Pending {
            port,
            id,
            session,
            update: update.clone(),
        });
        self.shared.work.notify_all();
        true
    }

    fn port_closed(&mut self, port: PortId) {
        let mut st = self.shared.lock();
        let keep: VecDeque<Work> = st
            .snapshots
            .drain(..)
            .filter(|w| !matches!(w, Work::Frames { port: p, .. } if *p == port))
            .collect();
        st.snapshots = keep;
        // Jobs whose host went away cannot commit: cancel them (agent jobs
        // never resume; user jobs are resubmitted by the host, section 17).
        for record in st.jobs.values() {
            if record.host == port && !record.state.is_terminal() {
                record.cancel.store(true, Ordering::Relaxed);
            }
        }
        drop(st);
        self.shared.gate.poke();
    }
}

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

fn next_work(shared: &Shared, lane: usize) -> Option<Work> {
    let mut st = shared.lock();
    loop {
        if st.shutdown {
            return None;
        }
        let work = match lane {
            SNAPSHOT_LANE => st.snapshots.pop_front(),
            AGENT_EXPORT_LANE => st.agent_exports.pop_front(),
            _ => st
                .user
                .as_mut()
                .and_then(|s| s.pop(pm_engine::clock::host_now()))
                .map(|s| s.item),
        };
        if let Some(work) = work {
            return Some(work);
        }
        st = shared
            .work
            .wait_timeout(st, Duration::from_millis(500))
            .map(|(g, _)| g)
            .unwrap_or_else(|e| e.into_inner().0);
    }
}

fn run_lane(shared: &Arc<Shared>, lane: usize) {
    let gate = LaneGate {
        gate: shared.gate.clone(),
        lane,
    };
    let mut renderer: Option<Renderer> = None;
    let mut blender = LaneBlender::default();
    let mut definitions = definitions::LaneDefinitions::default();
    while let Some(work) = next_work(shared, lane) {
        if renderer.is_none() {
            match shared.renderer() {
                Ok(r) => renderer = Some(r),
                Err(message) => {
                    fail_work(shared, work, ErrorCode::DeviceLost, &message);
                    continue;
                }
            }
        }
        let r = renderer.as_mut().expect("set above");
        // The work's own definitions (contract 10), before it is planned.
        let loaded = match &work {
            Work::Frames { document, .. } | Work::Export { document, .. } => {
                definitions.use_for(r, document)
            }
        };
        if let Err(message) = loaded {
            fail_work(shared, work, ErrorCode::InvalidMessage, &message);
            continue;
        }
        // Programs only the WebGL runner draws run in a window's runner
        // through the host (contract 14): the job waits for them.
        let (session, cancel) = match &work {
            Work::Frames {
                document, cancel, ..
            }
            | Work::Export {
                document, cancel, ..
            } => (document.session, cancel.clone()),
        };
        r.set_fallback_host(Some(Arc::new(fallback::HostFallback::new(
            shared.clone(),
            session,
            cancel,
        ))));
        // Text measures with the font cache seeds of the window the work is
        // for: the requesting port's window, or the window main opened the
        // session for (an export).
        r.set_text_seed_scope(work.seed_scope());
        // Level 1 SVG rasters come from the document's window through host
        // services (decision S1); job frames (exports, agent frames) never
        // draw a stand-in for one.
        r.set_svg_wait(true);
        let document = match &work {
            Work::Frames { document, .. } | Work::Export { document, .. } => document,
        };
        r.set_svg_source(Some(Arc::new(svg::HostSvgRasters::new(
            shared.clone(),
            document,
        ))));
        match work {
            Work::Frames {
                port,
                id,
                document,
                request,
                queued,
                cancel,
            } => {
                run_frames(
                    shared, r, &gate, port, id, &document, &request, queued, &cancel,
                );
                shared
                    .lock()
                    .frame_runs
                    .retain(|(_, flag)| !Arc::ptr_eq(flag, &cancel));
            }
            Work::Export {
                job,
                host,
                submit,
                document,
                cancel,
            } => run_job(
                shared,
                r,
                &gate,
                &job,
                host,
                &submit,
                &document,
                &cancel,
                &mut blender,
            ),
        }
        r.set_svg_source(None);
        // Keep the pool small between work items (lease pressure).
        r.trim(64 << 20);
    }
}

fn fail_work(shared: &Shared, work: Work, code: ErrorCode, message: &str) {
    match work {
        Work::Frames { port, id, .. } => {
            let error = EngineError::new(code, ErrorScope::Request { id }, message);
            shared.send(port, Envelope::new(Message::Error { id, error }));
        }
        Work::Export { job, host, .. } => {
            let error = job_error(&job, code, message);
            let mut st = shared.lock();
            if let Some(record) = st.jobs.get_mut(&job) {
                record.state = JobState::Failed;
                let status = status_of(&job, record, None, Some(error));
                st.outbox.messages.push((host, status_event(status)));
                st.retire(job);
            }
            drop(st);
            (shared.wake)();
        }
    }
}

/// Media paths for `document`'s assets, resolved through host services:
/// a `mediaRead` handle per asset, then its path. Assets the host cannot
/// resolve are offline (the capability check routes what needs them).
/// The media of `document`'s assets, through host services. Paths are kept
/// per session while the asset's record is unchanged: a relink or a replaced
/// file changes the record, and the asset is resolved again.
/// Assets without a path are asked again on every call (a waiting export sees
/// a relink or a remounted volume).
fn resolve_media(shared: &Shared, document: &JobDocument) -> MediaPaths {
    let session = document.session;
    let records = asset_records(&document.document);
    let mut known = MediaPaths::default();
    let missing: Vec<String> = {
        let st = shared.lock();
        let cached = st.media.get(&session);
        records
            .iter()
            .filter(|(asset, digest)| match cached.and_then(|c| c.get(asset)) {
                Some((d, path)) if d == digest => {
                    known.insert(asset.clone(), path.clone());
                    false
                }
                _ => true,
            })
            .map(|(asset, _)| asset.clone())
            .collect()
    };
    if missing.is_empty() {
        return known;
    }
    let mints = shared.call_many(
        missing
            .iter()
            .map(|asset| {
                (
                    Request::MintHandle(MintHandle {
                        kind: CapabilityKind::MediaRead {
                            asset: asset.as_str().into(),
                        },
                        session,
                    }),
                    Some(session),
                )
            })
            .collect(),
    );
    let granted: Vec<(String, CapabilityHandle)> = missing
        .into_iter()
        .zip(mints)
        .filter_map(|(asset, answer)| match answer {
            Ok(Response::Handle(grant)) => Some((asset, grant.handle)),
            _ => None,
        })
        .collect();
    let resolved = shared.call_many(
        granted
            .iter()
            .map(|(_, handle)| {
                (
                    Request::ResolveHandle(ResolveHandle {
                        handle: handle.clone(),
                        session,
                    }),
                    Some(session),
                )
            })
            .collect(),
    );
    for ((asset, _), answer) in granted.into_iter().zip(resolved) {
        if let Ok(Response::HandleResolved(r)) = answer
            && r.access == HandleAccess::Read
            && matches!(r.kind, CapabilityKind::MediaRead { .. })
        {
            known.insert(asset, PathBuf::from(r.path));
        }
    }
    // Only the document's current assets stay cached.
    let cache: HashMap<String, ([u8; 32], PathBuf)> = records
        .into_iter()
        .filter_map(|(asset, digest)| {
            let path = known.paths.get(&asset)?.clone();
            Some((asset, (digest, path)))
        })
        .collect();
    shared.lock().media.insert(session, cache);
    known
}

fn asset_scopes(media: &MediaPaths) -> Vec<ErrorScope> {
    media
        .paths
        .keys()
        .take(pm_protocol::error::MAX_IN_FLIGHT_SUSPECTS)
        .map(|a| ErrorScope::Asset {
            asset: a.as_str().into(),
        })
        .collect()
}

fn note(shared: &Shared, note: WorkNote) {
    shared.lock().outbox.work.push(note);
    (shared.wake)();
}

#[allow(clippy::too_many_arguments)]
fn run_frames(
    shared: &Shared,
    renderer: &mut Renderer,
    gate: &LaneGate,
    port: PortId,
    id: RequestId,
    document: &JobDocument,
    request: &AgentFrameRequest,
    queued: Instant,
    cancel: &AtomicBool,
) {
    let queued_ms = queued.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    let media = resolve_media(shared, document);
    let token = {
        let mut st = shared.lock();
        st.next_token += 1;
        st.next_token
    };
    note(
        shared,
        WorkNote::Started {
            token,
            scopes: asset_scopes(&media),
        },
    );
    let playing_at_start = shared.gate.is_playing();
    let mut routing = StillRouting::new();
    let mut outcomes = Vec::with_capacity(request.frames.len());
    let mut bytes = 0usize;
    for (index, frame) in request.frames.iter().enumerate() {
        let index = index as u32;
        let fail = |code: ErrorCode, message: String| AgentFrameOutcome::Failed {
            index,
            error: EngineError::new(code, ErrorScope::Request { id }, message),
        };
        if cancel.load(Ordering::Relaxed) {
            outcomes.push(fail(
                ErrorCode::Cancelled,
                "The agent run was cancelled.".into(),
            ));
            continue;
        }
        let rendered = render_still(
            renderer,
            &document.document,
            frame.composition.as_str(),
            frame.time,
            request.max_dimension,
            &media,
            &mut routing,
            gate,
            still_seed(frame.composition.as_str()),
            cancel,
        );
        let outcome = match rendered {
            Ok(f) => {
                let rgba = to_rgba8(&f.rgba);
                let image = match request.format {
                    AgentImageFormat::Jpeg { quality } => {
                        encode_jpeg(&rgba, f.width, f.height, quality)
                    }
                    AgentImageFormat::Png => encode_png(&rgba, f.width, f.height),
                };
                match image {
                    Ok(image) if bytes + image.len() <= AGENT_FRAMES_MAX_BYTES => {
                        bytes += image.len();
                        AgentFrameOutcome::Rendered {
                            index,
                            revision: document.revision,
                            width: f.width,
                            height: f.height,
                            image: ImagePayload::Inline {
                                bytes: pm_protocol::common::Bytes(image),
                            },
                            blender: BlenderUse::NotUsed,
                        }
                    }
                    Ok(_) => fail(
                        ErrorCode::BudgetExceeded,
                        "The batch's images exceed one message; ask for fewer or smaller frames."
                            .into(),
                    ),
                    Err(e) => fail(ErrorCode::Internal, e.to_string()),
                }
            }
            Err(JobError::Unsupported(list)) => fail(ErrorCode::Unsupported, list.join("; ")),
            Err(JobError::Cancelled) => fail(ErrorCode::Cancelled, "Cancelled.".into()),
            Err(e) => fail(ErrorCode::Internal, e.to_string()),
        };
        outcomes.push(outcome);
    }
    note(shared, WorkNote::Ended { token });
    let rendered = outcomes
        .iter()
        .filter(|o| matches!(o, AgentFrameOutcome::Rendered { .. }))
        .count() as u64;
    {
        let mut st = shared.lock();
        st.stats.frames_rendered += rendered;
        st.stats.frame_batches += 1;
        st.stats.max_queued_ms = st.stats.max_queued_ms.max(u64::from(queued_ms));
        if playing_at_start || shared.gate.is_playing() {
            st.stats.batches_during_playback += 1;
        }
    }
    let mut envelope = Envelope::new(Message::Response {
        id,
        body: Response::AgentFrames(AgentFrameResult {
            outcomes,
            queued_ms,
        }),
    });
    envelope.session = Some(document.session);
    envelope.revision = Some(document.revision);
    shared.send(port, envelope);
}

/// The slicing pass seed of agent stills: per composition, so the cost of a
/// composition's stills carries over from one agent turn to the next.
fn still_seed(composition: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("agent-still", composition).hash(&mut h);
    h.finish()
}

/// What an output destination answered: the handle and its path.
struct Destination {
    handle: CapabilityHandle,
    path: PathBuf,
}

fn destination(
    shared: &Shared,
    job: &JobId,
    session: SessionId,
    choice: &OutputChoice,
    folder: bool,
) -> Result<Destination, EngineError> {
    let overwrite = match choice {
        OutputChoice::AgentAutomatic { .. } => OverwritePolicy::UniqueName,
        // The user confirmed the name in the save dialog or chose it on
        // the command line.
        OutputChoice::UserChosen { .. } => OverwritePolicy::Replace,
    };
    let answer = shared.call(
        Request::OutputDestination(OutputDestinationRequest {
            job: job.clone(),
            choice: choice.clone(),
            overwrite,
            folder,
        }),
        Some(session),
    )?;
    let Response::OutputDestination(d) = answer else {
        return Err(job_error(
            job,
            ErrorCode::Internal,
            "The host answered outputDestination with something else.",
        ));
    };
    let resolved = shared.call(
        Request::ResolveHandle(ResolveHandle {
            handle: d.temporary.clone(),
            session,
        }),
        Some(session),
    );
    match resolved {
        Ok(Response::HandleResolved(r))
            if r.access == HandleAccess::Write
                && matches!(&r.kind, CapabilityKind::OutputWrite { job: j } if j == job) =>
        {
            Ok(Destination {
                handle: d.temporary,
                path: PathBuf::from(r.path),
            })
        }
        other => {
            // The temporary file exists: ask the host to remove it.
            let _ = shared.call(
                Request::CommitOutput(CommitOutput {
                    job: job.clone(),
                    temporary: d.temporary,
                    commit: false,
                    verification: None,
                }),
                Some(session),
            );
            Err(match other {
                Err(e) => e,
                Ok(_) => job_error(
                    job,
                    ErrorCode::CapabilityRejected,
                    "The output handle is not this job's.",
                ),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_job(
    shared: &Arc<Shared>,
    renderer: &mut Renderer,
    gate: &LaneGate,
    job: &JobId,
    host: PortId,
    submit: &SubmitJob,
    document: &JobDocument,
    cancel: &Arc<AtomicBool>,
    blender: &mut LaneBlender,
) {
    let JobSpec::Export {
        composition,
        settings,
        ..
    } = &submit.spec
    else {
        return;
    };
    let session = document.session;
    shared.status(host, job, |r| {
        r.state = JobState::Running;
        r.progress = 0.0;
    });
    let media = Arc::new(resolve_media(shared, document));
    let token = {
        let mut st = shared.lock();
        st.next_token += 1;
        st.next_token
    };
    note(
        shared,
        WorkNote::Started {
            token,
            scopes: asset_scopes(&media),
        },
    );
    let finish = |state: JobState, path: Option<String>, error: Option<EngineError>| {
        let mut st = shared.lock();
        match state {
            JobState::Done => st.stats.exports_done += 1,
            JobState::Cancelled => st.stats.exports_cancelled += 1,
            _ if error
                .as_ref()
                .is_some_and(|e| e.code == ErrorCode::Unsupported) =>
            {
                st.stats.exports_routed += 1
            }
            _ => st.stats.exports_failed += 1,
        }
        st.outbox.work.push(WorkNote::Ended { token });
        if let Some(record) = st.jobs.get_mut(job) {
            record.state = state;
            if state == JobState::Done {
                record.progress = 1.0;
            }
            let status = status_of(job, record, path, error);
            st.outbox.messages.push((host, status_event(status)));
            st.retire(job.clone());
        }
        drop(st);
        (shared.wake)();
    };
    let mut export = ExportJob {
        document: document.document.clone(),
        composition: composition.as_str().to_owned(),
        settings: settings.clone(),
        media,
        destination: PathBuf::from("pending"),
        overwrite: pm_export::Overwrite::InPlace,
        mode: Some(OUTPUT_MODE),
        seed: {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            job.hash(&mut h);
            h.finish()
        },
    };
    // Queued user jobs wait for relinking. An interactive export has already
    // obtained the user's "Export Without Missing Media" choice in main;
    // agent exports are unattended. Both use pm-export's missing-layer
    // appearance and report the clips to their caller.
    let offline = offline_media(&document.document, composition.as_str(), &export.media);
    if !offline.is_empty() {
        let message = offline_message(&offline);
        if settings.draft_from_proxies {
            return finish(
                JobState::Failed,
                None,
                Some(job_error(
                    job,
                    ErrorCode::MediaOffline,
                    format!("{message} No proxies of them exist to export from."),
                )),
            );
        }
        if submit.class == PriorityClass::UserJob && job.as_str().starts_with("queue-export-") {
            match wait_for_media(
                shared,
                host,
                job,
                document,
                composition.as_str(),
                cancel,
                &message,
            ) {
                Some(media) => export.media = Arc::new(media),
                None => return finish(JobState::Cancelled, None, None),
            }
            shared.status(host, job, |r| r.state = JobState::Running);
        }
    }
    // Blender compositions (contract 11): the lane's Blender stage, when
    // host services run Blender; a host without Blender routes them.
    let stage = match lane_blender(shared, blender, &export, document, submit.class, job) {
        Ok(stage) => stage,
        Err(e) => return finish(JobState::Failed, None, Some(e)),
    };
    // Routing first: nothing is created for work the engine cannot do.
    let mut plan = match plan_export_for(&export, stage.is_some(), renderer) {
        Ok(plan) => plan,
        Err(e) => {
            let state = if matches!(e, JobError::Cancelled) {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            return finish(state, None, Some(engine_error(job, &e)));
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return finish(JobState::Cancelled, None, None);
    }
    let Some(choice) = submit.output.as_ref() else {
        return finish(
            JobState::Failed,
            None,
            Some(job_error(
                job,
                ErrorCode::InvalidMessage,
                "An export needs an output choice.",
            )),
        );
    };
    let folder = matches!(settings.format, ExportFormat::PngSequence { .. });
    let dest = match destination(shared, job, session, choice, folder) {
        Ok(d) => d,
        Err(e) => {
            return finish(
                JobState::Failed,
                None,
                Some(EngineError {
                    scope: ErrorScope::Job { job: job.clone() },
                    ..e
                }),
            );
        }
    };
    export.destination = dest.path.clone();
    plan.spec.destination = dest.path.clone();
    let discard = |shared: &Shared| {
        let _ = shared.call(
            Request::CommitOutput(CommitOutput {
                job: job.clone(),
                temporary: dest.handle.clone(),
                commit: false,
                verification: None,
            }),
            Some(session),
        );
    };
    let sink = match sink_for(&plan) {
        Ok(s) => s,
        Err(e) => {
            discard(shared);
            return finish(JobState::Failed, None, Some(engine_error(job, &e)));
        }
    };
    let mut last = Instant::now() - PROGRESS_EVERY;
    let mut last_fraction = 0.0;
    let mut progress = |p: Progress| {
        let f = p.fraction();
        let preview = p.preview.as_ref().map(|v| JobPreview {
            frame: v.frame.min(u64::from(u32::MAX)) as u32,
            width: v.width,
            height: v.height,
            jpeg: pm_protocol::common::Bytes(v.jpeg.clone()),
        });
        if f >= 1.0
            || preview.is_some()
            || (last.elapsed() >= PROGRESS_EVERY && f - last_fraction >= 0.005)
        {
            last = Instant::now();
            last_fraction = f;
            shared.status_with(host, job, preview, None, |r| r.progress = f.min(0.999));
        }
    };
    // A `raster` override's layers (the exporting window's producers):
    // each frame asks the host for them (`compat::JobCompat`).
    let job_compat = (!settings.delivery.compat.is_empty()).then(|| {
        let id = document
            .document
            .root()
            .map(pm_render::js::layer_id)
            .unwrap_or_default();
        Arc::new(compat::JobCompat::new(
            shared.clone() as Arc<dyn compat::CompatHost>,
            document.session,
            id,
            document.revision,
            settings.delivery.compat.clone(),
        ))
    });
    renderer.set_compat_rasters(
        job_compat
            .clone()
            .map(|c| c as Arc<dyn pm_render::compat::CompatRasters>),
    );
    // A compatibility context in the exporting window draws over each frame
    // (today's WebM frames, `frame:rendered` before the canvas was encoded).
    let mut plan = plan;
    plan.frame_post = job_compat
        .as_ref()
        .and_then(|c| c.context_frames(export.composition.clone()));
    let result = run_export_with_blender(
        renderer,
        &export,
        plan,
        sink,
        gate,
        cancel,
        &mut progress,
        stage,
    );
    renderer.set_compat_rasters(None);
    // A layer the window's override could not answer for fails with why.
    let result = match (result, job_compat.as_ref().and_then(|c| c.failure())) {
        (Err(_), Some(why)) => Err(JobError::Render(format!(
            "An extension in the editor draws a layer of this export, and it could not be drawn: {why}"
        ))),
        (result, _) => result,
    };
    match result {
        Ok(delivered) => {
            let r = &delivered.report;
            let bytes = if folder {
                r.bytes
            } else {
                std::fs::metadata(&dest.path).map(|m| m.len()).unwrap_or(0)
            };
            let verification = OutputVerification {
                codec: r.codec.clone(),
                frames: r.frames.min(u64::from(u32::MAX)) as u32,
                audio_samples: pm_protocol::common::Count::new(
                    r.audio.as_ref().map_or(0, |a| a.samples),
                )
                .unwrap_or(pm_protocol::common::Count::new(0).expect("0 is safe")),
                bytes: pm_protocol::common::ByteSize::new(bytes)
                    .unwrap_or(pm_protocol::common::ByteSize::new(0).expect("0 is safe")),
            };
            let committed = shared.call(
                Request::CommitOutput(CommitOutput {
                    job: job.clone(),
                    temporary: dest.handle.clone(),
                    commit: true,
                    verification: Some(verification),
                }),
                Some(session),
            );
            match committed {
                Ok(Response::OutputCommitted(c)) if c.path.is_some() => {
                    finish(JobState::Done, c.path, None)
                }
                Ok(_) => finish(
                    JobState::Failed,
                    None,
                    Some(job_error(
                        job,
                        ErrorCode::ExportFailed,
                        "The host did not place the output.",
                    )),
                ),
                Err(e) => finish(
                    JobState::Failed,
                    None,
                    Some(EngineError {
                        scope: ErrorScope::Job { job: job.clone() },
                        ..e
                    }),
                ),
            }
        }
        Err(JobError::Cancelled) => {
            discard(shared);
            finish(JobState::Cancelled, None, None)
        }
        Err(e) => {
            discard(shared);
            finish(JobState::Failed, None, Some(engine_error(job, &e)))
        }
    }
}

/// A lane's Blender stage: created when its first Blender export starts.
#[derive(Default)]
struct LaneBlender {
    stage: Option<pm_export::blender::ExportBlender>,
    params: Option<Arc<Mutex<blender::JobParams>>>,
}

/// Bytes of decoded Blender surfaces a lane keeps in memory (today's
/// renderer budget; contract 8's `blenderSurfaces` category).
const BLENDER_SURFACE_BYTES: u64 = 512 << 20;

/// The lane's Blender stage for `export` when its composition renders with
/// Blender: `None` when it does not, or when host services do not run
/// Blender (the export then routes as before); an error when Blender is
/// missing (the export fails with appendix D's sentence).
fn lane_blender<'b>(
    shared: &Arc<Shared>,
    blender: &'b mut LaneBlender,
    export: &ExportJob,
    document: &JobDocument,
    class: PriorityClass,
    job: &JobId,
) -> Result<Option<&'b mut pm_export::blender::ExportBlender>, EngineError> {
    let needs =
        pm_render::unsupported_with_media(&export.document, &export.composition, &*export.media)
            .iter()
            .any(|u| u.feature == pm_render::Feature::Blender);
    if !needs {
        return Ok(None);
    }
    let params = blender::JobParams {
        session: document.session,
        priority: class,
        export: job.clone(),
    };
    let shared_params = blender
        .params
        .get_or_insert_with(|| Arc::new(Mutex::new(params.clone())))
        .clone();
    *shared_params.lock().unwrap_or_else(|e| e.into_inner()) = params;
    if blender.stage.is_none() {
        let mut backend = blender::HostBackend::new(shared.clone(), shared_params);
        // A host that does not serve Blender keeps routing these exports.
        match shared.call(Request::BlenderStatus, Some(document.session)) {
            Ok(Response::BlenderStatus(_)) => {}
            _ => return Ok(None),
        }
        let area = backend
            .area()
            .map_err(|e| job_error(job, ErrorCode::BlenderFailed, e.to_string()))?;
        let cache =
            pm_blender::SurfaceCache::new(BLENDER_SURFACE_BYTES, Some(area.0.join("surfaces")));
        blender.stage = Some(pm_export::blender::ExportBlender::new(
            pm_blender::job::BlenderStage::new(cache),
            Box::new(backend),
        ));
    }
    Ok(blender.stage.as_mut())
}

/// The probe lane: answers `probeMedia` requests in order until shutdown.
/// Requests still queued at shutdown are answered as cancelled.
fn run_probes(shared: &Shared) {
    loop {
        let next = {
            let mut st = shared.lock();
            loop {
                if st.shutdown {
                    let left: Vec<_> = st.probes.drain(..).collect();
                    drop(st);
                    for (port, id, _) in left {
                        let error = EngineError::new(
                            ErrorCode::Cancelled,
                            ErrorScope::Request { id },
                            "The job engine is shutting down.",
                        );
                        let mut envelope = Envelope::new(Message::Error { id, error });
                        envelope.session = None;
                        shared.send(port, envelope);
                    }
                    return;
                }
                if let Some(p) = st.probes.pop_front() {
                    break p;
                }
                // Every push and the shutdown flag notify `work` under this
                // lock, so the lane sleeps until there is something to do.
                st = shared.work.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (port, id, probe) = next;
        probe_media(shared, port, id, probe);
    }
}

/// Interpret Footage's facts of the file a `mediaRead` handle names: the
/// handle resolves through the host (session 0: no engine session), then
/// `pm_media::facts` reads the file with the asset's `interpret` member.
fn probe_media(
    shared: &Shared,
    port: PortId,
    id: RequestId,
    probe: pm_protocol::media::ProbeMedia,
) {
    let fail = |code: ErrorCode, message: String| {
        let mut envelope = Envelope::new(pm_protocol::envelope::Message::Error {
            id,
            error: EngineError::new(code, ErrorScope::Request { id }, message),
        });
        envelope.session = None;
        shared.send(port, envelope);
    };
    let resolved = shared.call(
        Request::ResolveHandle(ResolveHandle {
            handle: probe.handle.clone(),
            session: SessionId::ZERO,
        }),
        None,
    );
    let path = match resolved {
        Ok(Response::HandleResolved(r))
            if r.access == HandleAccess::Read
                && matches!(r.kind, CapabilityKind::MediaRead { .. }) =>
        {
            PathBuf::from(r.path)
        }
        Ok(_) => {
            return fail(
                ErrorCode::CapabilityRejected,
                "The handle does not name media to read.".into(),
            );
        }
        Err(e) => return fail(e.code, e.message),
    };
    match pm_media::facts::media_facts(&path, probe.interpret.as_deref()) {
        Ok(facts) => shared.send(
            port,
            Envelope::new(pm_protocol::envelope::Message::Response {
                id,
                body: Response::MediaFacts(Box::new(facts)),
            }),
        ),
        Err(e) => fail(ErrorCode::DecodeFailed, format!("{}: {e}", path.display())),
    }
}

/// "2 clips are offline: A001_C003.mov, Interview.wav." (appendix D).
fn offline_message(offline: &[OfflineClip]) -> String {
    let names: Vec<&str> = offline.iter().take(5).map(|c| c.name.as_str()).collect();
    let more = offline.len().saturating_sub(names.len());
    format!(
        "{} {} offline: {}{}.",
        offline.len(),
        if offline.len() == 1 {
            "clip is"
        } else {
            "clips are"
        },
        names.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    )
}

/// Waits until every clip the export needs is online again (section 14.8:
/// "Waiting for media · 2 clips offline"), asking the host every
/// [`OFFLINE_RECHECK`]. `None` when the job is cancelled while waiting.
fn wait_for_media(
    shared: &Shared,
    host: PortId,
    job: &JobId,
    document: &JobDocument,
    composition: &str,
    cancel: &AtomicBool,
    message: &str,
) -> Option<MediaPaths> {
    shared.status_with(
        host,
        job,
        None,
        Some(job_error(job, ErrorCode::MediaOffline, message)),
        |r| r.state = JobState::WaitingForMedia,
    );
    let mut said = message.to_owned();
    loop {
        let deadline = Instant::now() + OFFLINE_RECHECK;
        while Instant::now() < deadline {
            if cancel.load(Ordering::Relaxed) || shared.lock().shutdown {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let media = resolve_media(shared, document);
        let offline = offline_media(&document.document, composition, &media);
        if offline.is_empty() {
            return Some(media);
        }
        let now = offline_message(&offline);
        if now != said {
            shared.status_with(
                host,
                job,
                None,
                Some(job_error(job, ErrorCode::MediaOffline, now.clone())),
                |_| {},
            );
            said = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_messages_name_the_clips() {
        let clip = |n: &str| OfflineClip {
            asset: n.into(),
            name: n.into(),
        };
        assert_eq!(
            offline_message(&[clip("A001_C003.mov")]),
            "1 clip is offline: A001_C003.mov."
        );
        let many: Vec<_> = (0..7).map(|i| clip(&format!("c{i}.mov"))).collect();
        assert_eq!(
            offline_message(&many),
            "7 clips are offline: c0.mov, c1.mov, c2.mov, c3.mov, c4.mov and 2 more."
        );
    }

    fn record(state: JobState) -> JobRecord {
        JobRecord {
            class: PriorityClass::AgentExport,
            run: None,
            host: PortId(1),
            state,
            progress: 0.0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn ended_job_records_are_kept_for_the_last_few_hundred_jobs() {
        let mut st = State::default();
        st.jobs.insert("live".into(), record(JobState::Running));
        for i in 0..FINISHED_KEPT + 50 {
            let id = JobId::from(format!("job-{i}").as_str());
            st.jobs.insert(id.clone(), record(JobState::Done));
            st.retire(id);
        }
        assert_eq!(st.jobs.len(), FINISHED_KEPT + 1);
        assert!(!st.jobs.contains_key(&JobId::from("job-0")));
        assert!(st.jobs.contains_key(&JobId::from("job-50")));
        assert!(
            st.jobs.contains_key(&JobId::from("live")),
            "running jobs stay"
        );
        // A resubmitted id that runs again is not dropped by its old entry.
        let reused = JobId::from(format!("job-{}", 60).as_str());
        st.jobs.get_mut(&reused).unwrap().state = JobState::Running;
        for i in 0..FINISHED_KEPT {
            let id = JobId::from(format!("more-{i}").as_str());
            st.jobs.insert(id.clone(), record(JobState::Failed));
            st.retire(id);
        }
        assert!(st.jobs.contains_key(&reused));
    }
}
