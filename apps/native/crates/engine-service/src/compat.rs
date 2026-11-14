//! Raster overrides (plan section 20, `pm_render::compat`): a v1–3 extension
//! that wraps the `raster` service draws some layers itself.
//!
//! - [`LayerRasters`] paints the base raster an override starts from
//!   (`render.raster`, today's `PM.raster`) on its own renderer and thread, so
//!   an extension waiting for one never waits on the viewer's frames.
//! - [`CompatStore`] is the viewer's producer: a frame asks it for each layer
//!   today's compositor rasterizes. A layer it has no answer for goes to the
//!   extension realm through [`CompatStore::set_sink`], one batch at a time,
//!   and the realm's answers come back through [`CompatStore::answer_all`],
//!   which redraws the viewers. A paused frame waits for its answers
//!   (`CompatPending`); a playing or scrubbing frame never does: it draws a
//!   late layer from its nearest answer (plan 3.6.4: "a late override reuses
//!   its last-good surface"), or without the override when the layer has no
//!   answer yet, and is redrawn exact when the answer lands. The viewer
//!   never blocks on JavaScript, and never stops showing frames for it.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pm_model::{Document, Node};
use pm_protocol::common::{PixelRect, TICKS_PER_SECOND};
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
    /// Only these pixels of the whole (uncropped) raster are wanted: the
    /// reply is the whole raster with just this window painted
    /// ([`BaseRaster::window`]).
    pub window: Option<PixelRect>,
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
    /// For a [`BaseRequest::window`]: the part of the `width` × `height`
    /// raster `straight_rgba` holds (`[x, y, w, h]`), the same pixels the
    /// whole raster has there.
    pub window: Option<[u32; 4]>,
}

type Job = (BaseRequest, Sender<Result<Option<BaseRaster>, String>>);

/// The base raster worker: a second renderer with the app's fonts, owned by
/// one thread. Its device is its own (rasters return as CPU pixels), made on
/// the first request and released after [`RASTER_IDLE`] without one: most
/// projects never ask, and a clone of the viewer's first wgpu device kept
/// that device, its queue and programs alive through every renewal.
pub struct LayerRasters {
    jobs: Mutex<Sender<Job>>,
}

/// How long the base raster renderer outlives its last request.
const RASTER_IDLE: Duration = Duration::from_secs(30);

