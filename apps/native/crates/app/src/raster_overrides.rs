//! Raster overrides between the engine and a window's extension realm. The
//! viewer's frames ask the engine's `CompatStore` for each layer today's
//! compositor rasterizes; what it hasn't answered goes to the realm in a
//! batch, and the realm's answers (an override's canvas, or a pass-through)
//! come back to the store, which redraws. Base rasters (`render.raster`) are
//! painted by the engine's own raster worker. Nothing here runs on the main
//! thread or waits on the viewer.
use pm_compat_runner::{BaseRasterReply, BaseRasterRequest, RasterAnswer, RasterHooks};
use pm_engine_service::EngineService;
use pm_engine_service::compat::{BaseRequest, CompatAnswer, CompatStore, override_raster};
use pm_kernel::KernelHandle;
use pm_protocol::common::{PixelRect, TICKS_PER_SECOND};
use pm_protocol::snapshot::CompatNeed;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Batches asked of the realm and not answered, with the revision they are for.
#[derive(Default)]
struct Batches {
    next: u64,
    open: HashMap<u64, (u64, Vec<CompatNeed>)>,
}

pub(crate) struct Overrides {
    store: Arc<CompatStore>,
    rasters: Option<Arc<pm_engine_service::compat::LayerRasters>>,
    kernel: KernelHandle,
    project: String,
    batches: Arc<Mutex<Batches>>,
}

impl Overrides {
    pub(crate) fn new(engine: &EngineService, kernel: KernelHandle, project: String) -> Self {
        Self {
            store: engine.compat(),
            rasters: engine.layer_rasters(),
            kernel,
            project,
            batches: Arc::default(),
        }
    }
    pub(crate) fn hooks(&self) -> RasterHooks {
        let rasters = self.rasters.clone();
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
            }))
        });
        let (store, batches) = (self.store.clone(), self.batches.clone());
        let answers = Arc::new(move |batch: u64, answers: Vec<RasterAnswer>| {
            let Some((revision, needs)) = batches.lock().unwrap().open.remove(&batch) else {
                return;
            };
            let answered = needs
                .into_iter()
                .zip(answers)
                .map(|(need, answer)| {
                    let answer = match answer {
                        RasterAnswer::Native => CompatAnswer::Native,
                        RasterAnswer::Raster {
                            width,
                            height,
                            straight_rgba,
                            size,
                            anchor,
                        } => override_raster(width, height, straight_rgba, size, anchor),
                    };
                    (need, answer)
                })
                .collect();
            store.answer_all(revision, answered);
        });
        let (store, kernel, project) = (
            self.store.clone(),
            self.kernel.clone(),
            self.project.clone(),
        );
        let producing = Arc::new(move |on: bool| {
            let revision = kernel.published(&project).map_or(0, |p| p.revision());
            store.set_state(on, revision);
        });
        RasterHooks {
            base,
            answers,
            producing,
        }
    }
    /// The store's needs go to this window's realm, a batch at a time.
    pub(crate) fn connect(&self, realm: Arc<dyn Fn(u64, Value) + Send + Sync>) {
        let batches = self.batches.clone();
        self.store.set_sink(Some(Arc::new(move |revision, needs: Vec<CompatNeed>| {
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
            let mut open = batches.lock().unwrap();
            open.next += 1;
            let batch = open.next;
            open.open.insert(batch, (revision, needs));
            drop(open);
            realm(batch, Value::Array(wire));
        })));
    }
}

/// A new publication: frames are drawn at its revision.
pub(crate) fn published(engine: &EngineService, revision: u64) {
    let store = engine.compat();
    store.set_state(store.producing_now(), revision);
}
