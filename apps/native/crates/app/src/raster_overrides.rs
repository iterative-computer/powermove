//! Raster overrides between the engine and a window's extension realm. The
//! viewer's frames ask the engine's `CompatStore` for each layer today's
//! compositor rasterizes; what it hasn't answered goes to the realm in a
//! batch, and the realm's answers (an override's canvas, or a pass-through)
//! come back to the store, which redraws. Base rasters (`render.raster`) are
//! painted by the engine's own raster worker. Nothing here runs on the main
//! thread or waits on the viewer.
use pm_compat_runner::host::RasterInbox;
use pm_compat_runner::{BaseRasterReply, BaseRasterRequest, RasterAnswer, RasterHooks};
use pm_engine_service::EngineService;
use pm_engine_service::compat::{
    BaseRequest, CompatAnswer, CompatStore, Reply, hidden_raster, override_raster_window,
};
use pm_engine_service::jobs::{CompatJobNeed, CompatReply, JobSession, OverrideAnswers};
use pm_kernel::{KernelHandle, Published};
use pm_protocol::common::{PixelRect, TICKS_PER_SECOND};
use pm_protocol::snapshot::CompatNeed;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Who a batch's answers are for.
enum Asker {
    /// The viewer, at a document revision and wrapper generation.
    Viewer(u64, u64, Vec<CompatNeed>),
    /// An export frame, waiting on its job's reader thread.
    Export(Sender<Vec<RasterAnswer>>),
}
/// Batches asked of the realm and not answered.
#[derive(Default)]
struct Batches {
    next: u64,
    open: HashMap<u64, (Instant, Asker)>,
}
impl Batches {
    /// A new batch. Viewer batches the realm never answered (it was busy
    /// being reloaded, or stopped) retire: their needs are asked again.
    fn open(&mut self, asker: Asker) -> u64 {
        if self.open.len() > 256 {
            self.open
                .retain(|_, (at, asker)| matches!(asker, Asker::Export(_)) || at.elapsed() < STALE);
        }
        self.next += 1;
        self.open.insert(self.next, (Instant::now(), asker));
        self.next
    }
}
const STALE: Duration = Duration::from_secs(10);
/// How long a one-frame look waits for the window's realm.
const EXPORT_WAIT: Duration = Duration::from_secs(60);
/// An export waits for its extensions as long as they make progress (a
/// script runs, an extension starts, an answer lands): a loaded machine is
/// slow, not stopped. This long with no step at all, they have stopped.
const EXPORT_STALL: Duration = Duration::from_secs(120);
/// What an export is waiting for while its extensions are slow to answer
/// (`waiting for …`), for the export sheet; `None` while nothing is late.
pub(crate) type ExportWaiting = Arc<Mutex<Option<String>>>;
/// Past this the sheet says what the export waits for.
const WAITING_SHOWN: Duration = Duration::from_secs(2);
/// When an export's extensions last made progress (a step of
/// `RasterInbox::export_progress`), against [`EXPORT_STALL`].
struct Progress {
    steps: Option<u64>,
    at: Instant,
}
impl Progress {
    fn new(now: Instant) -> Self {
        Self { steps: None, at: now }
    }
    /// The realm has taken `steps` by `now`: whether nothing moved for `stall`.
    fn stalled(&mut self, steps: u64, now: Instant, stall: Duration) -> bool {
        if self.steps != Some(steps) {
            self.steps = Some(steps);
            self.at = now;
        }
        now.duration_since(self.at) >= stall
    }
}