impl LayerRasters {
    pub fn start(fonts: Option<pm_render::text::host::RenderFonts>) -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("pm-layer-rasters".into())
            .spawn(move || {
                let mut worker: Option<(Renderer, Option<u64>)> = None;
                loop {
                    let job = if worker.is_some() {
                        match rx.recv_timeout(RASTER_IDLE) {
                            Ok(job) => job,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                objc2::rc::autoreleasepool(|_| worker = None);
                                pm_memory::pressure_relief();
                                continue;
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match rx.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        }
                    };
                    let (request, reply) = job;
                    // This thread's implicit pool is never drained.
                    objc2::rc::autoreleasepool(|_| {
                        if worker.is_none() {
                            match pm_gpu::Gpu::new("pm-layer-rasters") {
                                Ok(gpu) => {
                                    let mut renderer = Renderer::new(gpu.device, gpu.queue);
                                    let _ = renderer
                                        .set_definitions(&pm_render::all_builtin_definitions());
                                    renderer.set_captions_hidden(false);
                                    worker = Some((renderer, None));
                                }
                                Err(error) => {
                                    let _ = reply
                                        .send(Err(format!("No GPU for layer rasters: {error}")));
                                    return;
                                }
                            }
                        }
                        let (renderer, generation) = worker.as_mut().expect("raster renderer");
                        if let Some(fonts) = &fonts {
                            fonts.sync(renderer, generation);
                        }
                        let _ = reply.send(paint(renderer, request));
                    });
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
        .layer_raster_window(
            &request.document,
            &request.composition,
            &request.layer,
            content.as_ref(),
            request.seconds,
            request.scale,
            pm_protocol::color::Precision::Float16,
            request.window.map(|w| [w.x, w.y, w.width, w.height]),
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
        window: None,
    };
    // The whole raster (as without a crop) with only the window painted: the
    // window of what `place_raster` makes of the whole canvas.
    if let Some(window) = request.window {
        let (cw, ch) = (raster.canvas.0.max(1), raster.canvas.1.max(1));
        let painted =
            pm_export::raster::encode_rgba8_f32(&raster.pixels.pixels, SnapshotPixels::Straight);
        let (pixels, x, y, w, h) = match pm_export::raster::place_raster(
            &painted,
            raster.pixels.crop,
            (cw, ch),
            Some(window),
        ) {
            Some((pixels, w, h)) => (pixels, window.x.min(cw), window.y.min(ch), w, h),
            None => (Vec::new(), window.x.min(cw), window.y.min(ch), 0, 0),
        };
        return Ok(Some(BaseRaster {
            window: Some([x, y, w, h]),
            ..geometry(cw, ch, pixels, [0., 0., 1., 1.])
        }));
    }
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
    let painted =
        pm_export::raster::encode_rgba8_f32(&raster.pixels.pixels, SnapshotPixels::Straight);
    let window = if raster.crops { request.crop } else { None };
    Ok(
        pm_export::raster::place_raster(&painted, raster.pixels.crop, raster.canvas, window).map(
            |(pixels, w, h)| {
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
            },
        ),
    )
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

/// The rasters one revision's frames draw from, newest last.
const MAX_BYTES: u64 = 256 << 20;
/// Answers kept at most (a pass-through costs no pixels but still a key).
const MAX_ENTRIES: usize = 16_384;
/// A need asked this long ago and never answered is asked again; a batch
/// unanswered this long no longer holds the next one back.
const ASK_AGAIN: Duration = Duration::from_secs(5);
/// What one look-ahead batch aims to cost the realm: answers land often and
/// the realm stays free for everything else it runs.
const BATCH_TARGET: Duration = Duration::from_millis(100);
/// Frames one look-ahead batch asks for at most.
const MAX_AHEAD: usize = 12;

/// How a frame being drawn treats a layer without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Late {
    /// Wait for it: the frame fails with `CompatPending` (paused frames, exact).
    #[default]
    Wait,
    /// Draw its nearest answer, or the layer without its override when it
    /// has none (playing and scrubbing: playback never freezes for an
    /// extension; the frame is marked late and redrawn exact).
    Nearest,
}

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
    /// Each layer's answered frames, for a late frame's nearest answer.
    by_layer: HashMap<(String, String), BTreeMap<i64, NeedKey>>,
    /// Asked of the realm and not answered yet, and when.
    asked: HashMap<NeedKey, Instant>,
    /// Needs whose override failed, with its message: the layer fails in
    /// their frames (drawn as nothing and reported, never waited for) and is
    /// not asked again until the revision or wrappers change.
    failed: HashMap<NeedKey, String>,
    /// Needs the realm could not answer this time: asked again once their
    /// backoff passes. Their frames wait (or draw the nearest answer) and are
    /// never drawn without the layer.
    unavailable: HashMap<NeedKey, Unavailable>,
    /// The frame being drawn: how it treats late layers, and which it drew
    /// from a nearest answer.
    late: Late,
    stood_in: Vec<CompatNeed>,
    /// The viewer's batch the realm is answering: when it went and its size.
    /// One at a time, so the realm answers what the viewer needs now and
    /// never works through a backlog of frames already gone by.
    in_flight: Option<(Instant, usize)>,
    /// The newest needs waiting for that batch; newer needs replace them.
    queued: Vec<CompatNeed>,
    /// Measured realm time per need (round trip of a batch over its size).
    per_need: Option<Duration>,
    /// Override layers in the last look-ahead frame, and the last frame it asked.
    layers: usize,
    /// The last frame the look-ahead asked for, and the scale it asked at:
    /// answers at another scale cover nothing (a windowed raster stands in
    /// only at its own scale).
    horizon: Option<(i64, u64)>,
}
struct Unavailable {
    attempts: u32,
    again_at: Instant,
}
/// The wait before asking again after `attempts` transient failures.
fn backoff(attempts: u32) -> Duration {
    (FIRST_RETRY * 2u32.saturating_pow(attempts.saturating_sub(1))).min(LAST_RETRY)
}
const FIRST_RETRY: Duration = Duration::from_millis(100);
const LAST_RETRY: Duration = Duration::from_secs(5);

type Redraw = Arc<dyn Fn() + Send + Sync>;

/// Redraws the viewers at each pending retry's time, earliest first, so
/// every unavailable need is asked again at its own backoff (a frame asks
/// for what is due). One thread per store while retries are pending.
#[derive(Default)]
struct Wakes(Arc<WakeQueue>);
#[derive(Default)]
struct WakeQueue {
    state: Mutex<WakeState>,
    changed: Condvar,
}
#[derive(Default)]
struct WakeState {
    at: BinaryHeap<Reverse<Instant>>,
    redraw: Option<Redraw>,
    running: bool,
    closed: bool,
}
impl Wakes {
    fn at(&self, deadlines: Vec<Instant>, redraw: Redraw) {
        let mut state = self.0.state.lock().unwrap();
        state.at.extend(deadlines.into_iter().map(Reverse));
        state.redraw = Some(redraw);
        if state.running {
            self.0.changed.notify_one();
            return;
        }
        state.running = true;
        drop(state);
        let queue = self.0.clone();
        let spawned = std::thread::Builder::new()
            .name("pm-compat-retry".into())
            .spawn(move || queue.run());
        if spawned.is_err() {
            self.0.state.lock().unwrap().running = false;
        }
    }
}
impl WakeQueue {
    fn run(&self) {
        let mut state = self.state.lock().unwrap();
        while !state.closed {
            let Some(&Reverse(next)) = state.at.peek() else {
                break;
            };
            let now = Instant::now();
            if next > now {
                state = self.changed.wait_timeout(state, next - now).unwrap().0;
                continue;
            }
            while state.at.peek().is_some_and(|Reverse(at)| *at <= now) {
                state.at.pop();
            }
            let redraw = state.redraw.clone();
            drop(state);
            if let Some(redraw) = redraw {
                redraw();
            }
            state = self.state.lock().unwrap();
        }
        state.running = false;
    }
}
impl Drop for Wakes {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().closed = true;
        self.0.changed.notify_all();
    }
}
/// Whether `answer` (to the need `key`) can stand in for a frame at `scale`
/// and `crop`: a whole raster always can; one holding only a window
/// ([`CompatRaster::window`]) only for a crop inside its window (elsewhere it
/// is transparent, and the frame would show holes), at its own scale or a
/// coarser frame's (playback at a reduced resolution): the frame's crop is
/// then taken to the answer's pixels, and the finer answer is sampled down,
/// never up.
fn stands_in(key: &NeedKey, answer: &CompatAnswer, scale: f64, crop: Option<PixelRect>) -> bool {
    let CompatAnswer::Raster(raster) = answer else {
        return true;
    };
    if raster.window.is_none() {
        return true;
    }
    let [x, y, w, h] = raster.held();
    let answered = f64::from_bits(key.3);
    if key.3 != scale.to_bits() && !(scale > 0. && answered > scale) {
        return false;
    }
    let k = if key.3 == scale.to_bits() {
        1.
    } else {
        answered / scale
    };
    crop.is_some_and(|c| {
        let at = |v: u32| (f64::from(v) * k).floor() as u32;
        let end = |v: u32| (f64::from(v) * k).ceil() as u32;
        let (left, top) = (at(c.x), at(c.y));
        let (right, bottom) = (
            end(c.x.saturating_add(c.width)).min(raster.width),
            end(c.y.saturating_add(c.height)).min(raster.height),
        );
        x <= left && y <= top && x + w >= right && y + h >= bottom
    })
}
impl Answers {
    /// A late layer's stand-in: its nearest answered frame at or before
    /// `ticks`, in this revision only (a later frame is content not reached
    /// yet; an earlier revision's is content the edit may have changed),
    /// that can stand in for a frame at `scale` and `crop`.
    fn nearest(
        &self,
        composition: &str,
        layer: &str,
        ticks: i64,
        scale: f64,
        crop: Option<PixelRect>,
    ) -> Option<&CompatAnswer> {
        let times = self
            .by_layer
            .get(&(composition.to_owned(), layer.to_owned()))?;
        times
            .range(..=ticks)
            .rev()
            .filter_map(|(_, key)| Some((key, self.map.get(key)?)))
            .find(|(key, answer)| stands_in(key, answer, scale, crop))
            .map(|(_, answer)| answer)
    }
    /// A late layer with no answered frame at or before `ticks` that stands
    /// in: its nearest answered frame after it that does (playback starting
    /// before the layer's first answer, a seek back).
    fn nearest_after(
        &self,
        composition: &str,
        layer: &str,
        ticks: i64,
        scale: f64,
        crop: Option<PixelRect>,
    ) -> Option<&CompatAnswer> {
        self.by_layer
            .get(&(composition.to_owned(), layer.to_owned()))?
            .range(ticks..)
            .filter_map(|(_, key)| Some((key, self.map.get(key)?)))
            .find(|(key, answer)| stands_in(key, answer, scale, crop))
            .map(|(_, answer)| answer)
    }
    fn forget(&mut self, key: &NeedKey) {
        self.map.remove(key);
        let layer = (key.0.clone(), key.1.clone());
        if let Some(times) = self.by_layer.get_mut(&layer) {
            if times.get(&key.2) == Some(key) {
                times.remove(&key.2);
            }
            if times.is_empty() {
                self.by_layer.remove(&layer);
            }
        }
    }
    /// Whether a viewer batch may go now (none in flight, or it was lost).
    fn free(&self) -> bool {
        self.in_flight
            .is_none_or(|(at, _)| at.elapsed() >= ASK_AGAIN)
    }
    /// The needs not answered, failed or asked recently.
    fn fresh(&self, needs: Vec<CompatNeed>) -> Vec<CompatNeed> {
        let mut seen = HashSet::new();
        needs
            .into_iter()
            .filter(|need| {
                let key = need_key(need);
                !self.map.contains_key(&key)
                    && !self.failed.contains_key(&key)
                    && self
                        .unavailable
                        .get(&key)
                        .is_none_or(|u| Instant::now() >= u.again_at)
                    && self
                        .asked
                        .get(&key)
                        .is_none_or(|at| at.elapsed() >= ASK_AGAIN)
                    && seen.insert(key)
            })
            .collect()
    }
}

/// Where needs go: the extension realm of the window showing the document,
/// with the revision and wrapper generation they are asked at.
pub type CompatSink = Arc<dyn Fn(u64, u64, Vec<CompatNeed>) + Send + Sync>;

/// What the realm answered for one need.
pub enum Reply {
    Draw(CompatAnswer),
    /// The override threw or returned what cannot be drawn (its message):
    /// the same revision fails the same way, so it is not asked again.
    Failed(String),
    /// The realm could not answer this time (a readback failed, the realm
    /// restarted, the answer was lost): asked again after a backoff.
    Unavailable(String),
    /// The realm had not seen the revision yet.
    Retry,
}

/// When this process started (ns since the epoch), fixed at first use.
fn process_started() -> u128 {
    static STARTED: std::sync::OnceLock<u128> = std::sync::OnceLock::new();
    *STARTED.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    })
}

