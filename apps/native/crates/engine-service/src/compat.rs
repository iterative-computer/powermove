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
//!   (`CompatPending`); a playing or scrubbing frame draws a late layer from
//!   its nearest answer (plan 3.6.4: "a late override reuses its last-good
//!   surface") and is redrawn exact when the answer lands. The viewer never
//!   blocks on JavaScript, and never stops showing frames for it.
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
        fonts: Option<pm_render::text::host::RenderFonts>,
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
    /// Draw its nearest answer, or hold the picture when it has none
    /// (playing and scrubbing: a late layer is never left out).
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
    /// The revision before an edit, for stand-ins only.
    previous: Option<Previous>,
    /// Override layers in the last look-ahead frame, and the last frame it asked.
    layers: usize,
    horizon: Option<i64>,
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
struct Previous {
    map: HashMap<NeedKey, CompatAnswer>,
    by_layer: HashMap<(String, String), BTreeMap<i64, NeedKey>>,
}
/// The nearest answered frame of a layer in one revision's answers.
fn nearest<'a>(
    map: &'a HashMap<NeedKey, CompatAnswer>,
    by_layer: &HashMap<(String, String), BTreeMap<i64, NeedKey>>,
    composition: &str,
    layer: &str,
    ticks: i64,
) -> Option<&'a CompatAnswer> {
    let times = by_layer.get(&(composition.to_owned(), layer.to_owned()))?;
    let before = times.range(..=ticks).next_back();
    let after = times.range(ticks..).next();
    let key = match (before, after) {
        (Some(b), Some(a)) if ticks - b.0 <= a.0 - ticks => b.1,
        (Some(_), Some(a)) => a.1,
        (Some((_, k)), None) | (None, Some((_, k))) => k,
        (None, None) => return None,
    };
    map.get(key)
}
impl Answers {
    /// The nearest answered frame of a late layer, this revision's first.
    fn nearest(&self, composition: &str, layer: &str, ticks: i64) -> Option<&CompatAnswer> {
        nearest(&self.map, &self.by_layer, composition, layer, ticks).or_else(|| {
            let p = self.previous.as_ref()?;
            nearest(&p.map, &p.by_layer, composition, layer, ticks)
        })
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
    /// Redraws the viewers when answers land.
    redraw: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
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
            let old = std::mem::take(&mut *answers);
            // An edit keeps the last revision's answers as stand-ins for
            // playing and scrubbed frames until its own land; new wrappers
            // don't.
            let previous = (old.epoch == epoch).then_some(Previous {
                map: old.map,
                by_layer: old.by_layer,
            });
            *answers = Answers {
                revision,
                epoch,
                frame: old.frame,
                late: old.late,
                stood_in: old.stood_in,
                per_need: old.per_need,
                layers: old.layers,
                previous,
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
    /// Playback's look-ahead at the playhead `now` (ticks, playing at
    /// `speed`): whether to ask now, how far ahead the answers will land (the
    /// realm's measured time for a batch) and how many frames the batch
    /// covers, about [`BATCH_TARGET`] of realm time. `None` while a batch is
    /// in flight or the last one still covers the frames ahead.
    pub(crate) fn ahead(&self, now: i64, speed: f64) -> Option<(Duration, usize)> {
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
        if let Some(horizon) = answers.horizon
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
        answers.horizon = Some(at + (frames - 1) * step);
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
    /// One frame of the open composition, in ticks.
    pub(crate) fn frame_ticks(&self) -> i64 {
        self.answers.lock().unwrap().frame
    }
    /// The frame about to be drawn: playing and scrubbed frames draw late
    /// layers from their nearest answer (holding the picture when there is
    /// none), paused frames wait.
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
        // again after its backoff): never drawn without the layer. A paused
        // frame waits (the viewer says what for after a few seconds); a
        // playing or scrubbed one draws the layer's nearest answer, or holds
        // the previous picture until one arrives.
        if answers.late == Late::Wait {
            return CompatAnswer::Missing;
        }
        let Some(stand_in) = answers.nearest(composition, layer, ticks).cloned() else {
            return CompatAnswer::Missing;
        };
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
    #[test]
    fn a_playing_frame_draws_a_late_layer_from_its_nearest_answer() {
        use crate::policy::Moment;
        let store = CompatStore::default();
        store.set_state(true, 7, 1);
        store.answer_all(7, 1, vec![(at("x", 1.), draw(10)), (at("x", 2.), draw(20))]);
        // Paused frames wait for their own answer.
        store.begin_frame(Moment::Paused);
        assert!(matches!(answer_at(&store, "x", 1.4), CompatAnswer::Missing));
        assert!(store.end_frame().is_empty());
        // Playing and scrubbed frames draw the nearest answered frame and say so.
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 1.4)), Some(10));
        assert_eq!(byte_of(answer_at(&store, "x", 1.6)), Some(20));
        assert_eq!(byte_of(answer_at(&store, "x", 2.0)), Some(20));
        let late = store.end_frame();
        assert_eq!(late.len(), 2, "the exact frame is not late");
        // A layer never answered is never left out: playing and scrubbing
        // hold the picture until it is answered.
        store.begin_frame(Moment::Playing);
        assert!(matches!(answer_at(&store, "y", 1.), CompatAnswer::Missing));
        assert!(store.end_frame().is_empty());
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
        assert!(matches!(answer_at(&store, "y", 1.), CompatAnswer::Missing));
        assert!(store.end_frame().is_empty());
        // A failed override still fails its layer, playing or not.
        store.answer_all(7, 1, vec![(at("x", 3.), Reply::Failed("boom".into()))]);
        store.begin_frame(Moment::Playing);
        assert!(matches!(
            answer_at(&store, "x", 3.),
            CompatAnswer::Failed(_)
        ));
        store.end_frame();
        // An edit keeps the last revision's answers as stand-ins only.
        store.set_state(true, 8, 1);
        store.begin_frame(Moment::Paused);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Missing));
        store.end_frame();
        store.begin_frame(Moment::Playing);
        assert_eq!(byte_of(answer_at(&store, "x", 1.)), Some(10));
        store.end_frame();
        // New wrappers don't.
        store.set_state(true, 8, 2);
        store.begin_frame(Moment::Scrub);
        assert!(matches!(answer_at(&store, "x", 1.), CompatAnswer::Missing));
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
        assert_eq!(store.ahead(0, 1.), Some((Duration::ZERO, 1)));
        let frame = 29_400_000; // 24 fps
        store.request_ahead(vec![at("x", 1.), at("y", 1.)], ticks(1.), 1, frame);
        assert_eq!(*asked.lock().unwrap(), vec![(1, 2)]);
        // In flight: nothing more.
        assert_eq!(store.ahead(0, 1.), None);
        std::thread::sleep(Duration::from_millis(20));
        store.answer_all(7, 1, vec![(at("x", 1.), draw(1)), (at("y", 1.), draw(1))]);
        // About 10 ms a need, two layers: 20 ms a frame, five frames a batch.
        let (lead, frames) = store.ahead(ticks(1.), 1.).unwrap();
        assert!((3..=5).contains(&frames), "{frames}");
        assert!(lead >= Duration::from_millis(60), "{lead:?}");
        store.request_ahead(vec![at("x", 2.), at("y", 2.)], ticks(2.), frames, frame);
        assert_eq!(asked.lock().unwrap()[1], (1, 2 * frames));
        store.answer_all(7, 1, vec![]);
        // The batch covers the frames ahead: nothing new until the playhead nears its end.
        assert_eq!(store.ahead(ticks(1.5), 1.), None);
        assert!(store.ahead(ticks(2.) + frames as i64 * frame, 1.).is_some());
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
