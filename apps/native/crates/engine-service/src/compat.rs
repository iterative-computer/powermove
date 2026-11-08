//! Raster overrides (plan section 20, `pm_render::compat`): a v1–3 extension
//! that wraps the `raster` service draws some layers itself.
//!
//! - [`LayerRasters`] paints the base raster an override starts from
//!   (`render.raster`, today's `PM.raster`) on its own renderer and thread, so
//!   an extension waiting for one never waits on the viewer's frames.
//! - [`CompatStore`] is the viewer's producer: a frame asks it for each layer
//!   today's compositor rasterizes; a layer it has no answer for fails the
//!   frame with `CompatPending`, the needs go to the extension realm through
//!   [`CompatStore::set_sink`], and the realm's answers come back through
//!   [`CompatStore::answer_all`], which redraws the viewers. The viewer never
//!   blocks on JavaScript.
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use pm_model::{Document, Node};
use pm_protocol::common::PixelRect;
use pm_protocol::snapshot::{CompatNeed, CompatSource, SnapshotPixels};
use pm_render::Renderer;
pub use pm_render::compat::CompatAnswer;
use pm_render::compat::{CompatRaster, CompatRasters};

/// One base raster request: the layer by id, or the layer object an
/// override passed (`content`, `{ ...L, d: { ...L.d, text } }`).
pub struct BaseRequest {
    pub document: Arc<Document>,
    pub composition: String,
    pub layer: String,
    pub content: Option<pm_model::Value>,
    pub seconds: f64,
    pub scale: f64,
    /// Today's preview crop window, in raster pixels.
    pub crop: Option<PixelRect>,
}

/// A base raster as an extension's canvas sees it: straight RGBA8 rows of
/// the canvas (or its crop window), with today's geometry.
pub struct BaseRaster {
    pub width: u32,
    pub height: u32,
    pub straight_rgba: Vec<u8>,
    pub box_size: (f64, f64),
    pub anchor: (f64, f64),
    pub selection: Option<[f64; 4]>,
    pub font_offset: Option<(f64, f64)>,
}

type Job = (BaseRequest, Sender<Result<Option<BaseRaster>, String>>);

/// The base raster worker: a second renderer on the app's GPU, with the
/// app's fonts, owned by one thread.
pub struct LayerRasters {
    jobs: Mutex<Sender<Job>>,
}

impl LayerRasters {
    pub fn start(
        device: wgpu::Device,
        queue: wgpu::Queue,
        fonts: Option<pm_node::fonts::render::RenderFonts>,
    ) -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("pm-layer-rasters".into())
            .spawn(move || {
                let mut renderer = Renderer::new(device, queue);
                let _ = renderer.set_definitions(&pm_render::all_builtin_definitions());
                renderer.set_captions_hidden(false);
                let mut generation = None;
                for (request, reply) in rx {
                    if let Some(fonts) = &fonts {
                        fonts.sync(&mut renderer, &mut generation);
                    }
                    let _ = reply.send(paint(&mut renderer, request));
                }
            })?;
        Ok(Arc::new(Self {
            jobs: Mutex::new(tx),
        }))
    }
    /// Blocks the caller (an extension realm's worker, never the main thread).
    pub fn raster(&self, request: BaseRequest) -> Result<Option<BaseRaster>, String> {
        let (tx, rx) = channel();
        self.jobs
            .lock()
            .unwrap()
            .send((request, tx))
            .map_err(|_| "The layer raster worker stopped".to_owned())?;
        rx.recv()
            .map_err(|_| "The layer raster worker stopped".to_owned())?
    }
}

fn paint(renderer: &mut Renderer, request: BaseRequest) -> Result<Option<BaseRaster>, String> {
    let content = request
        .content
        .map(|value| Node::from_value(value, request.document.revision()));
    let painted = renderer
        .layer_raster(
            &request.document,
            &request.composition,
            &request.layer,
            content.as_ref(),
            request.seconds,
            request.scale,
            pm_protocol::color::Precision::Float16,
        )
        .map_err(|e| e.to_string())?;
    let Some(raster) = painted else {
        return Ok(None);
    };
    let geometry = |width, height, straight_rgba| BaseRaster {
        width,
        height,
        straight_rgba,
        box_size: raster.box_size,
        anchor: raster.anchor,
        selection: raster.selection,
        font_offset: raster.font_offset,
    };
    // Text with no ink is a transparent canvas of its geometry.
    if raster.pixels.pixels.is_empty() {
        let (w, h) = raster.canvas;
        return Ok(Some(geometry(w, h, vec![0; w as usize * h as usize * 4])));
    }
    let painted = pm_node::present::snapshot::encode_rgba8_f32(
        &raster.pixels.pixels,
        SnapshotPixels::Straight,
    );
    let window = if raster.crops { request.crop } else { None };
    Ok(pm_node::present::snapshot::place_raster(
        &painted,
        raster.pixels.crop,
        raster.canvas,
        window,
    )
    .map(|(pixels, w, h)| geometry(w, h, pixels)))
}

