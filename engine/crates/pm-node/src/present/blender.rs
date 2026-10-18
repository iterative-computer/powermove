//! The paused viewer's Blender shading (plan section 8, "Blender surfaces";
//! contract 11 over the host port; today's `renderBlenderPreview` and
//! `schedulePreview` in `core/scene3d/rendering.ts`).
//!
//! While the viewer is paused in Material Preview (EEVEE) or Rendered (the
//! composition's engine) shading, each frame is drawn with
//! [`BlenderRender::Preview`]: every composition with 3D objects builds its
//! preview snapshot (preview samples and scale), draws the surfaces this
//! module has for it, and draws natively while it reports the snapshots it
//! lacks. Playback and the draft shadings (Wireframe, Solid) draw natively,
//! as today.
//!
//! The snapshots a paused frame lacks become the view's wanted set. After
//! today's 250 ms debounce a worker thread renders them through the Blender
//! stage ([`BlenderStage`]: the surface cache by section 14.7 key, then
//! host services' supervised Blender for the missing ones, as the job
//! engine's exports do), keeps the decoded surfaces under today's 512 MB
//! preview budget, and asks the render thread to draw the view again. A
//! new paused frame that wants something else, a draft shading, or the
//! view closing cancels the job, which stops Blender. Host services pause
//! Blender while anything plays (contract 13), so a job that is running
//! when playback starts finishes after it stops.
//!
//! A failure (Blender missing, a failed job) is reported to the viewer as a
//! session failure and stops previews of that view until its shading
//! changes, as today's `error` stops `schedulePreview` until `setShading`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_blender::job::BlenderStage;
use pm_blender::snapshot::Snapshot;
use pm_blender::{BlenderError, DecodedSurface, SurfaceCache};
use pm_engine::{Action, LogLevel};
use pm_protocol::codec;
use pm_protocol::common::{JobId, Revision};
use pm_protocol::envelope::{Envelope, Event, Message};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope, FailureReport};
use pm_protocol::frame::{Shading3d, ViewOverride};
use pm_protocol::priority::PriorityClass;
use pm_render::blender::{PreviewSurfaces, SnapshotOptions};
use pm_render::scene3d::View3d;

use super::render::{Key, Sink, Stats};
use crate::jobs::blender::{HostBackend, HostLink, JobParams};

/// Today's `schedulePreview` delay after a change.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Decoded preview surfaces kept in memory (today's 512 MB budget of the
/// renderer's Blender frames).
pub const SURFACE_BYTES: u64 = 512 << 20;

/// Decoded surfaces the stage's own cache keeps between jobs (the preview
/// map holds what the viewer draws; the stage re-reads its disk area).
const STAGE_BYTES: u64 = 64 << 20;

/// The shading of a view override when it uses Blender.
pub fn blender_shading(view: Option<&ViewOverride>) -> Option<Shading3d> {
    match view.map(|v| v.shading) {
        Some(s @ (Shading3d::Material | Shading3d::Rendered)) => Some(s),
        _ => None,
    }
}

/// The snapshot options of a Blender shading: Material Preview renders
/// with EEVEE; the editor camera when the viewer sends one.
pub fn options_for(view: &ViewOverride) -> SnapshotOptions {
    SnapshotOptions {
        force_eevee: view.shading == Shading3d::Material,
        camera: View3d::from_override(view).camera,
        settings: None,
    }
}

/// Counters of the paused viewer's Blender work (in the presenter's stats).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlenderPreviewStats {
    /// Stage runs (one per wanted set).
    pub runs: u64,
    /// Blender requests sent to host services.
    pub jobs: u64,
    /// Surfaces Blender rendered, and those found in the surface cache.
    pub rendered: u64,
    pub cached: u64,
    pub cancelled: u64,
    pub failed: u64,
    pub last_error: Option<String>,
    /// The device the last Blender request resolved (`auto` is recorded per
    /// job).
    pub device: Option<String>,
    /// Decoded surfaces held, and their bytes.
    pub surfaces: u64,
    pub bytes: u64,
}

struct Rendered {
    layers: HashMap<String, Arc<DecodedSurface>>,
    used: u64,
    bytes: u64,
}

struct Want {
    snapshots: Vec<Snapshot>,
    hashes: Vec<[u8; 32]>,
    shading: Shading3d,
    revision: Revision,
    since: Instant,
}