pub(crate) struct Overrides {
    rasters: Option<Arc<pm_engine_service::compat::LayerRasters>>,
    jobs: JobOverrides,
    windows: OverrideWindows,
}
/// What answers the raster overrides of this window's project for the jobs
/// started from it (exports, renders, agents' captures): cloned to each.
#[derive(Clone)]
pub(crate) struct JobOverrides {
    store: Arc<CompatStore>,
    kernel: KernelHandle,
    project: String,
    /// The document this window's extensions answer for.
    doc_id: u64,
    batches: Arc<Mutex<Batches>>,
    realm: Arc<Mutex<Option<RasterInbox>>>,
}
/// Each open document's [`JobOverrides`], for the jobs agents start: an
/// agent names a project, not a window. A project no window has open is
/// answered by the host its jobs run ([`Overrides::windowless`]).
#[derive(Clone, Default)]
pub struct OverrideWindows(
    Arc<Mutex<HashMap<u64, JobOverrides>>>,
    Arc<std::sync::OnceLock<Painters>>,
);
/// What a host without a window paints base rasters and canvas text with.
#[derive(Clone)]
pub(crate) struct Painters {
    pub layers: Option<Arc<pm_engine_service::compat::LayerRasters>>,
    pub fonts: pm_text::host::FontBroker,
}
impl OverrideWindows {
    pub(crate) fn of(&self, doc_id: u64) -> Option<JobOverrides> {
        self.0.lock().unwrap().get(&doc_id).cloned()
    }
    /// The app's own base raster worker and fonts: the ones its windows use.
    pub(crate) fn set_painters(&self, painters: Painters) {
        let _ = self.1.set(painters);
    }
    /// The app's, or with no app (the MCP process) ones made on first use.
    pub(crate) fn painters(&self) -> Painters {
        self.1
            .get_or_init(|| {
                let render = crate::text_host::render_fonts().unwrap_or_else(|| {
                    pm_render::text::host::RenderFonts::new(pm_text::host::FontBroker::new())
                });
                let fonts = render.broker.clone();
                Painters {
                    layers: pm_engine_service::compat::LayerRasters::start(Some(render)).ok(),
                    fonts,
                }
            })
            .clone()
    }
}

fn answer_of(answer: RasterAnswer) -> Reply {
    match answer {
        RasterAnswer::Native => Reply::Draw(CompatAnswer::Native),
        RasterAnswer::Hidden => Reply::Draw(hidden_raster()),
        RasterAnswer::Retry | RasterAnswer::Moved => Reply::Retry,
        RasterAnswer::Failed(message) => Reply::Failed(message),
        RasterAnswer::Unavailable(message) => Reply::Unavailable(message),
        RasterAnswer::Raster {
            width,
            height,
            premultiplied_rgba,
            size,
            anchor,
            uv,
            window,
        } => Reply::Draw(override_raster_window(
            width,
            height,
            window,
            premultiplied_rgba,
            size,
            anchor,
            uv,
        )),
    }
}