/// What one need is: the layer at one frame, scale and crop.
type NeedKey = (String, String, i64, u64, Option<[u32; 4]>);
fn key(composition: &str, layer: &str, ticks: i64, scale: f64, crop: Option<PixelRect>) -> NeedKey {
    (
        composition.to_owned(),
        layer.to_owned(),
        ticks,
        scale.to_bits(),
        crop.map(|c| [c.x, c.y, c.width, c.height]),
    )
}
fn need_key(need: &CompatNeed) -> NeedKey {
    key(
        need.composition.as_str(),
        need.layer.as_str(),
        need.time.get(),
        need.scale,
        need.crop,
    )
}

/// Frames asked ahead of one the viewer missed.
const AHEAD: i64 = 12;
/// The rasters one revision's frames draw from, newest last.
const MAX_BYTES: u64 = 256 << 20;

#[derive(Default)]
struct Answers {
    revision: u64,
    producing: bool,
    /// One frame of the open composition in ticks, for asking ahead.
    frame: i64,
    map: HashMap<NeedKey, CompatAnswer>,
    order: VecDeque<(NeedKey, u64)>,
    bytes: u64,
    /// Asked of the realm and not answered yet.
    asked: HashSet<NeedKey>,
}

/// Where needs go: the extension realm of the window showing the document.
pub type CompatSink = Arc<dyn Fn(u64, Vec<CompatNeed>) + Send + Sync>;

/// The viewer's raster override answers for the open document.
#[derive(Default)]
pub struct CompatStore {
    answers: Mutex<Answers>,
    sink: Mutex<Option<CompatSink>>,
    /// Redraws the viewers when answers land.
    redraw: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl CompatStore {
    /// Whether an override is registered, and the document revision frames
    /// are drawn at. A new revision forgets every answer.
    pub fn set_state(&self, producing: bool, revision: u64) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision {
            let frame = answers.frame;
            *answers = Answers {
                revision,
                frame,
                ..Answers::default()
            };
        }
        let changed = answers.producing != producing;
        answers.producing = producing;
        drop(answers);
        if changed && let Some(redraw) = self.redraw.lock().unwrap().clone() {
            redraw();
        }
    }
    pub fn producing_now(&self) -> bool {
        self.answers.lock().unwrap().producing
    }
    pub fn set_sink(&self, sink: Option<CompatSink>) {
        *self.sink.lock().unwrap() = sink;
    }
    pub(crate) fn set_redraw(&self, redraw: Arc<dyn Fn() + Send + Sync>) {
        *self.redraw.lock().unwrap() = Some(redraw);
    }
    /// One frame of the open composition, in ticks (0: don't ask ahead).
    pub fn set_frame(&self, ticks: i64) {
        self.answers.lock().unwrap().frame = ticks.max(0);
    }
    /// A frame's missing answers: each is asked of the realm once, and while
    /// playing with the same layers' next frames, so playback finds them answered.
    pub(crate) fn request(&self, needs: Vec<CompatNeed>, playing: bool) {
        let mut answers = self.answers.lock().unwrap();
        let revision = answers.revision;
        // Ahead only while playing: a still frame needs itself alone.
        let frame = if playing { answers.frame } else { 0 };
        let ahead = needs.iter().flat_map(|need| {
            (1..=AHEAD).filter(move |_| frame > 0).map(move |k| {
                let mut next = need.clone();
                next.time = pm_render::compat::ticks_of(
                    (need.time.get() as f64 + (k * frame) as f64)
                        / pm_protocol::common::TICKS_PER_SECOND as f64,
                );
                next
            })
        });
        let all: Vec<CompatNeed> = needs.iter().cloned().chain(ahead).collect();
        let fresh: Vec<CompatNeed> = all
            .into_iter()
            .filter(|need| {
                let key = need_key(need);
                !answers.map.contains_key(&key) && answers.asked.insert(key)
            })
            .collect();
        drop(answers);
        if fresh.is_empty() {
            return;
        }
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            sink(revision, fresh);
        }
    }
    /// The realm's answers for `revision`; stale ones are dropped.
    pub fn answer_all(&self, revision: u64, answered: Vec<(CompatNeed, CompatAnswer)>) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision {
            return;
        }
        for (need, answer) in answered {
            let key = need_key(&need);
            answers.asked.remove(&key);
            let bytes = match &answer {
                CompatAnswer::Raster(r) => r.data.len() as u64,
                _ => 0,
            };
            answers.bytes += bytes;
            answers.order.push_back((key.clone(), bytes));
            answers.map.insert(key, answer);
        }
        while answers.bytes > MAX_BYTES {
            let Some((old, bytes)) = answers.order.pop_front() else {
                break;
            };
            answers.map.remove(&old);
            answers.bytes -= bytes;
        }
        drop(answers);
        if let Some(redraw) = self.redraw.lock().unwrap().clone() {
            redraw();
        }
    }
}