struct Running {
    key: Key,
    hashes: Vec<[u8; 32]>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    surfaces: HashMap<[u8; 32], Rendered>,
    bytes: u64,
    tick: u64,
    wanted: BTreeMap<Key, Want>,
    running: Option<Running>,
    /// Views whose last job failed, with the shading it failed in.
    halted: HashMap<Key, Shading3d>,
    /// Host services do not run Blender (no Blender path to them).
    unsupported: bool,
    shutdown: bool,
}

struct Inner {
    state: Mutex<State>,
    work: Condvar,
    link: Arc<HostLink>,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    ready: Box<dyn Fn(Key) + Send + Sync>,
    budget: u64,
    debounce: Duration,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stats(&self, update: impl FnOnce(&mut BlenderPreviewStats)) {
        let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        update(&mut s.blender_preview);
    }

    fn log(&self, level: LogLevel, message: String) {
        (self.sink)(vec![Action::Log { level, message }]);
    }

    /// A session failure for the view (the viewer logs it; contract 11's
    /// codes).
    fn report(&self, key: Key, revision: Revision, error: &BlenderError) {
        let code = match error {
            BlenderError::Missing(_) => ErrorCode::BlenderMissing,
            BlenderError::Failed(_) => ErrorCode::BlenderFailed,
            BlenderError::Cancelled => return,
        };
        let error = EngineError::new(
            code,
            ErrorScope::Session { session: key.1 },
            error.to_string(),
        );
        let mut envelope = Envelope::new(Message::Event {
            body: Event::Failure(FailureReport { revision, error }),
        })
        .with_session(key.1);
        envelope.revision = Some(revision);
        if let Ok(bytes) = codec::encode(&envelope) {
            (self.sink)(vec![Action::Send { port: key.0, bytes }]);
        }
    }

    fn remember(
        &self,
        st: &mut State,
        hash: [u8; 32],
        layers: HashMap<String, Arc<DecodedSurface>>,
    ) {
        st.tick += 1;
        let bytes = layers.values().map(|s| s.bytes()).sum();
        if let Some(old) = st.surfaces.insert(
            hash,
            Rendered {
                layers,
                used: st.tick,
                bytes,
            },
        ) {
            st.bytes -= old.bytes;
        }
        st.bytes += bytes;
        // Least recently drawn first out; the frame just rendered stays.
        while st.bytes > self.budget && st.surfaces.len() > 1 {
            let Some(oldest) = st
                .surfaces
                .iter()
                .filter(|(h, _)| **h != hash)
                .min_by_key(|(_, r)| r.used)
                .map(|(h, _)| *h)
            else {
                break;
            };
            if let Some(r) = st.surfaces.remove(&oldest) {
                st.bytes -= r.bytes;
            }
        }
        let (n, b) = (
            st.surfaces.values().map(|r| r.layers.len() as u64).sum(),
            st.bytes,
        );
        self.stats(|s| {
            s.surfaces = n;
            s.bytes = b;
        });
    }
}

/// The paused viewer's Blender previews, shared by the presenter (which
/// routes host traffic to its [`HostLink`]) and the render thread.
pub struct PreviewBlender {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl PreviewBlender {
    /// Starts the worker. `ready` asks the render thread to draw a view
    /// again (its preview surfaces arrived).
    pub(crate) fn start(
        link: Arc<HostLink>,
        sink: Sink,
        stats: Arc<Mutex<Stats>>,
        ready: Box<dyn Fn(Key) + Send + Sync>,
    ) -> Result<Arc<PreviewBlender>, String> {
        Self::start_with(link, sink, stats, ready, SURFACE_BYTES, DEBOUNCE)
    }