impl Overrides {
    pub(crate) fn new(
        engine: &EngineService,
        kernel: KernelHandle,
        project: String,
        windows: OverrideWindows,
    ) -> Self {
        let doc_id = kernel.published(&project).map_or(0, |p| p.snapshot.doc_id);
        let jobs = JobOverrides {
            store: engine.compat(doc_id),
            doc_id,
            kernel,
            project,
            batches: Arc::default(),
            realm: Arc::default(),
        };
        windows.0.lock().unwrap().insert(doc_id, jobs.clone());
        Self {
            rasters: engine.layer_rasters(),
            jobs,
            windows,
        }
    }
    /// For a project no window has open: the jobs an agent starts are
    /// answered by this host's extensions exactly as a window's would be.
    /// Its state is its own (no viewer asks it), and a window that opens
    /// the project takes its place.
    pub(crate) fn windowless(
        kernel: KernelHandle,
        project: String,
        windows: OverrideWindows,
    ) -> Self {
        let doc_id = kernel.published(&project).map_or(0, |p| p.snapshot.doc_id);
        let overrides = Self {
            rasters: windows.painters().layers,
            jobs: JobOverrides {
                store: Arc::default(),
                doc_id,
                kernel,
                project,
                batches: Arc::default(),
                realm: Arc::default(),
            },
            windows,
        };
        overrides.register();
        overrides
    }
    /// Answers the project's jobs unless a window does (again, after a
    /// window that had taken over has closed).
    pub(crate) fn register(&self) {
        let mut windows = self.windows.0.lock().unwrap();
        windows
            .entry(self.jobs.doc_id)
            .or_insert_with(|| self.jobs.clone());
    }
    pub(crate) fn jobs(&self) -> JobOverrides {
        self.jobs.clone()
    }
    pub(crate) fn hooks(&self) -> RasterHooks {
        let rasters = self.rasters.clone();
        // An export realm's layers paint from its frozen document.
        let base = Arc::new(move |request: BaseRasterRequest| {
            let rasters = rasters
                .as_ref()
                .ok_or("This window has no layer raster worker")?;
            let started = std::time::Instant::now();
            let painted = rasters.raster(BaseRequest {
                document: request.document,
                composition: request.composition,
                layer: request.layer,
                content: request.content,
                seconds: request.seconds,
                scale: request.scale,
                crop: request.crop.map(|[x, y, width, height]| PixelRect {
                    x,
                    y,
                    width,
                    height,
                }),
                window: request.window.map(|[x, y, width, height]| PixelRect {
                    x,
                    y,
                    width,
                    height,
                }),
            })?;
            let ms = started.elapsed().as_millis() as u64;
            if ms >= 20 {
                crate::diag::line(json!({"rasters": "base", "ms": ms}));
            }
            Ok(painted.map(|r| BaseRasterReply {
                width: r.width,
                height: r.height,
                straight_rgba: r.straight_rgba,
                size: r.box_size,
                anchor: r.anchor,
                selection: r.selection,
                font_offset: r.font_offset,
                uv: r.uv,
                window: r.window,
            }))
        });
        let (store, batches) = (self.jobs.store.clone(), self.jobs.batches.clone());
        let answers = Arc::new(move |batch: u64, answers: Vec<RasterAnswer>| {
            let asked = batches.lock().unwrap().open.remove(&batch);
            let (revision, epoch, needs) = match asked {
                Some((_, Asker::Viewer(revision, epoch, needs))) => (revision, epoch, needs),
                Some((_, Asker::Export(reply))) => {
                    let _ = reply.send(answers);
                    return;
                }
                None => return,
            };
            if crate::diag::enabled() {
                let kinds: Vec<&str> = answers
                    .iter()
                    .map(|a| match a {
                        RasterAnswer::Native => "native",
                        RasterAnswer::Hidden => "hidden",
                        RasterAnswer::Retry => "retry",
                        RasterAnswer::Moved => "moved",
                        RasterAnswer::Failed(_) => "failed",
                        RasterAnswer::Unavailable(_) => "unavailable",
                        RasterAnswer::Raster { .. } => "raster",
                    })
                    .collect();
                crate::diag::line(json!({"rasters": "answer", "batch": batch, "kinds": kinds}));
            }
            let answered = needs
                .into_iter()
                .zip(answers.into_iter().map(answer_of))
                .collect();
            store.answer_all(revision, epoch, answered);
        });
        let (store, kernel, project) = (
            self.jobs.store.clone(),
            self.jobs.kernel.clone(),
            self.jobs.project.clone(),
        );
        let producing = Arc::new(move |on: bool, epoch: u64, code: u64| {
            let revision = kernel.published(&project).map_or(0, |p| p.revision());
            // The disk render cache keys override frames by the compiled
            // records the wrappers run (the ones export realms pin).
            store.set_code(code);
            // Jobs started while an override is registered ask this realm.
            store.set_state(on, revision, epoch);
        });
        RasterHooks {
            base,
            answers,
            producing,
        }
    }
    /// The store's needs go to this window's realm, a batch at a time.
    pub(crate) fn connect(&self, realm: RasterInbox) {
        *self.jobs.realm.lock().unwrap() = Some(realm.clone());
        let batches = self.jobs.batches.clone();
        self.jobs.store.set_sink(Some(Arc::new(
            move |revision, epoch, needs: Vec<CompatNeed>| {
                let wire: Vec<Value> = needs
                    .iter()
                    .map(|need| {
                        json!({
                            "composition": need.composition.as_str(),
                            "layer": need.layer.as_str(),
                            "seconds": need.time.get() as f64 / TICKS_PER_SECOND as f64,
                            "scale": need.scale,
                            "crop": need.crop.map(|c| json!({"x":c.x,"y":c.y,"width":c.width,"height":c.height})),
                        })
                    })
                    .collect();
                let batch = batches
                    .lock()
                    .unwrap()
                    .open(Asker::Viewer(revision, epoch, needs));
                crate::diag::line(
                    json!({"rasters": "ask", "batch": batch, "needs": wire.len(), "revision": revision}),
                );
                realm.ask(batch, Value::Array(wire), json!({"revision": revision}));
            },
        )));
    }
}

