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
use pm_engine_service::jobs::{CompatAnswerer, CompatJobNeed, CompatPin, CompatReply};
use pm_kernel::KernelHandle;
use pm_protocol::common::{PixelRect, TICKS_PER_SECOND};
use pm_protocol::snapshot::CompatNeed;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
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
    store: Arc<CompatStore>,
    rasters: Option<Arc<pm_engine_service::compat::LayerRasters>>,
    kernel: KernelHandle,
    project: String,
    /// The document this window's extensions answer for.
    doc_id: u64,
    batches: Arc<Mutex<Batches>>,
    realm: Arc<Mutex<Option<RasterInbox>>>,
}

fn answer_of(answer: RasterAnswer) -> Reply {
    match answer {
        RasterAnswer::Native => Reply::Draw(CompatAnswer::Native),
        RasterAnswer::Hidden => Reply::Draw(hidden_raster()),
        RasterAnswer::Retry | RasterAnswer::Moved => Reply::Retry,
        RasterAnswer::Failed(_) => Reply::Failed,
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
    pub(crate) fn new(engine: &EngineService, kernel: KernelHandle, project: String) -> Self {
        let doc_id = kernel.published(&project).map_or(0, |p| p.snapshot.doc_id);
        Self {
            store: engine.compat(doc_id),
            doc_id,
            rasters: engine.layer_rasters(),
            kernel,
            project,
            batches: Arc::default(),
            realm: Arc::default(),
        }
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
        let (store, batches) = (self.store.clone(), self.batches.clone());
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
            self.store.clone(),
            self.kernel.clone(),
            self.project.clone(),
        );
        let export = self.export_answerer();
        let begin = self.export_begin();
        let doc_id = self.doc_id;
        let producing = Arc::new(move |on: bool, epoch: u64| {
            let revision = kernel.published(&project).map_or(0, |p| p.revision());
            store.set_state(on, revision, epoch);
            // Exports started while an override is registered ask this realm.
            let store = store.clone();
            pm_engine_service::jobs::set_compat_answerer(
                doc_id,
                on.then(|| {
                    (
                        Arc::new(move || store.producing_now().then(|| store.epoch()))
                            as pm_engine_service::jobs::CompatState,
                        export.clone(),
                        begin.clone(),
                    )
                }),
            );
        });
        RasterHooks {
            base,
            answers,
            producing,
        }
    }
    /// A delivery starts: its realm starts now, so the extensions' code and
    /// storage it runs are the ones at the export's start.
    fn export_begin(&self) -> pm_engine_service::jobs::CompatBegin {
        let realm = self.realm.clone();
        let (kernel, project, doc_id) = (self.kernel.clone(), self.project.clone(), self.doc_id);
        Arc::new(move |pin: &CompatPin| {
            let realm = realm.lock().unwrap().clone();
            if let (Some(realm), Ok(frozen)) = (realm, frozen_of(pin, &kernel, &project, doc_id)) {
                let _ = realm.export(&pin.job, frozen, pin.cancel.clone());
            }
        })
    }
    /// An export frame's needs, waited for on the job's reader thread (never
    /// the main thread) until they come, the job is cancelled, the window's
    /// extensions stop, or a minute passes. A delivery (an export, a render
    /// cache fill) is answered by a realm of its own, started when it starts
    /// from the publication it froze, so nothing the person edits meanwhile
    /// reaches it and the window's realm keeps serving the viewer. A one-frame
    /// look (an agent's still) asks the window's realm while that realm is at
    /// its exact revision, and its own realm only once the project moved on.
    fn export_answerer(&self) -> CompatAnswerer {
        let (batches, realm) = (self.batches.clone(), self.realm.clone());
        let (kernel, project, doc_id) = (self.kernel.clone(), self.project.clone(), self.doc_id);
        Arc::new(move |pin: &CompatPin, needs: &[CompatJobNeed]| {
            let realm = realm
                .lock()
                .unwrap()
                .clone()
                .ok_or("The project's window has no extensions running")?;
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
                    if pin.cancel.load(Ordering::Acquire) {
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
            if pin.look {
                // Behind the still's revision: ask again once it caught up.
                loop {
                    let got = ask(json!({"revision": pin.revision, "exact": true}))?;
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
                    let frozen = frozen_of(pin, &kernel, &project, doc_id)?;
                    realm.export(&pin.job, frozen, pin.cancel.clone())?;
                    ask(json!({"job": pin.job}))?
                }
            };
            answers
                .into_iter()
                .map(|answer| match answer {
                    RasterAnswer::Native => Ok(CompatReply::Native),
                    RasterAnswer::Hidden => Ok(CompatReply::Hidden),
                    RasterAnswer::Failed(error) => Err(error),
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
    /// The store's needs go to this window's realm, a batch at a time.
    pub(crate) fn connect(&self, realm: RasterInbox) {
        *self.realm.lock().unwrap() = Some(realm.clone());
        let batches = self.batches.clone();
        self.store.set_sink(Some(Arc::new(
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

/// The publication a job froze; without one (a request made before it could
/// carry it), the window's while it is still at the job's revision.
fn frozen_of(
    pin: &CompatPin,
    kernel: &KernelHandle,
    project: &str,
    doc_id: u64,
) -> Result<Arc<pm_kernel::Published>, String> {
    pin.frozen
        .clone()
        .and_then(|f| f.downcast::<pm_kernel::Published>().ok())
        .or_else(|| {
            kernel
                .published(project)
                .filter(|p| p.snapshot.doc_id == doc_id && p.revision() == pin.revision)
        })
        .ok_or_else(|| "The project changed before its extensions could draw it".to_owned())
}
impl Drop for Overrides {
    /// The window's extensions are gone: its project's frames and exports
    /// stop asking them (and draw natively until another realm answers);
    /// exports waiting on them fail now.
    fn drop(&mut self) {
        self.store.set_sink(None);
        self.store.set_state(false, 0, 0);
        pm_engine_service::jobs::set_compat_answerer(self.doc_id, None);
        *self.realm.lock().unwrap() = None;
        self.batches.lock().unwrap().open.clear();
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