    pub(crate) fn start_with(
        link: Arc<HostLink>,
        sink: Sink,
        stats: Arc<Mutex<Stats>>,
        ready: Box<dyn Fn(Key) + Send + Sync>,
        budget: u64,
        debounce: Duration,
    ) -> Result<Arc<PreviewBlender>, String> {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            work: Condvar::new(),
            link,
            sink,
            stats,
            ready,
            budget,
            debounce,
        });
        let worker = inner.clone();
        let thread = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Interactive,
            "pm-viewer-blender",
            move || run(worker),
        )
        .map_err(|e| format!("could not start the viewer's Blender worker: {e}"))?;
        Ok(Arc::new(PreviewBlender {
            inner,
            thread: Mutex::new(Some(thread)),
        }))
    }

    /// The preview host of one paused render of `key` in `view`'s Blender
    /// shading; hand its wanted snapshots to [`Self::submit`] afterwards.
    pub fn host(&self, view: &ViewOverride) -> Arc<ViewHost> {
        Arc::new(ViewHost {
            inner: self.inner.clone(),
            options: options_for(view),
            wanted: Mutex::new(Vec::new()),
            hashes: Mutex::new(HashMap::new()),
        })
    }

    /// What a paused render of `key` wanted (`shading` `None`: it drew in a
    /// draft shading, so nothing is wanted). Replacing a running job's set
    /// cancels it.
    pub fn submit(
        &self,
        key: Key,
        shading: Option<Shading3d>,
        revision: Revision,
        host: Option<&ViewHost>,
    ) {
        let mut st = self.inner.lock();
        let snapshots = match (shading, host) {
            (Some(shading), Some(host)) => {
                if st.halted.get(&key).is_some_and(|s| *s != shading) {
                    // `setShading` clears today's error.
                    st.halted.remove(&key);
                }
                if st.halted.contains_key(&key) || st.unsupported {
                    Vec::new()
                } else {
                    host.take_wanted()
                }
            }
            _ => {
                st.halted.remove(&key);
                Vec::new()
            }
        };
        let hashes: Vec<[u8; 32]> = snapshots.iter().map(|s| s.hash().0).collect();
        let cancel_running = st
            .running
            .as_ref()
            .is_some_and(|r| r.key == key && !r.hashes.iter().all(|h| hashes.contains(h)));
        if cancel_running && let Some(r) = &st.running {
            r.cancel.store(true, Ordering::Relaxed);
        }
        if snapshots.is_empty() {
            st.wanted.remove(&key);
        } else {
            let same = st.wanted.get(&key).is_some_and(|w| w.hashes == hashes);
            let running_same = st
                .running
                .as_ref()
                .is_some_and(|r| r.key == key && r.hashes == hashes);
            if running_same {
                st.wanted.remove(&key);
            } else if !same {
                st.wanted.insert(
                    key,
                    Want {
                        snapshots,
                        hashes,
                        shading: shading.unwrap_or(Shading3d::Rendered),
                        revision,
                        since: Instant::now(),
                    },
                );
            }
        }
        drop(st);
        self.inner.work.notify_all();
    }

    /// The view or its session went away: its wanted set goes and a job it
    /// runs is cancelled.
    pub fn forget(&self, key: Key) {
        let mut st = self.inner.lock();
        st.wanted.remove(&key);
        st.halted.remove(&key);
        if let Some(r) = st.running.as_ref().filter(|r| r.key == key) {
            r.cancel.store(true, Ordering::Relaxed);
        }
        drop(st);
        self.inner.work.notify_all();
    }

    /// Whether a job runs or waits (tests).
    pub fn busy(&self) -> bool {
        let st = self.inner.lock();
        st.running.is_some() || !st.wanted.is_empty()
    }

    /// Stops the worker: a running job is cancelled.
    pub fn shutdown(&self) {
        {
            let mut st = self.inner.lock();
            st.shutdown = true;
            if let Some(r) = &st.running {
                r.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.inner.work.notify_all();
        self.inner.link.shutdown();
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for PreviewBlender {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The preview surfaces of one paused render ([`PreviewSurfaces`]).
pub struct ViewHost {
    inner: Arc<Inner>,
    options: SnapshotOptions,
    wanted: Mutex<Vec<Snapshot>>,
    /// Snapshot hashes by address: a render looks each snapshot up once
    /// per layer, and hashing one covers all of its geometry.
    hashes: Mutex<HashMap<usize, [u8; 32]>>,
}

impl ViewHost {
    fn hash(&self, snapshot: &Snapshot) -> [u8; 32] {
        let address = snapshot as *const Snapshot as usize;
        *self
            .hashes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(address)
            .or_insert_with(|| snapshot.hash().0)
    }

    fn take_wanted(&self) -> Vec<Snapshot> {
        std::mem::take(&mut *self.wanted.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl PreviewSurfaces for ViewHost {
    fn options(&self) -> SnapshotOptions {
        self.options.clone()
    }

    fn lookup(&self, snapshot: &Snapshot, layer: &str) -> Option<Arc<DecodedSurface>> {
        let hash = self.hash(snapshot);
        let mut st = self.inner.lock();
        st.tick += 1;
        let tick = st.tick;
        let rendered = st.surfaces.get_mut(&hash)?;
        rendered.used = tick;
        rendered.layers.get(layer).cloned()
    }

    fn wanted(&self, snapshot: Snapshot) {
        self.wanted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(snapshot);
    }
}

/// The worker's Blender stage and backend, made with the first job (the
/// stage's cache lives in the Blender Surfaces area host services name).
struct Worker {
    params: Arc<Mutex<JobParams>>,
    backend: HostBackend<HostLink>,
    stage: Option<BlenderStage>,
}

fn run(inner: Arc<Inner>) {
    let params = Arc::new(Mutex::new(JobParams {
        session: pm_protocol::common::SessionId::new(1).expect("1 is a session id"),
        priority: PriorityClass::Interactive,
        export: JobId::new("preview"),
    }));
    let mut worker = Worker {
        backend: HostBackend::new(inner.link.clone(), params.clone()),
        params,
        stage: None,
    };
    loop {
        // The next due wanted set (oldest first).
        let picked = {
            let mut st = inner.lock();
            loop {
                if st.shutdown {
                    return;
                }
                let now = Instant::now();
                let due = st
                    .wanted
                    .iter()
                    .min_by_key(|(_, w)| w.since)
                    .map(|(k, w)| (*k, w.since + inner.debounce));
                match due {
                    Some((key, at)) if at <= now => {
                        let want = st.wanted.remove(&key).expect("found above");
                        let cancel = Arc::new(AtomicBool::new(false));
                        st.running = Some(Running {
                            key,
                            hashes: want.hashes.clone(),
                            cancel: cancel.clone(),
                        });
                        break (key, want, cancel);
                    }
                    Some((_, at)) => {
                        st = inner
                            .work
                            .wait_timeout(st, at - now)
                            .map(|(g, _)| g)
                            .unwrap_or_else(|e| e.into_inner().0);
                    }
                    None => {
                        st = inner.work.wait(st).unwrap_or_else(|e| e.into_inner());
                    }
                }
            }
        };
        let (key, want, cancel) = picked;
        let result = render(&inner, &mut worker, key, &want, &cancel);
        let mut st = inner.lock();
        st.running = None;
        match result {
            Ok(frames) => {
                for (hash, layers) in frames {
                    inner.remember(&mut st, hash, layers);
                }
                drop(st);
                (inner.ready)(key);
            }
            Err(BlenderError::Cancelled) => {
                drop(st);
                inner.stats(|s| s.cancelled += 1);
            }
            Err(error) => {
                // A host without Blender stops every view (`unsupported`).
                if !st.unsupported {
                    st.halted.insert(key, want.shading);
                }
                st.wanted.remove(&key);
                drop(st);
                let message = error.to_string();
                inner.stats(|s| {
                    s.failed += 1;
                    s.last_error = Some(message.clone());
                });
                inner.log(
                    LogLevel::Warn,
                    format!("viewer: the Blender preview failed: {message}"),
                );
                inner.report(key, want.revision, &error);
            }
        }
    }
}

type Frames = Vec<([u8; 32], HashMap<String, Arc<DecodedSurface>>)>;

fn render(
    inner: &Inner,
    worker: &mut Worker,
    key: Key,
    want: &Want,
    cancel: &AtomicBool,
) -> Result<Frames, BlenderError> {
    *worker.params.lock().unwrap_or_else(|e| e.into_inner()) = JobParams {
        session: key.1,
        priority: PriorityClass::Interactive,
        export: JobId::new(format!("preview-{}-{}", key.0.0, key.1.get())),
    };
    inner.stats(|s| s.runs += 1);
    if worker.stage.is_none() {
        // A host without a Blender path answers `unsupported`: previews
        // stay native for this launch.
        match blender_status(&inner.link, key.1) {
            Ok(()) => {}
            Err(e) => {
                if matches!(e, StatusError::Unsupported) {
                    inner.lock().unsupported = true;
                }
                return Err(e.into());
            }
        }
        let (area, _) = worker.backend.area()?;
        let cache = SurfaceCache::new(STAGE_BYTES, Some(area.join("surfaces")));
        worker.stage = Some(BlenderStage::new(cache));
    }
    let stage = worker.stage.as_mut().expect("made above");
    let before = (stage.stats, stage.devices.len());
    let result = stage.prepare(&mut worker.backend, &want.snapshots, cancel, &mut |_, _| {});
    let after = stage.stats;
    let device = stage.devices[before.1..].last().cloned();
    inner.stats(|s| {
        s.jobs += after.jobs - before.0.jobs;
        s.rendered += after.rendered - before.0.rendered;
        s.cached += after.cached - before.0.cached;
        if let Some(d) = device {
            s.device = Some(format!("{d:?}"));
        }
    });
    let surfaces = result?;
    let mut out = Vec::new();
    for (snapshot, hash) in want.snapshots.iter().zip(&want.hashes) {
        let layers = surfaces
            .get(&snapshot.frame.lookup())
            .cloned()
            .unwrap_or_default();
        let missing: HashSet<String> = snapshot
            .layers()
            .into_iter()
            .filter(|l| !layers.contains_key(l))
            .collect();
        if !missing.is_empty() {
            return Err(BlenderError::Failed(format!(
                "Blender did not render {} layer(s) of the preview",
                missing.len()
            )));
        }
        out.push((*hash, layers));
    }
    Ok(out)
}

/// Why Blender status failed.
enum StatusError {
    /// Host services do not run Blender.
    Unsupported,
    Missing(String),
    Failed(String),
}

impl From<StatusError> for BlenderError {
    fn from(e: StatusError) -> BlenderError {
        match e {
            StatusError::Unsupported => {
                BlenderError::Failed("Host services do not run Blender for the viewer.".into())
            }
            StatusError::Missing(m) => BlenderError::Missing(m),
            StatusError::Failed(m) => BlenderError::Failed(m),
        }
    }
}

/// `blenderStatus`: Ok when a supported Blender is there.
fn blender_status(
    link: &HostLink,
    session: pm_protocol::common::SessionId,
) -> Result<(), StatusError> {
    use crate::jobs::blender::HostPort;
    use pm_protocol::envelope::{Request, Response};
    {
        match link.call(Request::BlenderStatus, Some(session)) {
            Ok(Response::BlenderStatus(s)) if s.supported() => Ok(()),
            Ok(Response::BlenderStatus(s)) => Err(StatusError::Missing(
                s.error
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| pm_blender::BLENDER_MISSING_MESSAGE.to_owned()),
            )),
            Ok(_) => Err(StatusError::Failed(
                "unexpected Blender status answer".into(),
            )),
            Err(e) if e.code == ErrorCode::Unsupported => Err(StatusError::Unsupported),
            Err(e) if e.code == ErrorCode::BlenderMissing => Err(StatusError::Missing(e.message)),
            Err(e) => Err(StatusError::Failed(e.message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;

    use pm_blender::key::hex;
    use pm_blender::snapshot::BlenderFrame;
    use pm_blender::surface::encode_rgba16;
    use pm_protocol::blender::{BlenderStatus, BlenderSurface, ResolvedDevice};
    use pm_protocol::common::{ByteSize, PixelRect, SessionId, Size};
    use pm_protocol::envelope::{Request, Response};
    use pm_protocol::host::{CapabilityHandle, HandleAccess, HandleGrant, HandleResolved, PortId};
    use pm_protocol::priority::{CancelJobs, JobsCancelled};

    const KEY: Key = (
        PortId(3),
        match SessionId::new(7) {
            Ok(s) => s,
            Err(_) => panic!(),
        },
    );

    /// How the stand-in host answers `blenderRender`.
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Render,
        /// Holds every render until it is cancelled.
        Hold,
        Missing,
    }

    #[derive(Default)]
    struct Seen {
        renders: Vec<(JobId, usize)>,
        cancels: Vec<JobId>,
        statuses: u32,
    }

    /// Host services in a thread: the Blender Surfaces area in a temporary
    /// directory, Blender's surfaces written as 16-bit PNGs before the
    /// answer, cancellation as main does it.
    struct Host {
        link: Arc<HostLink>,
        seen: Arc<Mutex<Seen>>,
        mode: Arc<Mutex<Mode>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        area: PathBuf,
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn area() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pm-viewer-blender-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(w: u32, h: u32, seed: u16) -> Vec<u8> {
        let mut rgba = vec![0u16; (w * h * 4) as usize];
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = ((y * w + x) * 4) as usize;
                rgba[i] = seed.wrapping_mul(97).wrapping_add(x as u16 * 1000);
                rgba[i + 1] = 30_000;
                rgba[i + 2] = y as u16 * 2000;
                rgba[i + 3] = 65_535;
            }
        }
        encode_rgba16(w, h, &rgba, &[])
    }

    fn host(link: Arc<HostLink>, area: PathBuf, mode: Mode) -> Host {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mode = Arc::new(Mutex::new(mode));
        let stop = Arc::new(AtomicBool::new(false));
        let (l, s, m, st, a) = (
            link.clone(),
            seen.clone(),
            mode.clone(),
            stop.clone(),
            area.clone(),
        );
        let thread = std::thread::spawn(move || {
            let handle = CapabilityHandle::from_random_bytes([9; 16]);
            let mut held: Vec<(u64, JobId)> = Vec::new();
            let mut seed = 0u16;
            while !st.load(Ordering::Relaxed) {
                for call in l.drain() {
                    let answer = match call.request {
                        Request::BlenderStatus => {
                            s.lock().unwrap().statuses += 1;
                            let missing = *m.lock().unwrap() == Mode::Missing;
                            Ok(Response::BlenderStatus(BlenderStatus {
                                available: !missing,
                                version: (!missing).then(|| "4.5.14".into()),
                                error: missing.then(|| "Install Blender 4.5 or later.".into()),
                            }))
                        }
                        Request::MintHandle(mint) => Ok(Response::Handle(HandleGrant {
                            handle: handle.clone(),
                            kind: mint.kind,
                            content_hash: None,
                        })),
                        Request::ResolveHandle(r) => Ok(Response::HandleResolved(HandleResolved {
                            handle: r.handle,
                            kind: pm_protocol::host::CapabilityKind::Cache {
                                cache: pm_protocol::host::CacheKind::BlenderSurfaces,
                            },
                            access: HandleAccess::ReadWrite,
                            path: a.to_string_lossy().into_owned(),
                        })),
                        Request::BlenderRender(job) => {
                            s.lock()
                                .unwrap()
                                .renders
                                .push((job.job.clone(), job.surfaces.len()));
                            if *m.lock().unwrap() == Mode::Hold {
                                held.push((call.token, job.job.clone()));
                                continue;
                            }
                            for (i, sr) in job.surfaces.iter().enumerate() {
                                seed += 1;
                                let bytes = png(sr.size.width, sr.size.height, seed);
                                std::fs::write(a.join(format!("{}.png", hex(&sr.key))), &bytes)
                                    .unwrap();
                                l.event(&Event::BlenderSurface(BlenderSurface {
                                    job: job.job.clone(),
                                    request_index: i as u32,
                                    key: sr.key,
                                    size: sr.size,
                                    crop: PixelRect {
                                        x: 0,
                                        y: 0,
                                        width: sr.size.width,
                                        height: sr.size.height,
                                    },
                                    handle: job.output.clone(),
                                    bytes: ByteSize::new(bytes.len() as u64).unwrap(),
                                    device: ResolvedDevice::Cpu,
                                }));
                            }
                            Ok(Response::Ack)
                        }
                        Request::CancelJobs(CancelJobs::Job { job }) => {
                            s.lock().unwrap().cancels.push(job.clone());
                            if let Some(i) = held.iter().position(|(_, j)| *j == job) {
                                let (token, _) = held.remove(i);
                                l.answer(
                                    token,
                                    Err(EngineError::new(
                                        ErrorCode::Cancelled,
                                        ErrorScope::Connection,
                                        "Blender request cancelled",
                                    )),
                                );
                            }
                            Ok(Response::JobsCancelled(JobsCancelled { jobs: vec![job] }))
                        }
                        other => panic!("unexpected host call {other:?}"),
                    };
                    l.answer(call.token, answer);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        Host {
            link,
            seen,
            mode,
            stop,
            thread: Some(thread),
            area,
        }
    }

    fn snapshot(layers: &[&str], time: f64) -> Snapshot {
        let objects: Vec<String> = layers
            .iter()
            .map(|id| format!(r#"{{"id":"{id}","meshes":[]}}"#))
            .collect();
        let json = pm_model::json::parse(&format!(
            r#"{{"width":12,"height":8,"settings":{{"enabled":false,"engine":"eevee","samples":4,"previewSamples":4,"previewScale":0.5,"denoise":true,"device":"cpu"}},"objects":[{}],"lights":[],"camera":{{"matrix":[{time}]}}}}"#,
            objects.join(",")
        ))
        .unwrap();
        Snapshot {
            frame: BlenderFrame {
                composition_path: vec!["c".into()],
                time,
                width: 24,
                height: 16,
                shutter: None,
                preview: true,
            },
            json,
            assets: Vec::new(),
        }
    }

    struct Rig {
        blender: Arc<PreviewBlender>,
        ready: Arc<Mutex<Vec<Key>>>,
        sent: Arc<Mutex<Vec<Envelope>>>,
        stats: Arc<Mutex<Stats>>,
        host: Host,
    }

    fn rig(mode: Mode, area_dir: Option<PathBuf>, budget: u64) -> Rig {
        let link = HostLink::new(Arc::new(|| {}));
        let ready = Arc::new(Mutex::new(Vec::new()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let r = ready.clone();
        let s = sent.clone();
        let sink: Sink = Arc::new(move |actions| {
            for a in actions {
                if let Action::Send { bytes, .. } = a {
                    s.lock().unwrap().push(codec::decode(&bytes).unwrap());
                }
            }
        });
        let blender = PreviewBlender::start_with(
            link.clone(),
            sink,
            stats.clone(),
            Box::new(move |key| r.lock().unwrap().push(key)),
            budget,
            Duration::from_millis(20),
        )
        .unwrap();
        let host = host(link, area_dir.unwrap_or_else(area), mode);
        Rig {
            blender,
            ready,
            sent,
            stats,
            host,
        }
    }

    fn rendered(view: &ViewOverride) -> ViewOverride {
        ViewOverride {
            shading: Shading3d::Rendered,
            ..view.clone()
        }
    }

    fn material() -> ViewOverride {
        ViewOverride {
            camera: None,
            shading: Shading3d::Material,
            xray: false,
            local_view: Vec::new(),
        }
    }

    /// One paused render of `snapshots` in `view`: what it draws, as the
    /// renderer would ask (lookups, then the ones it lacks).
    fn paint(
        r: &Rig,
        view: &ViewOverride,
        snapshots: &[Snapshot],
    ) -> Vec<Option<Arc<DecodedSurface>>> {
        let host = r.blender.host(view);
        let mut out = Vec::new();
        for s in snapshots {
            let found: Vec<_> = s.layers().iter().map(|l| host.lookup(s, l)).collect();
            if found.iter().any(Option::is_none) {
                host.wanted(s.clone());
            }
            out.extend(found);
        }
        r.blender.submit(
            KEY,
            blender_shading(Some(view)),
            Revision::ZERO,
            Some(&host),
        );
        out
    }

    fn wait(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_paused_frame_asks_once_draws_what_comes_back_and_reuses_the_disk_cache() {
        let r = rig(Mode::Render, None, SURFACE_BYTES);
        let a = snapshot(&["box", "ball"], 0.0);
        assert!(
            paint(&r, &material(), std::slice::from_ref(&a))
                .iter()
                .all(Option::is_none)
        );
        // Painted again before the debounce: still one request.
        paint(&r, &material(), std::slice::from_ref(&a));
        wait("surfaces", || !r.ready.lock().unwrap().is_empty());
        let drawn = paint(&r, &material(), std::slice::from_ref(&a));
        assert!(drawn.iter().all(Option::is_some));
        let s = drawn[0].as_ref().unwrap();
        // Decoded at the snapshot's size, cropped to its content.
        assert_eq!(s.size, Size::new(12, 8));
        assert_eq!(
            s.crop,
            PixelRect {
                x: 0,
                y: 0,
                width: 12,
                height: 8
            }
        );
        let seen = r.host.seen.lock().unwrap();
        assert_eq!(seen.renders.len(), 1, "one request for the frame's layers");
        assert_eq!(seen.renders[0].1, 2);
        assert!(
            seen.renders[0]
                .0
                .as_str()
                .starts_with("preview-3-7-blender-")
        );
        drop(seen);
        let stats = r.stats.lock().unwrap().blender_preview.clone();
        assert_eq!((stats.jobs, stats.rendered, stats.cached), (1, 2, 0));
        assert_eq!(stats.device.as_deref(), Some("Cpu"));
        assert!(!r.blender.busy());
        assert_eq!(r.host.link.pending(), (0, 0, 0), "nothing left open");

        // A new launch (empty memory) finds the surfaces in the disk area.
        let area = r.host.area.clone();
        let again = rig(Mode::Render, Some(area), SURFACE_BYTES);
        paint(&again, &material(), std::slice::from_ref(&a));
        wait("cached", || !again.ready.lock().unwrap().is_empty());
        assert!(again.host.seen.lock().unwrap().renders.is_empty());
        let stats = again.stats.lock().unwrap().blender_preview.clone();
        assert_eq!((stats.jobs, stats.cached), (0, 2));
        assert!(
            paint(&again, &material(), std::slice::from_ref(&a))
                .iter()
                .all(Option::is_some)
        );
        // Rendered shading of the same snapshot is the same key: cached.
        let view = rendered(&material());
        assert!(
            paint(&again, &view, std::slice::from_ref(&a))
                .iter()
                .all(Option::is_some)
        );
    }

    #[test]
    fn a_new_frame_or_a_draft_shading_cancels_the_running_job_and_stops_blender() {
        let r = rig(Mode::Hold, None, SURFACE_BYTES);
        let a = snapshot(&["box"], 0.0);
        let b = snapshot(&["box"], 1.0);
        paint(&r, &material(), std::slice::from_ref(&a));
        wait("first job", || {
            r.host.seen.lock().unwrap().renders.len() == 1
        });
        // The playhead moved while paused: the job for frame a is cancelled.
        paint(&r, &material(), std::slice::from_ref(&b));
        wait("cancel", || r.host.seen.lock().unwrap().cancels.len() == 1);
        wait("second job", || {
            r.host.seen.lock().unwrap().renders.len() == 2
        });
        {
            let seen = r.host.seen.lock().unwrap();
            assert_eq!(seen.cancels[0], seen.renders[0].0);
        }
        // Solid shading: nothing is wanted, the job for b stops too.
        let solid = ViewOverride {
            shading: Shading3d::Solid,
            ..material()
        };
        r.blender
            .submit(KEY, blender_shading(Some(&solid)), Revision::ZERO, None);
        wait("second cancel", || {
            r.host.seen.lock().unwrap().cancels.len() == 2
        });
        wait("idle", || !r.blender.busy());
        assert_eq!(r.stats.lock().unwrap().blender_preview.cancelled, 2);
        assert!(r.ready.lock().unwrap().is_empty());
        // The view closing cancels as well.
        paint(&r, &material(), std::slice::from_ref(&a));
        wait("third job", || {
            r.host.seen.lock().unwrap().renders.len() == 3
        });
        r.blender.forget(KEY);
        wait("third cancel", || {
            r.host.seen.lock().unwrap().cancels.len() == 3
        });
        wait("idle again", || !r.blender.busy());
        assert_eq!(r.host.link.pending().2, 0, "no Blender job left open");
    }

    #[test]
    fn a_missing_blender_is_reported_once_until_the_shading_changes() {
        let r = rig(Mode::Missing, None, SURFACE_BYTES);
        let a = snapshot(&["box"], 0.0);
        paint(&r, &material(), std::slice::from_ref(&a));
        wait("failure", || !r.sent.lock().unwrap().is_empty());
        {
            let sent = r.sent.lock().unwrap();
            let Message::Event {
                body: Event::Failure(report),
            } = &sent[0].message
            else {
                panic!("{:?}", sent[0]);
            };
            assert_eq!(report.error.code, ErrorCode::BlenderMissing);
            assert_eq!(report.error.scope, ErrorScope::Session { session: KEY.1 });
            assert_eq!(sent[0].session, Some(KEY.1));
        }
        // Painting again in the same shading asks nothing (today's `error`).
        paint(&r, &material(), std::slice::from_ref(&a));
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(r.host.seen.lock().unwrap().statuses, 1);
        // Blender installed and the shading changed: it renders.
        *r.host.mode.lock().unwrap() = Mode::Render;
        paint(&r, &rendered(&material()), std::slice::from_ref(&a));
        wait("surfaces", || !r.ready.lock().unwrap().is_empty());
        assert_eq!(r.sent.lock().unwrap().len(), 1);
        assert_eq!(r.stats.lock().unwrap().blender_preview.failed, 1);
    }

    #[test]
    fn surfaces_stay_within_the_budget_least_recently_drawn_first_out() {
        // One 12 × 8 f32 surface is 1,536 bytes: room for two frames.
        let r = rig(Mode::Render, None, 3_500);
        let frames: Vec<Snapshot> = (0..3).map(|i| snapshot(&["box"], f64::from(i))).collect();
        for (i, f) in frames.iter().enumerate() {
            paint(&r, &material(), std::slice::from_ref(f));
            wait("frame", || r.ready.lock().unwrap().len() == i + 1);
            if i == 1 {
                // Frame 0 drawn again: frame 1 is now the oldest.
                assert!(paint(&r, &material(), &frames[..1])[0].is_some());
            }
        }
        let stats = r.stats.lock().unwrap().blender_preview.clone();
        assert_eq!(stats.surfaces, 2);
        assert!(stats.bytes <= 3_500);
        let host = r.blender.host(&material());
        assert!(host.lookup(&frames[0], "box").is_some());
        assert!(host.lookup(&frames[1], "box").is_none());
        assert!(host.lookup(&frames[2], "box").is_some());
    }
}