/// The viewer's raster override answers for the open document.
#[derive(Default)]
pub struct CompatStore {
    answers: Mutex<Answers>,
    sink: Mutex<Option<CompatSink>>,
    /// Redraws the viewers that wait for answers when answers land.
    redraw: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Redraws every viewer of the document when overrides start or stop
    /// answering: each of its pictures is another one.
    rewrapped: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Redraws again when each unavailable need's backoff ends.
    wakes: Wakes,
    /// The code the wrappers come from (`set_code`); 0 while unknown.
    code: std::sync::atomic::AtomicU64,
}

impl CompatStore {
    /// Identity of the extension code the wrappers come from, the same on
    /// every launch while that code is unchanged.
    pub fn set_code(&self, code: u64) {
        self.code.store(code, std::sync::atomic::Ordering::Release);
    }
    /// What the disk render cache keys override frames by: the wrappers'
    /// code and their generation in this realm. Frames rendered with the
    /// same code in the same generation are kept across launches; without a
    /// code identity they belong to this process alone.
    pub fn cache_identity(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        match self.code.load(std::sync::atomic::Ordering::Acquire) {
            0 => (std::process::id(), process_started()).hash(&mut hash),
            code => code.hash(&mut hash),
        }
        self.epoch().hash(&mut hash);
        hash.finish()
    }
    /// Whether an override is registered, the generation of wrappers
    /// answering, and the document revision frames are drawn at. A new
    /// revision or generation forgets every answer.
    pub fn set_state(&self, producing: bool, revision: u64, epoch: u64) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision || answers.epoch != epoch {
            // An edit or new wrappers: no earlier answer stands in, so a
            // frame never shows content the edit changed.
            let old = std::mem::take(&mut *answers);
            *answers = Answers {
                revision,
                epoch,
                frame: old.frame,
                late: old.late,
                stood_in: old.stood_in,
                per_need: old.per_need,
                layers: old.layers,
                ..Answers::default()
            };
        }
        let changed = answers.producing != producing;
        answers.producing = producing;
        drop(answers);
        if changed && let Some(redraw) = self.rewrapped.lock().unwrap().clone() {
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
    pub(crate) fn set_rewrapped(&self, redraw: Arc<dyn Fn() + Send + Sync>) {
        *self.rewrapped.lock().unwrap() = Some(redraw);
    }
    /// One frame of the open composition, in ticks (0: don't ask ahead).
    pub fn set_frame(&self, ticks: i64) {
        self.answers.lock().unwrap().frame = ticks.max(0);
    }
    /// A paused or scrubbed frame's missing answers, asked of the realm.
    /// While a batch is in flight they wait, replacing older waiting needs.
    pub(crate) fn request(&self, needs: Vec<CompatNeed>) {
        let mut answers = self.answers.lock().unwrap();
        let fresh = answers.fresh(needs);
        if fresh.is_empty() {
            return;
        }
        if !answers.free() {
            answers.queued = fresh;
            return;
        }
        self.dispatch(answers, fresh);
    }
    /// Needs asked ahead of the picture that will want them (a paused
    /// picture's refinement): sent as [`CompatStore::request`] sends them
    /// when no batch is in flight, else queued behind the needs already
    /// waiting, never in their place, so the picture on screen is answered
    /// first.
    pub(crate) fn request_behind(&self, needs: Vec<CompatNeed>) {
        let mut answers = self.answers.lock().unwrap();
        let fresh = answers.fresh(needs);
        if fresh.is_empty() {
            return;
        }
        if !answers.free() {
            let waiting: HashSet<_> = answers.queued.iter().map(need_key).collect();
            answers.queued.extend(
                fresh
                    .into_iter()
                    .filter(|n| !waiting.contains(&need_key(n))),
            );
            return;
        }
        self.dispatch(answers, fresh);
    }
    /// Playback's look-ahead at the playhead `now` (ticks, playing at
    /// `speed`): whether to ask now, how far ahead the answers will land (the
    /// realm's measured time for a batch) and how many frames the batch
    /// covers, about [`BATCH_TARGET`] of realm time. `None` while a batch is
    /// in flight or the last one still covers the frames ahead.
    pub(crate) fn ahead(&self, now: i64, speed: f64, scale: f64) -> Option<(Duration, usize)> {
        let answers = self.answers.lock().unwrap();
        if !answers.producing || !answers.free() || !answers.queued.is_empty() {
            return None;
        }
        let Some(per_need) = answers.per_need else {
            // Not measured yet: one frame, a frame ahead.
            return Some((Duration::ZERO, 1));
        };
        let per_frame = per_need * answers.layers.max(1) as u32;
        let frames = (BATCH_TARGET.as_secs_f64() / per_frame.as_secs_f64().max(1e-6)) as usize;
        let frames = frames.clamp(1, MAX_AHEAD);
        let lead = per_frame * frames as u32;
        // The last batch still covers the playhead for longer than a new one
        // would take to land.
        let lead_ticks = lead.as_secs_f64() * speed * TICKS_PER_SECOND as f64;
        if let Some((horizon, at)) = answers.horizon
            && at == scale.to_bits()
            && (horizon - now) as f64 * speed.signum() > lead_ticks.abs()
        {
            return None;
        }
        Some((lead, frames))
    }
    /// The needs of the frame at `at`, ahead of the playhead, and the
    /// `frames - 1` frames after it (`step` ticks apart, negative in
    /// reverse), as one batch.
    pub(crate) fn request_ahead(&self, needs: Vec<CompatNeed>, at: i64, frames: usize, step: i64) {
        let frames = frames.max(1) as i64;
        let mut answers = self.answers.lock().unwrap();
        let scale = needs.first().map_or(0, |need| need.scale.to_bits());
        answers.horizon = Some((at + (frames - 1) * step, scale));
        if !needs.is_empty() {
            answers.layers = needs.len();
        }
        let all = (0..frames)
            .flat_map(|k| {
                needs.iter().map(move |need| {
                    let mut next = need.clone();
                    next.time = pm_protocol::common::Ticks::saturating_from_i128(
                        i128::from(need.time.get()) + i128::from(k * step),
                    );
                    next
                })
            })
            .collect();
        let fresh = answers.fresh(all);
        if fresh.is_empty() || !answers.free() {
            return;
        }
        self.dispatch(answers, fresh);
    }
    fn dispatch(&self, mut answers: std::sync::MutexGuard<'_, Answers>, needs: Vec<CompatNeed>) {
        let now = Instant::now();
        if answers.asked.len() > MAX_ENTRIES {
            answers.asked.retain(|_, at| at.elapsed() < ASK_AGAIN);
        }
        for need in &needs {
            answers.asked.insert(need_key(need), now);
        }
        answers.in_flight = Some((now, needs.len()));
        let (revision, epoch) = (answers.revision, answers.epoch);
        drop(answers);
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            sink(revision, epoch, needs);
        } else {
            self.answers.lock().unwrap().in_flight = None;
        }
    }
    /// The realm's measured time per need, the batch in flight (its age and
    /// size), the needs waiting behind it and the override layers of the
    /// last look-ahead frame: a traced viewer's after-frame row.
    pub(crate) fn trace(&self) -> serde_json::Value {
        let answers = self.answers.lock().unwrap();
        serde_json::json!({
            "per_need_ms": answers.per_need.map(|d| d.as_secs_f64() * 1e3),
            "in_flight_ms": answers.in_flight.map(|(at, _)| at.elapsed().as_secs_f64() * 1e3),
            "in_flight": answers.in_flight.map(|(_, n)| n),
            "queued": answers.queued.len(),
            "layers": answers.layers,
        })
    }
    /// One frame of the open composition, in ticks.
    pub(crate) fn frame_ticks(&self) -> i64 {
        self.answers.lock().unwrap().frame
    }
    /// The frame about to be drawn: playing and scrubbed frames draw late
    /// layers from their nearest answer (or without the override when there
    /// is none), paused frames wait.
    pub(crate) fn begin_frame(&self, moment: crate::policy::Moment) {
        let mut answers = self.answers.lock().unwrap();
        answers.late = match moment {
            crate::policy::Moment::Playing | crate::policy::Moment::Scrub => Late::Nearest,
            crate::policy::Moment::Paused => Late::Wait,
        };
        answers.stood_in.clear();
    }
    /// The needs the frame just drawn showed from a nearest answer: the
    /// frame is not exact, and these are still to be answered.
    pub(crate) fn end_frame(&self) -> Vec<CompatNeed> {
        let mut answers = self.answers.lock().unwrap();
        answers.late = Late::Wait;
        std::mem::take(&mut answers.stood_in)
    }
    /// The realm's answers for `revision` and `epoch`; stale ones are dropped.
    pub fn answer_all(&self, revision: u64, epoch: u64, answered: Vec<(CompatNeed, Reply)>) {
        let mut answers = self.answers.lock().unwrap();
        if answers.revision != revision || answers.epoch != epoch {
            return;
        }
        if let Some((at, n)) = answers.in_flight.take() {
            let per_need = at.elapsed() / n.max(1) as u32;
            answers.per_need = Some(match answers.per_need {
                Some(before) => (before * 3 + per_need) / 4,
                None => per_need,
            });
        }
        let mut drew = false;
        let mut again = Vec::new();
        for (need, reply) in answered {
            let key = need_key(&need);
            answers.asked.remove(&key);
            if !matches!(reply, Reply::Unavailable(_)) {
                answers.unavailable.remove(&key);
            }
            let answer = match reply {
                Reply::Draw(answer) => answer,
                Reply::Failed(message) => {
                    if answers.failed.len() >= MAX_ENTRIES {
                        answers.failed.clear();
                    }
                    answers.failed.insert(key, message);
                    drew = true;
                    continue;
                }
                // Asked again by the next frame that needs it.
                Reply::Retry => {
                    drew = true;
                    continue;
                }
                // Asked again by the first frame that needs it after the
                // backoff; a redraw then makes sure one does.
                Reply::Unavailable(_) => {
                    if answers.unavailable.len() >= MAX_ENTRIES {
                        answers.unavailable.clear();
                    }
                    let attempts = answers.unavailable.get(&key).map_or(0, |u| u.attempts) + 1;
                    let again_at = Instant::now() + backoff(attempts);
                    answers
                        .unavailable
                        .insert(key, Unavailable { attempts, again_at });
                    again.push(again_at);
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
            answers
                .by_layer
                .entry((key.0.clone(), key.1.clone()))
                .or_default()
                .insert(key.2, key.clone());
            answers.map.insert(key, answer);
            drew = true;
        }
        while answers.bytes > MAX_BYTES || answers.map.len() > MAX_ENTRIES {
            let Some((old, bytes)) = answers.order.pop_front() else {
                break;
            };
            answers.forget(&old);
            answers.bytes -= bytes;
        }
        // The needs that waited for this batch go now, less what it answered.
        let queued = std::mem::take(&mut answers.queued);
        let queued = answers.fresh(queued);
        if queued.is_empty() {
            drop(answers);
        } else {
            self.dispatch(answers, queued);
        }
        if let Some(redraw) = self.redraw.lock().unwrap().clone() {
            if drew {
                redraw();
            }
            if !again.is_empty() {
                self.wakes.at(again, redraw);
            }
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
    redraw: Mutex<Option<Arc<Redrawn>>>,
}
/// What a document's store asks of its viewers (`doc_id`; `true`: its
/// overrides started or stopped answering, so every picture of it is
/// drawn again; `false`: answers landed, for the pictures waiting on them).
pub(crate) type Redrawn = dyn Fn(u64, bool) + Send + Sync;
impl CompatStores {
    pub(crate) fn set_redraw(&self, redraw: Arc<Redrawn>) {
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
                    let answers = redraw.clone();
                    store.set_redraw(Arc::new(move || answers(doc_id, false)));
                    store.set_rewrapped(Arc::new(move || redraw(doc_id, true)));
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
        let mut answers = self.answers.lock().unwrap();
        let key = key(composition, layer, ticks, scale, crop);
        if let Some(answer) = answers.map.get(&key) {
            return answer.clone();
        }
        // A failed override fails its layer: reported, not waited for.
        if let Some(message) = answers.failed.get(&key) {
            return CompatAnswer::Failed(message.clone());
        }
        // Not answered yet, or the realm couldn't answer this time (asked
        // again after its backoff). A paused frame waits (the viewer says
        // what for after a few seconds). A playing or scrubbed one never
        // does: it draws the layer's nearest earlier answer, else its nearest
        // later one, else the layer without its override, and is redrawn
        // exact when the answer lands.
        if answers.late == Late::Wait {
            return CompatAnswer::Missing;
        }
        let stand_in = answers
            .nearest(composition, layer, ticks, scale, crop)
            .or_else(|| answers.nearest_after(composition, layer, ticks, scale, crop))
            .cloned()
            .unwrap_or(CompatAnswer::Native);
        answers.stood_in.push(CompatNeed {
            composition: composition.into(),
            layer: layer.into(),
            time: pm_render::compat::ticks_of(time),
            source,
            scale,
            crop,
        });
        stand_in
    }
}

/// A document's answers as a renderer beside the viewer's reads them (a
/// paused viewer's settled picture, `crate::settle`): the exact answer or
/// none, whatever the frame the viewer's own renderer is drawing lets
/// stand in.
pub(crate) struct ExactAnswers(pub Arc<CompatStore>);
impl CompatRasters for ExactAnswers {
    fn producing(&self, source: CompatSource) -> bool {
        self.0.producing(source)
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
        let answers = self.0.answers.lock().unwrap();
        let key = key(composition, layer, ticks, scale, crop);
        match (answers.map.get(&key), answers.failed.get(&key)) {
            (Some(answer), _) => answer.clone(),
            (None, Some(message)) => CompatAnswer::Failed(message.clone()),
            (None, None) => CompatAnswer::Missing,
        }
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
    override_raster_window(width, height, None, premultiplied_rgba, size, anchor, uv)
}
/// [`override_raster`] of which `premultiplied_rgba` holds only `window`
/// (`[x, y, w, h]`; the rest is transparent and never sampled).
pub fn override_raster_window(
    width: u32,
    height: u32,
    window: Option<[u32; 4]>,
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
        window,
    }))
}
/// An override that returned nothing: the layer draws nothing (main skips it).
pub fn hidden_raster() -> CompatAnswer {
    override_raster(1, 1, vec![0; 4], [1., 1.], [0., 0.], [0., 0., 1., 1.])
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A window of a base raster is the whole raster's straight pixels there,
    /// with the whole raster's size and geometry.
    #[test]
    fn a_base_window_is_the_whole_raster_s_pixels_there() {
        let project = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let document = Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(project)),
            1,
        ));
        // The raster worker makes its own device on the first request.
        let rasters = LayerRasters::start(None).unwrap();
        let ask = |window: Option<PixelRect>| {
            rasters
                .raster(BaseRequest {
                    document: document.clone(),
                    composition: "Pshapes-primitives-root".into(),
                    layer: "L00d".into(),
                    content: None,
                    seconds: 1.25,
                    scale: 5.5,
                    crop: None,
                    window,
                })
                .unwrap()
                .unwrap()
        };
        let whole = ask(None);
        assert_eq!(whole.window, None);
        assert!(
            whole.straight_rgba.iter().any(|&b| b > 0),
            "the shape is inked"
        );
        let (w, h) = (whole.width, whole.height);
        for [x, y, ww, wh] in [
            [w / 4, h / 3, w / 2, h / 3],
            [w - 2, h - 1, 9, 9],
            [0, 0, 1, 1],
        ] {
            let part = ask(Some(PixelRect {
                x,
                y,
                width: ww,
                height: wh,
            }));
            let [px, py, pw, ph] = part.window.unwrap();
            assert_eq!([px, py], [x, y]);
            assert_eq!([pw, ph], [ww.min(w - x), wh.min(h - y)]);
            assert_eq!((part.width, part.height, part.uv), (w, h, whole.uv));
            assert_eq!((part.box_size, part.anchor), (whole.box_size, whole.anchor));
            let expected: Vec<u8> = (py..py + ph)
                .flat_map(|r| {
                    let at = ((r * w + px) * 4) as usize;
                    whole.straight_rgba[at..at + (pw * 4) as usize].to_vec()
                })
                .collect();
            assert_eq!(part.straight_rgba, expected, "window {x},{y}");
        }
    }
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
    /// Override frames on disk are keyed by the wrappers' code and their
    /// generation: the same code and generation in another launch (another
    /// store) find them; other code, another generation, or no known code
    /// (this process only) don't.
    #[test]
    fn override_cache_identity_follows_code_and_generation() {
        let launch = |code: u64, epoch: u64| {
            let store = CompatStore::default();
            store.set_code(code);
            store.set_state(true, 1, epoch);
            store.cache_identity()
        };
        assert_eq!(launch(7, 2), launch(7, 2));
        assert_ne!(launch(7, 2), launch(8, 2));
        assert_ne!(launch(7, 2), launch(7, 3));
        assert_ne!(launch(0, 2), launch(7, 2));
    }
    #[test]
    fn each_document_answers_from_its_own_window() {
        let stores = CompatStores::default();
        let (a, b) = (stores.document(1), stores.document(2));
        let (asked_a, asked_b) = (asking(&a), asking(&b));
        a.set_state(true, 7, 1);
        b.set_state(true, 7, 1);
        a.request(vec![need("x")]);
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
        store.request(vec![need("x")]);
        assert_eq!(*asked.lock().unwrap(), vec![(2, 1)]);
    }
    /// Needs asked ahead (a refinement's) wait behind the visible picture's
    /// while a batch runs: the next batch sends the visible ones first.
    #[test]
    fn needs_asked_ahead_never_displace_the_visible_pictures() {
        let store = CompatStore::default();
        let batches = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let log = batches.clone();
        store.set_sink(Some(Arc::new(move |_, _, needs: Vec<CompatNeed>| {
            log.lock()
                .unwrap()
                .push(needs.iter().map(|n| n.layer.as_str().to_owned()).collect())
        })));
        store.set_state(true, 7, 1);
        store.request(vec![need("running")]);
        // While that batch runs: the visible picture's need, then a
        // refinement's asked ahead.
        store.request(vec![need("visible")]);
        store.request_behind(vec![need("ahead")]);
        store.answer_all(7, 1, vec![(need("running"), draw(255))]);
        assert_eq!(
            batches.lock().unwrap().last().unwrap(),
            &vec!["visible".to_owned(), "ahead".to_owned()]
        );
        // With nothing in flight, asked ahead goes at once.
        let store = CompatStore::default();
        let asked = asking(&store);
        store.set_state(true, 7, 1);
        store.request_behind(vec![need("ahead")]);
        assert_eq!(*asked.lock().unwrap(), vec![(1, 1)]);
    }
    #[test]
    fn a_failed_override_fails_its_layer_without_asking_again() {
        let store = CompatStore::default();
        let asked = asking(&store);
        store.set_state(true, 7, 1);
        store.request(vec![need("x")]);
        store.answer_all(7, 1, vec![(need("x"), Reply::Failed("boom".into()))]);
        // The layer fails with the override's message; the frame does not
        // wait for an answer that will never come.
        assert!(matches!(answer(&store, "x"), CompatAnswer::Failed(m) if m == "boom"));
        store.request(vec![need("x")]);
        assert_eq!(asked.lock().unwrap().len(), 1);
        // A realm that had not caught up is asked again.
        store.request(vec![need("y")]);
        store.answer_all(7, 1, vec![(need("y"), Reply::Retry)]);
        store.request(vec![need("y")]);
        assert_eq!(asked.lock().unwrap().len(), 3);
        // An edit retries the failed one.
        store.set_state(true, 8, 1);
        store.request(vec![need("x")]);
        assert_eq!(asked.lock().unwrap().len(), 4);
    }
    /// A readback error, a restarted realm or a lost answer is asked again
    /// after a backoff (a redraw makes the frame ask), then draws; one that
    /// keeps failing is reported meanwhile but still asked again. Only an
    /// override's throw is sticky for the revision.
    #[test]
    fn a_transient_failure_is_asked_again_after_a_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let store = CompatStore::default();
        let asked = asking(&store);
        let redraws = Arc::new(AtomicUsize::new(0));
        let count = redraws.clone();
        store.set_redraw(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        let redrawn = |after: usize| {
            let start = Instant::now();
            while redraws.load(Ordering::SeqCst) <= after {
                assert!(start.elapsed() < Duration::from_secs(10), "no redraw");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        store.set_state(true, 7, 1);
        let unavailable = || Reply::Unavailable("readback".into());
        store.request(vec![need("x")]);
        store.answer_all(7, 1, vec![(need("x"), unavailable())]);
        // Not the layer's failure: the paused frame waits for it.
        assert!(matches!(answer(&store, "x"), CompatAnswer::Missing));
        store.request(vec![need("x")]);
        assert_eq!(asked.lock().unwrap().len(), 1, "not before the backoff");
        redrawn(redraws.load(Ordering::SeqCst));
        store.request(vec![need("x")]);
        assert_eq!(asked.lock().unwrap().len(), 2, "asked again");
        store.answer_all(7, 1, vec![(need("x"), draw(9))]);
        assert!(matches!(answer(&store, "x"), CompatAnswer::Raster(r) if r.data[0] == 9));

        // Failing every time: asked again after each backoff, never drawn
        // without the layer and never sticky.
        for attempt in 1..=4usize {
            let before = redraws.load(Ordering::SeqCst);
            store.request(vec![need("y")]);
            assert_eq!(asked.lock().unwrap().len(), 2 + attempt);
            store.answer_all(7, 1, vec![(need("y"), unavailable())]);
            assert!(
                matches!(answer(&store, "y"), CompatAnswer::Missing),
                "attempt {attempt}: the paused frame waits, the layer is not left out"
            );
            redrawn(before);
        }
        store.request(vec![need("y")]);
        assert_eq!(asked.lock().unwrap().len(), 7);
        // An override's throw is not asked again for this revision.
        store.answer_all(7, 1, vec![(need("y"), Reply::Failed("boom".into()))]);
        store.request(vec![need("y")]);
        assert_eq!(asked.lock().unwrap().len(), 7);
    }
    /// Two layers the realm could not answer wait different backoffs: each
    /// gets a redraw when its own wait ends and is asked again then, even
    /// after the earlier one was answered (no wake is lost to the shortest).
    #[test]
    fn every_pending_retry_wakes_at_its_own_time() {
        let store = CompatStore::default();
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        store.set_sink(Some(Arc::new(move |_, _, needs: Vec<CompatNeed>| {
            log.lock()
                .unwrap()
                .extend(needs.iter().map(|n| n.layer.as_str().to_owned()));
        })));
        let redraws = Arc::new(Mutex::new(Vec::<Instant>::new()));
        let times = redraws.clone();
        store.set_redraw(Arc::new(move || times.lock().unwrap().push(Instant::now())));
        store.set_state(true, 7, 1);
        let unavailable = || Reply::Unavailable("readback".into());
        // "slow" has failed once already, so its next wait is twice as long.
        store.answer_all(7, 1, vec![(need("slow"), unavailable())]);
        store.request(vec![need("fast"), need("slow")]);
        store.answer_all(
            7,
            1,
            vec![(need("fast"), unavailable()), (need("slow"), unavailable())],
        );
        let due = |layer: &str| {
            let answers = store.answers.lock().unwrap();
            answers.unavailable[&need_key(&need(layer))].again_at
        };
        let (fast, slow) = (due("fast"), due("slow"));
        assert!(slow > fast);
        let redrawn_after = |at: Instant| {
            let start = Instant::now();
            while !redraws.lock().unwrap().iter().any(|t| *t >= at) {
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "no redraw when the wait ended"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        redrawn_after(fast);
        asked.lock().unwrap().clear();
        store.request(vec![need("fast"), need("slow")]);
        if Instant::now() < slow {
            assert_eq!(*asked.lock().unwrap(), ["fast"]);
        }
        store.answer_all(7, 1, vec![(need("fast"), draw(1))]);
        redrawn_after(slow);
        asked.lock().unwrap().clear();
        store.request(vec![need("fast"), need("slow")]);
        assert_eq!(*asked.lock().unwrap(), ["slow"], "asked at its own time");
    }
    fn at(layer: &str, seconds: f64) -> CompatNeed {
        let mut n = need(layer);
        n.time = pm_render::compat::ticks_of(seconds);
        n
    }
    fn answer_at(store: &CompatStore, layer: &str, seconds: f64) -> CompatAnswer {
        store.answer("c", layer, seconds, 1., CompatSource::RasterOverride, None)
    }
    fn byte_of(answer: CompatAnswer) -> Option<u8> {
        match answer {
            CompatAnswer::Raster(r) => Some(r.data[0]),
            _ => None,
        }
    }
    /// A raster holding only its need's crop window stands in only for a
    /// frame at its scale whose crop it holds: anything else would sample its
    /// transparent rest (review: panning across a crop bucket showed holes),
    /// so that frame draws the layer without its override, as for no answer.
    #[test]
    fn a_windowed_answer_stands_in_only_for_crops_it_holds() {
        use crate::policy::Moment;
        let crop = |x, y, width, height| {
            Some(PixelRect {
                x,
                y,
                width,
                height,
            })
        };
        let windowed = |window| {
            Reply::Draw(override_raster_window(
                100,
                80,
                Some(window),
                vec![7; 40 * 30 * 4],
                [100., 80.],
                [0., 0.],
                [0., 0., 1., 1.],
            ))
        };
        let store = CompatStore::default();
        store.set_state(true, 7, 1);
        let mut first = at("x", 1.);
        first.crop = crop(20, 10, 40, 30);
        store.answer_all(7, 1, vec![(first, windowed([20, 10, 40, 30]))]);
        let ask = |crop, scale| {
            let t = 1.5;
            store.answer("c", "x", t, scale, CompatSource::RasterOverride, crop)
        };
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(ask(crop(20, 10, 40, 30), 1.)), Some(7));
        assert_eq!(byte_of(ask(crop(25, 12, 30, 20), 1.)), Some(7));
        // A crop the window ends inside of at the raster's edge is not held.
        assert!(matches!(
            ask(crop(60, 40, 256, 256), 1.),
            CompatAnswer::Native
        ));
        assert!(matches!(
            ask(crop(10, 10, 40, 30), 1.),
            CompatAnswer::Native
        ));
        assert!(matches!(ask(None, 1.), CompatAnswer::Native));
        assert!(matches!(
            ask(crop(20, 10, 40, 30), 2.),
            CompatAnswer::Native
        ));
        // A coarser frame (playback at half resolution) draws the finer
        // window when its crop, taken to the window's pixels, lies inside.
        assert_eq!(byte_of(ask(crop(10, 5, 20, 15), 0.5)), Some(7));
        assert!(matches!(ask(crop(5, 5, 20, 15), 0.5), CompatAnswer::Native));
        store.end_frame();
        // A whole raster stands in for any crop and scale, as before.
        let mut whole = at("x", 1.2);
        whole.crop = crop(0, 0, 256, 256);
        store.answer_all(7, 1, vec![(whole, draw(9))]);
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(ask(crop(10, 10, 40, 30), 2.)), Some(9));
        store.end_frame();
    }
    #[test]
    fn a_playing_frame_never_waits_for_an_extension_layer() {
        use crate::policy::Moment;
        let store = CompatStore::default();
        store.set_state(true, 7, 1);
        store.answer_all(7, 1, vec![(at("x", 1.), draw(10)), (at("x", 2.), draw(20))]);
        // Paused frames wait for their own answer.
        store.begin_frame(Moment::Paused);
        assert!(matches!(answer_at(&store, "x", 1.4), CompatAnswer::Missing));
        assert!(store.end_frame().is_empty());
        // Playing and scrubbed frames draw the nearest earlier answered frame
        // and say so: never one the playhead hasn't reached.
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 1.4)), Some(10));
        assert_eq!(byte_of(answer_at(&store, "x", 1.9)), Some(10));
        assert_eq!(byte_of(answer_at(&store, "x", 2.0)), Some(20));
        let late = store.end_frame();
        assert_eq!(late.len(), 2, "the exact frame is not late");
        // Playback never freezes for an extension. Before a layer's first
        // answer the nearest later one stands in; a layer never answered is
        // drawn without its override. Both are late: redrawn exact.
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 0.5)), Some(10));
        assert!(matches!(answer_at(&store, "y", 1.), CompatAnswer::Native));
        assert_eq!(store.end_frame().len(), 2);
        // One the realm keeps failing to answer draws its nearest answer.
        store.answer_all(
            7,
            1,
            vec![(at("x", 1.5), Reply::Unavailable("busy".into()))],
        );
        store.answer_all(
            7,
            1,
            vec![(at("x", 1.5), Reply::Unavailable("busy".into()))],
        );
        store.answer_all(
            7,
            1,
            vec![(at("x", 1.5), Reply::Unavailable("busy".into()))],
        );
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 1.5)), Some(10));
        assert_eq!(store.end_frame().len(), 1);
        store.begin_frame(Moment::Scrub);
        assert!(matches!(answer_at(&store, "y", 1.), CompatAnswer::Native));
        assert_eq!(store.end_frame().len(), 1);
        // A failed override still fails its layer, playing or not.
        store.answer_all(7, 1, vec![(at("x", 3.), Reply::Failed("boom".into()))]);
        store.begin_frame(Moment::Playing);
        assert!(matches!(
            answer_at(&store, "x", 3.),
            CompatAnswer::Failed(_)
        ));
        store.end_frame();
        // After an edit no earlier revision's answer stands in: a paused
        // frame waits for this revision's answer, a playing one draws the
        // layer without its override meanwhile.
        store.set_state(true, 8, 1);
        store.begin_frame(Moment::Paused);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Missing));
        store.end_frame();
        store.begin_frame(Moment::Playing);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Native));
        store.end_frame();
        store.answer_all(8, 1, vec![(at("x", 1.), draw(30))]);
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 1.5)), Some(30));
        store.end_frame();
        // New wrappers neither.
        store.set_state(true, 8, 2);
        store.begin_frame(Moment::Paused);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Missing));
        store.end_frame();
        store.begin_frame(Moment::Scrub);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Native));
        store.end_frame();
    }
    #[test]
    fn one_batch_at_a_time_and_the_newest_needs_go_next() {
        let store = CompatStore::default();
        let asked = Arc::new(Mutex::new(vec![]));
        let log = asked.clone();
        store.set_sink(Some(Arc::new(move |_, _, needs: Vec<CompatNeed>| {
            log.lock().unwrap().push(needs);
        })));
        store.set_state(true, 7, 1);
        store.request(vec![at("x", 1.)]);
        // Frames gone by while the realm answers are never asked.
        store.request(vec![at("x", 2.)]);
        store.request(vec![at("x", 3.), at("y", 3.)]);
        assert_eq!(asked.lock().unwrap().len(), 1);
        store.answer_all(7, 1, vec![(at("x", 1.), draw(1))]);
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[1], vec![at("x", 3.), at("y", 3.)]);
    }
    #[test]
    fn playback_asks_ahead_by_the_realms_measured_time() {
        let store = CompatStore::default();
        let asked = asking(&store);
        store.set_state(true, 7, 1);
        // Unmeasured: one frame.
        assert_eq!(store.ahead(0, 1., 1.), Some((Duration::ZERO, 1)));
        let frame = 29_400_000; // 24 fps
        store.request_ahead(vec![at("x", 1.), at("y", 1.)], ticks(1.), 1, frame);
        assert_eq!(*asked.lock().unwrap(), vec![(1, 2)]);
        // In flight: nothing more.
        assert_eq!(store.ahead(0, 1., 1.), None);
        std::thread::sleep(Duration::from_millis(20));
        store.answer_all(7, 1, vec![(at("x", 1.), draw(1)), (at("y", 1.), draw(1))]);
        // About 10 ms a need, two layers: 20 ms a frame, five frames a batch.
        let (lead, frames) = store.ahead(ticks(1.), 1., 1.).unwrap();
        assert!((3..=5).contains(&frames), "{frames}");
        assert!(lead >= Duration::from_millis(60), "{lead:?}");
        store.request_ahead(vec![at("x", 2.), at("y", 2.)], ticks(2.), frames, frame);
        assert_eq!(asked.lock().unwrap()[1], (1, 2 * frames));
        store.answer_all(7, 1, vec![]);
        // The batch covers the frames ahead: nothing new until the playhead nears its end.
        assert_eq!(store.ahead(ticks(1.5), 1., 1.), None);
        assert!(
            store
                .ahead(ticks(2.) + frames as i64 * frame, 1., 1.)
                .is_some()
        );
        // Playback that changes scale (Auto going to half resolution) asks
        // again at once: the batch at the old scale covers none of its frames.
        assert!(store.ahead(ticks(1.5), 1., 0.5).is_some());
    }
    fn ticks(seconds: f64) -> i64 {
        pm_render::compat::ticks_of(seconds).get()
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
