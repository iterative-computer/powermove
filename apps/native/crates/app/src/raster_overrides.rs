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
    BaseRequest, CompatAnswer, CompatStore, Reply, hidden_raster, override_raster,
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
/// How long an export frame waits for the realm.
const EXPORT_WAIT: Duration = Duration::from_secs(60);

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
/// agent names a project, not a window.
#[derive(Clone, Default)]
pub struct OverrideWindows(Arc<Mutex<HashMap<u64, JobOverrides>>>);
impl OverrideWindows {
    pub(crate) fn of(&self, doc_id: u64) -> Option<JobOverrides> {
        self.0.lock().unwrap().get(&doc_id).cloned()
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
        } => Reply::Draw(override_raster(
            width,
            height,
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
            })?;
            Ok(painted.map(|r| BaseRasterReply {
                width: r.width,
                height: r.height,
                straight_rgba: r.straight_rgba,
                size: r.box_size,
                anchor: r.anchor,
                selection: r.selection,
                font_offset: r.font_offset,
                uv: r.uv,
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
    pub(crate) fn pin(&self, session: &mut JobSession, published: &Arc<Published>, look: bool) {
        if !self.store.producing_now() {
            return;
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
        session.answer_overrides(self.answers(job, revision, frozen, look, cancel));
    }
    /// [`Self::pin`] from the project as it is published now (background
    /// rendering's fills, frozen from the viewer's scene a moment ago).
    pub(crate) fn pin_current(&self, session: &mut JobSession) {
        if let Some(published) = self
            .kernel
            .published(&self.project)
            .filter(|p| p.snapshot.doc_id == self.doc_id)
        {
            self.pin(session, &published, false);
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
                realm.ask(batch, wire.clone(), ask);
                loop {
                    if cancel.load(Ordering::Acquire) {
                        batches.lock().unwrap().open.remove(&batch);
                        return Err("cancelled".into());
                    }
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(answers) => return Ok(answers),
                        Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                        Err(RecvTimeoutError::Timeout) => {
                            batches.lock().unwrap().open.remove(&batch);
                            return Err("The extension did not draw the layer in time".into());
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
                    RasterAnswer::Raster {
                        width,
                        height,
                        premultiplied_rgba,
                        size,
                        anchor,
                        uv,
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
