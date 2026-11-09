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
use std::time::{Duration, Instant};

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
    /// The part of the canvas the layer's quad samples (`[ox, oy, sx, sy]`):
    /// a crop window's canvas sits inside the whole raster.
    pub uv: [f64; 4],
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
    let geometry = |width, height, straight_rgba, uv| BaseRaster {
        width,
        height,
        straight_rgba,
        box_size: raster.box_size,
        anchor: raster.anchor,
        selection: raster.selection,
        font_offset: raster.font_offset,
        uv,
    };
    // Text with no ink is a transparent canvas of its geometry.
    if raster.pixels.pixels.is_empty() {
        let (w, h) = raster.canvas;
        return Ok(Some(geometry(
            w,
            h,
            vec![0; w as usize * h as usize * 4],
            [0., 0., 1., 1.],
        )));
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
    .map(|(pixels, w, h)| {
        // A crop window's canvas: the quad still spans the whole raster.
        let uv = match window {
            Some(c) if w > 0 && h > 0 => {
                let (cw, ch) = (f64::from(w), f64::from(h));
                [
                    -f64::from(c.x) / cw,
                    -f64::from(c.y) / ch,
                    f64::from(raster.canvas.0) / cw,
                    f64::from(raster.canvas.1) / ch,
                ]
            }
            _ => [0., 0., 1., 1.],
        };
        geometry(w, h, pixels, uv)
    }))
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
/// Answers kept at most (a pass-through costs no pixels but still a key).
const MAX_ENTRIES: usize = 16_384;
/// A need asked this long ago and never answered is asked again.
const ASK_AGAIN: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Answers {
    revision: u64,
    /// The realm's generation of `raster` wrappers the answers are from.
    epoch: u64,
    producing: bool,
    /// One frame of the open composition in ticks, for asking ahead.
    frame: i64,
    map: HashMap<NeedKey, CompatAnswer>,
    order: VecDeque<(NeedKey, u64)>,
    bytes: u64,
    /// Asked of the realm and not answered yet, and when.
    asked: HashMap<NeedKey, Instant>,
    /// Needs whose override failed: their frames fail (as main's compositor
    /// throws) and are not asked again until the revision or wrappers change.
    failed: HashSet<NeedKey>,
}

/// Where needs go: the extension realm of the window showing the document,
/// with the revision and wrapper generation they are asked at.
pub type CompatSink = Arc<dyn Fn(u64, u64, Vec<CompatNeed>) + Send + Sync>;

/// What the realm answered for one need.
pub enum Reply {
    Draw(CompatAnswer),
    /// The override threw or returned what cannot be drawn.
    Failed,
    /// The realm had not seen the revision yet.
    Retry,
}