/// One [`CompatStore`] per open document (kernel `doc_id`): each window's
/// extensions answer for their own project's frames.
#[derive(Default)]
pub struct CompatStores {
    stores: Mutex<HashMap<u64, Arc<CompatStore>>>,
    redraw: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
impl CompatStores {
    pub(crate) fn set_redraw(&self, redraw: Arc<dyn Fn() + Send + Sync>) {
        *self.redraw.lock().unwrap() = Some(redraw);
    }
    /// The document's store, made on first use.
    pub fn document(&self, doc_id: u64) -> Arc<CompatStore> {
        self.stores
            .lock()
            .unwrap()
            .entry(doc_id)
            .or_insert_with(|| {
                let store = Arc::new(CompatStore::default());
                if let Some(redraw) = self.redraw.lock().unwrap().clone() {
                    store.set_redraw(redraw);
                }
                store
            })
            .clone()
    }
    /// A closed document's answers go.
    pub fn forget(&self, doc_id: u64) {
        self.stores.lock().unwrap().remove(&doc_id);
    }
}
impl CompatRasters for CompatStore {
    fn producing(&self, source: CompatSource) -> bool {
        source == CompatSource::RasterOverride && self.answers.lock().unwrap().producing
    }
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<PixelRect>,
    ) -> CompatAnswer {
        if source != CompatSource::RasterOverride {
            return CompatAnswer::Native;
        }
        let ticks = pm_render::compat::ticks_of(time).get();
        self.answers
            .lock()
            .unwrap()
            .map
            .get(&key(composition, layer, ticks, scale, crop))
            .cloned()
            .unwrap_or(CompatAnswer::Missing)
    }
}

/// A realm canvas as an override's raster: premultiplied RGBA8 rows (the
/// engine's fast upload path), the quad's size and anchor in layer pixels.
pub fn override_raster(
    width: u32,
    height: u32,
    premultiplied_rgba: Vec<u8>,
    size: [f64; 2],
    anchor: [f64; 2],
) -> CompatAnswer {
    CompatAnswer::Raster(Arc::new(CompatRaster {
        width,
        height,
        format: pm_protocol::snapshot::CompatFormat::Rgba8Premultiplied,
        anchor,
        size,
        uv: [0.0, 0.0, 1.0, 1.0],
        data: Arc::new(premultiplied_rgba),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{CompositionId, LayerId};
    fn need(layer: &str) -> CompatNeed {
        CompatNeed {
            composition: CompositionId::new("c"),
            layer: LayerId::new(layer),
            time: pm_render::compat::ticks_of(1.),
            source: CompatSource::RasterOverride,
            scale: 1.,
            crop: None,
        }
    }
    #[test]
    fn each_document_answers_from_its_own_window() {
        let stores = CompatStores::default();
        let (a, b) = (stores.document(1), stores.document(2));
        let asked = Arc::new(Mutex::new(vec![]));
        for (doc, store) in [(1u64, &a), (2u64, &b)] {
            let asked = asked.clone();
            store.set_sink(Some(Arc::new(move |_, needs: Vec<CompatNeed>| {
                asked.lock().unwrap().push((doc, needs.len()));
            })));
            store.set_state(true, 7);
        }
        a.request(vec![need("x")], false);
        assert_eq!(*asked.lock().unwrap(), vec![(1, 1)]);
        a.answer_all(
            7,
            vec![(
                need("x"),
                override_raster(1, 1, vec![9; 4], [1., 1.], [0., 0.]),
            )],
        );
        let answer = |store: &CompatStore| {
            store.answer("c", "x", 1., 1., CompatSource::RasterOverride, None)
        };
        assert!(matches!(answer(&a), CompatAnswer::Raster(_)));
        assert!(matches!(answer(&b), CompatAnswer::Missing));
        assert!(Arc::ptr_eq(&stores.document(1), &a));
        stores.forget(1);
        assert!(!Arc::ptr_eq(&stores.document(1), &a));
    }
}