impl JobOverrides {
    /// The generation of override wrappers drawing the project now (none: no
    /// override).
    pub(crate) fn epoch(&self) -> Option<u64> {
        self.store.producing_now().then(|| self.store.epoch())
    }
    /// While an override is registered, `session`'s override layers are
    /// answered by this window's extensions from `published`, the project the
    /// job froze. A delivery (an export, a render cache fill) is answered by
    /// a realm of its own, started now, so nothing the person edits meanwhile
    /// reaches it, the extensions' code and storage are the ones at its
    /// start, and the window's realm keeps serving the viewer. A one-frame
    /// `look` (an agent's still) asks the window's realm while that realm is
    /// at its exact revision, and its own realm only once the project moved
    /// on. Answers wait on the job's reader thread (never the main thread)
    /// until they come, the job is cancelled, the window's extensions stop,
    /// or a minute passes.
    pub(crate) fn pin(
        &self,
        session: &mut JobSession,
        published: &Arc<Published>,
        look: bool,
    ) -> ExportWaiting {
        let waiting = ExportWaiting::default();
        if !self.store.producing_now() {
            return waiting;
        }
        let job = session.request.job.as_str().to_owned();
        let revision = session.request.revision;
        let cancel = session.cancellation();
        // A job frozen from the viewer's scene a moment after `published`:
        // the window's publication while it is still at the job's revision.
        let frozen = Some(published.clone())
            .filter(|p| p.revision() == revision)
            .or_else(|| {
                self.kernel
                    .published(&self.project)
                    .filter(|p| p.snapshot.doc_id == self.doc_id && p.revision() == revision)
            });
        if !look && let (Some(realm), Some(frozen)) = (self.realm.lock().unwrap().clone(), &frozen)
        {
            let _ = realm.export(&job, frozen.clone(), cancel.clone());
        }
        session.answer_overrides(self.answers(
            job,
            revision,
            frozen,
            look,
            cancel,
            waiting.clone(),
        ));
        waiting
    }
    /// [`Self::pin`] from the project as it is published now (background
    /// rendering's fills, frozen from the viewer's scene a moment ago).
    pub(crate) fn pin_current(&self, session: &mut JobSession) {
        if let Some(published) = self
            .kernel
            .published(&self.project)
            .filter(|p| p.snapshot.doc_id == self.doc_id)
        {
            let _ = self.pin(session, &published, false);
        }
    }
    /// What answers job `job`'s needs (see [`Self::pin`]).
    fn answers(
        &self,
        job: String,
        revision: u64,
        frozen: Option<Arc<Published>>,
        look: bool,
        cancel: Arc<AtomicBool>,
        waiting: ExportWaiting,
    ) -> OverrideAnswers {
        let (batches, realm) = (self.batches.clone(), self.realm.clone());
        Box::new(move |needs: &[CompatJobNeed]| {
            let realm = realm
                .lock()
                .unwrap()
                .clone()
                .ok_or("The project's window has no extensions running")?;
            let frozen = frozen
                .clone()
                .ok_or("The project changed before its extensions could draw it")?;
            let wire = Value::Array(
                needs
                    .iter()
                    .map(|n| {
                        json!({"composition": n.composition, "layer": n.layer,
                        "seconds": n.seconds, "scale": n.scale})
                    })
                    .collect(),
            );
            let deadline = Instant::now() + EXPORT_WAIT;
            let ask = |ask: Value| -> Result<Vec<RasterAnswer>, String> {
                let (tx, rx) = std::sync::mpsc::channel();
                let batch = batches.lock().unwrap().open(Asker::Export(tx));
                let export = ask["job"].as_str().map(str::to_owned);
                realm.ask(batch, wire.clone(), ask);
                let asked = Instant::now();
                // The export's own realm: waited for while it makes progress.
                let mut progress = Progress::new(Instant::now());
                loop {
                    if cancel.load(Ordering::Acquire) {
                        batches.lock().unwrap().open.remove(&batch);
                        *waiting.lock().unwrap() = None;
                        return Err("cancelled".into());
                    }
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(answers) => {
                            *waiting.lock().unwrap() = None;
                            return Ok(answers);
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            let Some(job) = &export else {
                                if Instant::now() < deadline {
                                    continue;
                                }
                                batches.lock().unwrap().open.remove(&batch);
                                return Err(
                                    "The project's extensions did not draw the frame in time"
                                        .into(),
                                );
                            };
                            let (steps, note) = realm
                                .export_progress(job)
                                .unwrap_or((0, "waiting for the export's extensions".into()));
                            if asked.elapsed() >= WAITING_SHOWN {
                                *waiting.lock().unwrap() = Some(note.clone());
                            }
                            if progress.stalled(steps, Instant::now(), EXPORT_STALL) {
                                batches.lock().unwrap().open.remove(&batch);
                                *waiting.lock().unwrap() = None;
                                return Err(format!(
                                    "The export stopped {note}: nothing moved for {} s",
                                    EXPORT_STALL.as_secs()
                                ));
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            return Err("The project's extensions stopped during the export".into());
                        }
                    }
                }
            };

            let mut answers = None;
            if look {
                // Behind the still's revision: ask again once it caught up.
                loop {
                    let got = ask(json!({"revision": revision, "exact": true}))?;
                    if got.iter().any(|a| matches!(a, RasterAnswer::Moved)) {
                        break;
                    }
                    if !got.iter().any(|a| matches!(a, RasterAnswer::Retry)) {
                        answers = Some(got);
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(
                            "The project's extensions did not reach the frame's revision".into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let answers = match answers {
                Some(answers) => answers,
                None => {
                    realm.export(&job, frozen, cancel.clone())?;
                    ask(json!({"job": job}))?
                }
            };
            answers
                .into_iter()
                .map(|answer| match answer {
                    RasterAnswer::Native => Ok(CompatReply::Native),
                    RasterAnswer::Hidden => Ok(CompatReply::Hidden),
                    RasterAnswer::Failed(error) | RasterAnswer::Unavailable(error) => Err(error),
                    RasterAnswer::Retry | RasterAnswer::Moved => Err(
                        "The project's extensions could not answer for the export's revision"
                            .into(),
                    ),
                    // Export frames never crop, so their answers are whole.
                    RasterAnswer::Raster {
                        window: Some(_), ..
                    } => Err("An export frame was answered with a raster window".into()),
                    RasterAnswer::Raster {
                        width,
                        height,
                        premultiplied_rgba,
                        size,
                        anchor,
                        uv,
                        window: None,
                    } => Ok(CompatReply::Raster {
                        width,
                        height,
                        rgba: premultiplied_rgba,
                        size,
                        anchor,
                        uv,
                    }),
                })
                .collect()
        })
    }
}
impl Drop for Overrides {
    /// The window's extensions are gone: its project's frames and jobs stop
    /// asking them (and draw natively until another realm answers); jobs
    /// waiting on them fail now.
    fn drop(&mut self) {
        let jobs = &self.jobs;
        jobs.store.set_sink(None);
        jobs.store.set_state(false, 0, 0);
        let mut windows = self.windows.0.lock().unwrap();
        if windows
            .get(&jobs.doc_id)
            .is_some_and(|w| Arc::ptr_eq(&w.realm, &jobs.realm))
        {
            windows.remove(&jobs.doc_id);
        }
        drop(windows);
        *jobs.realm.lock().unwrap() = None;
        jobs.batches.lock().unwrap().open.clear();
    }
}
/// A new publication: frames are drawn at its revision, and asked ahead a
/// frame of its open composition at a time.
pub(crate) fn published(engine: &EngineService, p: &pm_kernel::Published) {
    let store = engine.compat(p.snapshot.doc_id);
    // An open gesture (a drag) first: its steps' revisions keep the answers
    // of the ones before to stand in.
    store.set_editing(p.entry.is_some());
    store.set_state(store.producing_now(), p.revision(), store.epoch());
    let fps = p
        .snapshot
        .root()
        .and_then(|root| {
            let open = p.snapshot.open_comp();
            let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
            comp.get("fps")?.to_value().as_f64()
        })
        .filter(|fps| *fps > 0.)
        .unwrap_or(30.);
    store.set_frame((TICKS_PER_SECOND as f64 / fps).round() as i64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, OpenSource};

    fn window(
        kernel: &KernelHandle,
        store: &Arc<CompatStore>,
        windows: &OverrideWindows,
    ) -> Overrides {
        let doc_id = kernel.published("synthetic").unwrap().snapshot.doc_id;
        let jobs = JobOverrides {
            store: store.clone(),
            kernel: kernel.clone(),
            project: "synthetic".into(),
            doc_id,
            batches: Arc::default(),
            realm: Arc::default(),
        };
        windows.0.lock().unwrap().insert(doc_id, jobs.clone());
        Overrides {
            rasters: None,
            jobs,
            windows: windows.clone(),
        }
    }
    fn session(published: &Published) -> JobSession {
        use pm_protocol::{color::*, common::*, priority::*};
        let settings = ExportSettings {
            format: ExportFormat::PngStill { sixteen_bit: false },
            alpha: AlphaOutput::Straight,
            quality: QualityTier::High,
            size: Size::new(64, 64),
            frame_rate: FrameRate::new(30, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new(1).unwrap(),
            },
            audio: false,
            audio_index_playhead: None,
            motion_blur: false,
            color: OutputColor::SrgbImage,
            precision: Precision::Float16,
            dither: false,
            draft_from_proxies: false,
            delivery: Default::default(),
        };
        let request = pm_engine_service::jobs::ExportRequest::new(
            "job".into(),
            &published.snapshot,
            JobSpec::Export {
                session: SessionId::new(published.snapshot.doc_id).unwrap(),
                composition: published.snapshot.render_comp().as_str().into(),
                settings,
            },
            Default::default(),
            "/nonexistent".into(),
        )
        .unwrap();
        JobSession::new(request)
    }
    fn overridden(session: &JobSession) -> bool {
        matches!(&session.request.spec, pm_protocol::priority::JobSpec::Export { settings, .. }
            if settings.delivery.compat.contains(&pm_protocol::snapshot::CompatSource::RasterOverride))
    }

    /// An export waits for its extensions while they make progress, however
    /// long that takes on a loaded machine (the fixed minute it had failed
    /// exports whose realm was still starting); it gives up only when nothing
    /// at all moved for the whole stall.
    #[test]
    fn an_export_waits_while_its_extensions_make_progress() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut slow = Progress::new(start);
        // A step every 30 s, for ten minutes: never stalled.
        for (i, s) in (0..600).step_by(30).enumerate() {
            assert!(!slow.stalled(i as u64, at(s), EXPORT_STALL), "at {s} s");
            assert!(!slow.stalled(i as u64, at(s + 29), EXPORT_STALL));
        }
        // No step at all: stalled once the whole stall has passed, not before.
        let mut stopped = Progress::new(start);
        assert!(!stopped.stalled(7, at(0), EXPORT_STALL));
        assert!(!stopped.stalled(7, at(119), EXPORT_STALL));
        assert!(stopped.stalled(7, at(120), EXPORT_STALL));
        // A step resets it.
        assert!(!stopped.stalled(8, at(121), EXPORT_STALL));
    }
    /// A project no window has open is answered by the host its jobs run;
    /// a window that opens it takes over with its own state untouched, and
    /// once that window has closed the jobs' host answers again.
    #[test]
    fn a_project_without_a_window_is_answered_until_a_window_takes_over() {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(
            r##"{"id":"synthetic","name":"Jobs","compId":"root","w":64,"h":64,"fps":30,"dur":1,
            "bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        kernel
            .open("synthetic", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let published = kernel.published("synthetic").unwrap();
        let doc_id = published.snapshot.doc_id;
        let windows = OverrideWindows::default();
        let answers = |who: &Overrides| {
            windows
                .of(doc_id)
                .is_some_and(|j| Arc::ptr_eq(&j.realm, &who.jobs.realm))
        };
        let agent = Overrides::windowless(kernel.clone(), "synthetic".into(), windows.clone());
        assert!(answers(&agent));
        agent.jobs.store.set_state(true, published.revision(), 1);
        let mut session = session(&published);
        windows
            .of(doc_id)
            .unwrap()
            .pin(&mut session, &published, false);
        assert!(overridden(&session), "its wrapper is asked");

        let store = Arc::new(CompatStore::default());
        let opened = window(&kernel, &store, &windows);
        store.set_state(true, published.revision(), 7);
        agent.register();
        assert!(answers(&opened), "the window answers");
        drop(agent);
        assert!(answers(&opened));
        assert_eq!(windows.of(doc_id).and_then(|j| j.epoch()), Some(7));

        let agent = Overrides::windowless(kernel.clone(), "synthetic".into(), windows.clone());
        assert!(answers(&opened), "a window is never displaced");
        drop(opened);
        assert!(windows.of(doc_id).is_none());
        agent.register();
        assert!(answers(&agent));
        drop(agent);
        actor.shutdown();
    }
    /// Jobs are pinned only while an override is registered; a reload's new
    /// extensions keep answering agents after the old ones go; a closed
    /// window answers nothing, at once.
    #[test]
    fn jobs_follow_the_window_that_answers_their_overrides() {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(
            r##"{"id":"synthetic","name":"Jobs","compId":"root","w":64,"h":64,"fps":30,"dur":1,
            "bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        kernel
            .open("synthetic", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let published = kernel.published("synthetic").unwrap();
        let doc_id = published.snapshot.doc_id;
        let (store, windows) = (Arc::new(CompatStore::default()), OverrideWindows::default());
        let old = window(&kernel, &store, &windows);
        let mut unpinned = session(&published);
        old.jobs.pin(&mut unpinned, &published, false);
        assert!(!overridden(&unpinned), "no override is registered");
        assert_eq!(windows.of(doc_id).and_then(|j| j.epoch()), None);
        store.set_state(true, published.revision(), 3);
        assert_eq!(windows.of(doc_id).and_then(|j| j.epoch()), Some(3));

        let reloaded = window(&kernel, &store, &windows);
        drop(old);
        store.set_state(true, published.revision(), 4);
        let agents = windows
            .of(doc_id)
            .expect("the reloaded window answers agents");
        assert!(Arc::ptr_eq(&agents.realm, &reloaded.jobs.realm));
        let mut pinned = session(&published);
        agents.pin(&mut pinned, &published, false);
        assert!(overridden(&pinned));

        let answers = reloaded.jobs.answers(
            "job".into(),
            published.revision(),
            Some(published.clone()),
            false,
            pinned.cancellation(),
            ExportWaiting::default(),
        );
        drop(reloaded);
        assert!(windows.of(doc_id).is_none());
        assert!(!store.producing_now());
        let need = CompatJobNeed {
            composition: "root".into(),
            layer: "title".into(),
            seconds: 0.,
            scale: 1.,
        };
        let started = Instant::now();
        let error = answers(&[need]).unwrap_err();
        assert_eq!(error, "The project's window has no extensions running");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