/// The viewer's raster override answers for the open document.
#[derive(Default)]
pub struct CompatStore {
    answers: Mutex<Answers>,
    sink: Mutex<Option<CompatSink>>,
    /// Redraws the viewers when answers land.
    redraw: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl CompatStore {
    /// Whether an override is registered, the generation of wrappers
    /// answering, and the document revision frames are drawn at. A new
    /// revision or generation forgets every answer.
    pub fn set_state(&self, producing: bool, revision: u64, epoch: u64) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision || answers.epoch != epoch {
            let frame = answers.frame;
            *answers = Answers {
                revision,
                epoch,
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
    /// The generation of wrappers answering now.
    pub fn epoch(&self) -> u64 {
        self.answers.lock().unwrap().epoch
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
        let (revision, epoch) = (answers.revision, answers.epoch);
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
        // Asks the realm never answered (it stopped, or dropped them) retire.
        if answers.asked.len() > MAX_ENTRIES {
            answers.asked.retain(|_, at| at.elapsed() < ASK_AGAIN);
        }
        let now = Instant::now();
        let answers = &mut *answers;
        let fresh: Vec<CompatNeed> = all
            .into_iter()
            .filter(|need| {
                let key = need_key(need);
                if answers.map.contains_key(&key) || answers.failed.contains(&key) {
                    return false;
                }
                match answers.asked.get(&key) {
                    Some(at) if at.elapsed() < ASK_AGAIN => false,
                    _ => {
                        answers.asked.insert(key, now);
                        true
                    }
                }
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            sink(revision, epoch, fresh);
        }
    }
    /// The realm's answers for `revision` and `epoch`; stale ones are dropped.
    pub fn answer_all(&self, revision: u64, epoch: u64, answered: Vec<(CompatNeed, Reply)>) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision || answers.epoch != epoch {
            return;
        }
        let mut drew = false;
        for (need, reply) in answered {
            let key = need_key(&need);
            answers.asked.remove(&key);
            let answer = match reply {
                Reply::Draw(answer) => answer,
                Reply::Failed => {
                    if answers.failed.len() >= MAX_ENTRIES {
                        answers.failed.clear();
                    }
                    answers.failed.insert(key);
                    continue;
                }
                // Asked again by the next frame that needs it.
                Reply::Retry => {
                    drew = true;
                    continue;
                }
            };
            if answers.map.contains_key(&key) {
                continue;
            }
            let bytes = match &answer {
                CompatAnswer::Raster(r) => r.data.len() as u64,
                _ => 0,
            };
            answers.bytes += bytes;
            answers.order.push_back((key.clone(), bytes));
            answers.map.insert(key, answer);
            drew = true;
        }
        while answers.bytes > MAX_BYTES || answers.map.len() > MAX_ENTRIES {
            let Some((old, bytes)) = answers.order.pop_front() else {
                break;
            };
            answers.map.remove(&old);
            answers.bytes -= bytes;
        }
        drop(answers);
        if drew && let Some(redraw) = self.redraw.lock().unwrap().clone() {
            redraw();
        }
    }
    #[cfg(test)]
    fn sizes(&self) -> (usize, usize, usize) {
        let answers = self.answers.lock().unwrap();
        (answers.map.len(), answers.order.len(), answers.asked.len())
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
/// engine's fast upload path), the quad's size and anchor in layer pixels,
/// and the part of the canvas it samples.
pub fn override_raster(
    width: u32,
    height: u32,
    premultiplied_rgba: Vec<u8>,
    size: [f64; 2],
    anchor: [f64; 2],
    uv: [f64; 4],
) -> CompatAnswer {
    CompatAnswer::Raster(Arc::new(CompatRaster {
        width,
        height,
        format: pm_protocol::snapshot::CompatFormat::Rgba8Premultiplied,
        anchor,
        size,
        uv,
        data: Arc::new(premultiplied_rgba),
    }))
}
/// An override that returned nothing: the layer draws nothing (main skips it).
pub fn hidden_raster() -> CompatAnswer {
    override_raster(1, 1, vec![0; 4], [1., 1.], [0., 0.], [0., 0., 1., 1.])
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
    fn draw(byte: u8) -> Reply {
        Reply::Draw(override_raster(
            1,
            1,
            vec![byte; 4],
            [1., 1.],
            [0., 0.],
            [0., 0., 1., 1.],
        ))
    }
    fn answer(store: &CompatStore, layer: &str) -> CompatAnswer {
        store.answer("c", layer, 1., 1., CompatSource::RasterOverride, None)
    }
    fn asking(store: &CompatStore) -> Arc<Mutex<Vec<(u64, usize)>>> {
        let asked = Arc::new(Mutex::new(vec![]));
        let log = asked.clone();
        store.set_sink(Some(Arc::new(move |_, epoch, needs: Vec<CompatNeed>| {
            log.lock().unwrap().push((epoch, needs.len()));
        })));
        asked
    }
    #[test]
    fn each_document_answers_from_its_own_window() {
        let stores = CompatStores::default();
        let (a, b) = (stores.document(1), stores.document(2));
        let (asked_a, asked_b) = (asking(&a), asking(&b));
        a.set_state(true, 7, 1);
        b.set_state(true, 7, 1);
        a.request(vec![need("x")], false);
        assert_eq!(*asked_a.lock().unwrap(), vec![(1, 1)]);
        assert!(asked_b.lock().unwrap().is_empty());
        a.answer_all(7, 1, vec![(need("x"), draw(9))]);
        assert!(matches!(answer(&a, "x"), CompatAnswer::Raster(_)));
        assert!(matches!(answer(&b, "x"), CompatAnswer::Missing));
        assert!(Arc::ptr_eq(&stores.document(1), &a));
        stores.forget(1);
        assert!(!Arc::ptr_eq(&stores.document(1), &a));
    }
    #[test]
    fn new_wrappers_at_the_same_revision_forget_earlier_answers() {
        let store = CompatStore::default();
        let asked = asking(&store);
        store.set_state(true, 7, 1);
        store.answer_all(7, 1, vec![(need("x"), draw(255))]);
        assert!(matches!(answer(&store, "x"), CompatAnswer::Raster(_)));
        // Another provider registered: same revision, new generation.
        store.set_state(true, 7, 2);
        assert!(matches!(answer(&store, "x"), CompatAnswer::Missing));
        // An answer computed by the earlier wrappers is dropped.
        store.answer_all(7, 1, vec![(need("x"), draw(255))]);
        assert!(matches!(answer(&store, "x"), CompatAnswer::Missing));
        store.request(vec![need("x")], false);
        assert_eq!(*asked.lock().unwrap(), vec![(2, 1)]);
    }
    #[test]
    fn a_failed_override_fails_its_frames_without_asking_again() {
        let store = CompatStore::default();
        let asked = asking(&store);
        store.set_state(true, 7, 1);
        store.request(vec![need("x")], false);
        store.answer_all(7, 1, vec![(need("x"), Reply::Failed)]);
        assert!(matches!(answer(&store, "x"), CompatAnswer::Missing));
        store.request(vec![need("x")], false);
        assert_eq!(asked.lock().unwrap().len(), 1);
        // A realm that had not caught up is asked again.
        store.request(vec![need("y")], false);
        store.answer_all(7, 1, vec![(need("y"), Reply::Retry)]);
        store.request(vec![need("y")], false);
        assert_eq!(asked.lock().unwrap().len(), 3);
        // An edit retries the failed one.
        store.set_state(true, 8, 1);
        store.request(vec![need("x")], false);
        assert_eq!(asked.lock().unwrap().len(), 4);
    }
    #[test]
    fn pass_through_answers_are_bounded_too() {
        let store = CompatStore::default();
        store.set_state(true, 7, 1);
        let needs: Vec<_> = (0..MAX_ENTRIES + 500)
            .map(|i| {
                let mut n = need("x");
                n.time = pm_protocol::common::Ticks::new(i as i64).unwrap();
                (n, Reply::Draw(CompatAnswer::Native))
            })
            .collect();
        store.answer_all(7, 1, needs);
        let (map, order, _) = store.sizes();
        assert_eq!((map, order), (MAX_ENTRIES, MAX_ENTRIES));
    }
}
