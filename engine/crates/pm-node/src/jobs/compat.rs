//! Compatibility rasters in exports (plan section 20; owner amendments 23
//! and 24): a `raster` override's layers in an export frame.
//!
//! The exporting window names its producers in the job's settings
//! (`DeliveryOptions::compat`). For every layer today's compositor would
//! rasterize through `PM.raster` (`pm_render::compat`), the job asks the
//! host (`Request::CompatFrame`, contract 14); main asks the editor windows,
//! and the one with the document open runs its override at that frame and
//! answers with the raster or a pass-through. The job waits for the answer
//! (an export has no deadline per frame), so the frame draws exactly what
//! the preview draws. A frame whose answer cannot be had fails the export
//! with the host's reason (no window has the document open, the override
//! went away); it never draws the layer without its override.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pm_protocol::common::{Revision, SessionId};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::snapshot::{CompatFrameRequest, CompatSource, PixelPayload};
use pm_render::compat::{CompatAnswer, CompatRaster, CompatRasters, ticks_of};

/// Answers a job's compatibility layers through the host.
pub(crate) trait CompatHost: Send + Sync {
    fn call(&self, request: Request, session: SessionId) -> Result<Response, String>;
    /// Several requests at once, answered in order (the host serves them
    /// back to back instead of one round trip each).
    fn call_many(&self, requests: Vec<Request>, session: SessionId) -> Vec<Result<Response, String>> {
        requests
            .into_iter()
            .map(|request| self.call(request, session))
            .collect()
    }
}

/// Frames past the asked one whose answers an export fetches with it: one
/// host round trip serves this many frames of a layer.
const FETCH_AHEAD: i64 = 4;
/// Most requests in one batch.
const MAX_BATCH: usize = 512;
/// Most requests sent to the host at once: main draws at most 1024 of a
/// window's compatibility frames at a time (`compat-frames.ts` `MAX_ASKS`)
/// and refuses more; a batch goes in turns of this many. Main posts the
/// asks it has at once to the window together, which answers them in one
/// task: a large project's frame asks for hundreds of layers. A frame asks
/// only for what the background fetch did not get ahead of it.
const HOST_ASKS: usize = 32;
/// Most requests the background fetch has at the host at once: with a
/// frame's own turn ([`HOST_ASKS`]) never above main's bound of 1024. Each
/// turn is a round trip through main and the window, but the window answers
/// a turn in one task and returns its answers when that task ends: turns of
/// 512 put a frame's own asks behind a second of answers for later frames
/// (PMV3's export took 431 s against 168 s in turns of 96).
const AHEAD_ASKS: usize = 96;
/// Frames past the one drawn whose answers a pipelined export keeps asked
/// for, and how close the drawn frame may come to the last one asked for
/// before more are asked.
const AHEAD_FRAMES: i64 = 12;
const AHEAD_LOW: i64 = 6;
/// Most bytes of answers fetched ahead and kept: past it the background
/// fetch waits for the export to use them.
const AHEAD_BYTES: usize = 384 << 20;

type AnswerKey = (String, String, i64, CompatSource, u64);
type LayerKey = (String, String, CompatSource, u64);

/// One export job's producers and the answers it got.
///
/// A pipelined job ([`JobCompat::pipelined`]) keeps the next frames'
/// answers asked for in the background: a helper thread asks the host for
/// the layers of the last frame that lacked answers at the next
/// [`AHEAD_FRAMES`] frames, in turns of [`AHEAD_ASKS`], while the export
/// renders. A frame then finds its answers (the window drew them while the
/// engine drew the frames before), or waits only for those still on their
/// way; what was never asked (a layer that just appeared) is asked for in
/// the frame, as before.
pub(crate) struct JobCompat {
    host: Arc<dyn CompatHost>,
    session: SessionId,
    document: String,
    revision: Revision,
    sources: Vec<CompatSource>,
    answers: Mutex<HashMap<AnswerKey, CompatAnswer>>,
    /// Why an answer could not be had (the export fails with it).
    failure: Mutex<Option<String>>,
    /// The job itself, for the background fetch (unset: no pipelining).
    me: std::sync::Weak<JobCompat>,
    ahead: Mutex<Ahead>,
    /// Answers being fetched, and a signal when some arrive.
    inflight: Mutex<std::collections::HashSet<AnswerKey>>,
    arrived: std::sync::Condvar,
    /// Bytes of raster answers held.
    bytes: std::sync::atomic::AtomicUsize,
}

/// What the background fetch asks for next.
#[derive(Default)]
struct Ahead {
    /// The first observed local time of each composition in this output
    /// frame. Later shutter samples and retry plans use the same boundary.
    frame_times: HashMap<String, i64>,
    /// The layers seen in missing frames and the last time scheduled for
    /// each. A new layer starts at its own frame, without moving older
    /// layers' cursors backwards and filling the worker queue again.
    layers: HashMap<LayerKey, i64>,
    /// Each composition's observed answer times. A remapped precomp cannot
    /// change another composition's frame step.
    cadences: HashMap<String, Cadence>,
    /// Answers before each composition's floor are done (an export moves
    /// forward).
    floors: HashMap<String, i64>,
    worker: Option<std::sync::mpsc::Sender<AheadBatch>>,
}

#[derive(Default)]
struct Cadence {
    last: Option<i64>,
    step: i64,
    candidate: i64,
    candidate_count: u8,
    learned: bool,
    epoch: u64,
}

impl Cadence {
    /// True when the speculative grid changed. One changed interval stops
    /// the old grid immediately; two equal intervals restore a stable one.
    fn observe(&mut self, time: i64) -> bool {
        let Some(last) = self.last.replace(time) else { return false };
        if time == last { return false }
        let Some(delta) = time.checked_sub(last) else { return self.disable() };
        if delta <= 0 || delta > pm_protocol::common::TICKS_PER_SECOND {
            return self.disable();
        }
        if delta == self.step {
            self.candidate = 0;
            self.candidate_count = 0;
            return false;
        }
        if !self.learned {
            self.learned = true;
            self.step = delta;
            self.epoch = self.epoch.wrapping_add(1);
            return true;
        }
        let changed = if self.step != 0 { self.disable() } else { false };
        if self.candidate == delta {
            self.candidate_count = self.candidate_count.saturating_add(1);
        } else {
            self.candidate = delta;
            self.candidate_count = 1;
        }
        if self.candidate_count < 2 { return changed }
        self.step = delta;
        self.candidate = 0;
        self.candidate_count = 0;
        self.epoch = self.epoch.wrapping_add(1);
        true
    }

    fn disable(&mut self) -> bool {
        self.candidate = 0;
        self.candidate_count = 0;
        if self.step == 0 { return false }
        self.step = 0;
        self.epoch = self.epoch.wrapping_add(1);
        true
    }
}

struct AheadBatch {
    composition: String,
    epoch: u64,
    keys: Vec<AnswerKey>,
}

fn raster_bytes(answer: &CompatAnswer) -> usize {
    match answer {
        CompatAnswer::Raster(r) => r.data.len(),
        _ => 0,
    }
}

impl JobCompat {
    pub(crate) fn new(
        host: Arc<dyn CompatHost>,
        session: SessionId,
        document: String,
        revision: Revision,
        sources: Vec<CompatSource>,
    ) -> JobCompat {
        JobCompat {
            host,
            session,
            document,
            revision,
            sources,
            answers: Mutex::new(HashMap::new()),
            failure: Mutex::new(None),
            me: std::sync::Weak::new(),
            ahead: Mutex::new(Ahead::default()),
            inflight: Mutex::new(std::collections::HashSet::new()),
            arrived: std::sync::Condvar::new(),
            bytes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The job shared, with its answers fetched ahead in the background.
    pub(crate) fn pipelined(self) -> Arc<JobCompat> {
        Arc::new_cyclic(|me| JobCompat {
            me: me.clone(),
            ..self
        })
    }

    fn insert_answer(&self, answers: &mut HashMap<AnswerKey, CompatAnswer>, key: AnswerKey, answer: CompatAnswer) {
        let added = raster_bytes(&answer);
        if let Some(old) = answers.insert(key, answer) {
            self.bytes
                .fetch_sub(raster_bytes(&old), std::sync::atomic::Ordering::Relaxed);
        }
        self.bytes
            .fetch_add(added, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drops answers of composition `comp` before `floor` (ticks).
    fn prune(&self, comp: &str, floor: i64) {
        let mut answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
        let mut freed = 0;
        answers.retain(|k, v| {
            let keep = k.0 != comp || k.2 >= floor;
            if !keep {
                freed += raster_bytes(v);
            }
            keep
        });
        self.bytes
            .fetch_sub(freed, std::sync::atomic::Ordering::Relaxed);
    }

    /// The first answer of an output frame, including a prefetched hit,
    /// observes the composition's local clock. Later shutter samples and
    /// retry plans may use different local times without changing cadence
    /// or evicting answers that this frame still needs.
    fn observe(&self, comp: &str, time: i64) {
        let mut ahead = self.ahead.lock().unwrap_or_else(|e| e.into_inner());
        if ahead.frame_times.contains_key(comp) {
            return;
        }
        ahead.frame_times.insert(comp.to_owned(), time);
        if ahead.cadences.entry(comp.to_owned()).or_default().observe(time) {
            // A changed clock makes every old layer cursor's future grid
            // invalid. Rebuild it from this frame once the clock is stable.
            for ((composition, _, _, _), scheduled) in &mut ahead.layers {
                if composition == comp {
                    *scheduled = time;
                }
            }
        }
        let step = ahead.cadences.get(comp).map_or(0, |cadence| cadence.step);
        let floor = time.saturating_sub(step.max(0));
        let prune = ahead.floors.get(comp).is_none_or(|f| floor > *f);
        if prune {
            ahead.floors.insert(comp.to_owned(), floor);
        }
        drop(ahead);
        if prune {
            self.prune(comp, floor);
        }
    }

    /// Ask for future answers on the first local clock of this output frame.
    /// Fetch may discover new layers at later shutter samples, but may not
    /// move the frame's prune floor or speculative grid forward.
    fn advance(&self, comp: &str) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let mut ahead = self.ahead.lock().unwrap_or_else(|e| e.into_inner());
        let Some(&time) = ahead.frame_times.get(comp) else { return };
        let (step, epoch) = ahead.cadences.get(comp).map_or((0, 0), |cadence| (cadence.step, cadence.epoch));
        if step <= 0 || ahead.layers.is_empty() {
            return;
        }
        let mut keys = Vec::new();
        for ((c, l, source, scale), scheduled) in &mut ahead.layers {
            if c != comp {
                continue;
            }
            if *scheduled < time + AHEAD_LOW * step {
                let mut t = (*scheduled + step).max(time + step);
                let to = time + AHEAD_FRAMES * step;
                while t <= to {
                    keys.push((c.clone(), l.clone(), t, *source, *scale));
                    t += step;
                }
                *scheduled = to;
            }
        }
        // A near frame needs all its layers before a distant frame needs
        // any. HashMap iteration otherwise sends twelve times of one layer
        // before the next layer's first answer.
        keys.sort_unstable_by_key(|key| key.2);
        if !keys.is_empty() {
            if ahead.worker.is_none() {
                ahead.worker = spawn_ahead(&me);
            }
            if let Some(worker) = &ahead.worker
                && worker.send(AheadBatch { composition: comp.to_owned(), epoch, keys }).is_err()
            {
                ahead.worker = None;
            }
        }
    }

    /// Asks the host for `keys` in turns of `turn`, marking them in flight
    /// (keys answered or in flight already are skipped); `needed` keys
    /// that cannot be had fail the export. False when one failed.
    fn ask(&self, keys: Vec<(AnswerKey, bool)>, turn: usize) -> bool {
        let keys: Vec<(AnswerKey, bool)> = {
            let answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
            let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            keys.into_iter()
                .filter(|(k, _)| !answers.contains_key(k) && inflight.insert(k.clone()))
                .collect()
        };
        let mut ok = true;
        for chunk in keys.chunks(turn) {
            let requests: Vec<Request> = chunk
                .iter()
                .map(|((composition, layer, time, source, scale), _)| {
                    Request::CompatFrame(CompatFrameRequest {
                        document: self.document.as_str().into(),
                        composition: composition.as_str().into(),
                        layer: layer.as_str().into(),
                        time: pm_protocol::common::Ticks::new(*time)
                            .unwrap_or(pm_protocol::common::Ticks::ZERO),
                        source: *source,
                        scale: f64::from_bits(*scale),
                        revision: self.revision,
                        // Layer producers only: SVG rasters have their own
                        // source (`pm_render::vector::svg_rasters`).
                        svg: None,
            blender: None,
                        frame: None,
                    })
                })
                .collect();
            let results = if ok && self.failure().is_none() {
                self.host.call_many(requests, self.session)
            } else {
                Vec::new()
            };
            {
                let mut answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
                for ((key, needed), result) in chunk.iter().zip(results) {
                    match answer_of(result) {
                        Ok(answer) => self.insert_answer(&mut answers, key.clone(), answer),
                        // A frame ahead that cannot be had is asked for
                        // again when its frame comes (and fails then, with
                        // its own reason).
                        Err(message) if *needed => {
                            self.fail_message(message);
                            ok = false;
                        }
                        Err(_) => {}
                    }
                }
            }
            let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            for (key, _) in chunk {
                inflight.remove(key);
            }
            drop(inflight);
            self.arrived.notify_all();
        }
        ok
    }

    /// Waits until none of `keys` is being fetched.
    fn wait_for(&self, keys: &[AnswerKey]) {
        let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        while keys.iter().any(|k| inflight.contains(k)) && self.failure().is_none() {
            inflight = self
                .arrived
                .wait_timeout(inflight, std::time::Duration::from_millis(50))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

/// The background fetch of a pipelined job: asks for what it is sent,
/// within [`AHEAD_BYTES`], until the job is gone.
fn spawn_ahead(job: &Arc<JobCompat>) -> Option<std::sync::mpsc::Sender<AheadBatch>> {
    let (tx, rx) = std::sync::mpsc::channel::<AheadBatch>();
    let weak = Arc::downgrade(job);
    std::thread::Builder::new()
        .name("pm-export-compat-ahead".into())
        .spawn(move || {
            while let Ok(batch) = rx.recv() {
                for chunk in batch.keys.chunks(AHEAD_ASKS) {
                    // Within the byte bound: the export uses (and drops)
                    // answers as it moves on.
                    loop {
                        let Some(job) = weak.upgrade() else { return };
                        if job.failure().is_some() {
                            return;
                        }
                        let current = job.ahead.lock().unwrap_or_else(|e| e.into_inner())
                            .cadences.get(&batch.composition)
                            .is_some_and(|cadence| cadence.step > 0 && cadence.epoch == batch.epoch);
                        if !current {
                            break;
                        }
                        if job.bytes.load(std::sync::atomic::Ordering::Relaxed) < AHEAD_BYTES {
                            break;
                        }
                        drop(job);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    let Some(job) = weak.upgrade() else { return };
                    let current = job.ahead.lock().unwrap_or_else(|e| e.into_inner())
                        .cadences.get(&batch.composition)
                        .is_some_and(|cadence| cadence.step > 0 && cadence.epoch == batch.epoch);
                    if !current {
                        break;
                    }
                    job.ask(chunk.iter().map(|k| (k.clone(), false)).collect(), AHEAD_ASKS);
                }
            }
        })
        .ok()?;
    Some(tx)
}

impl JobCompat {
    /// The first reason an answer could not be had.
    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Records `message` as the failure (the first one wins) and returns it.
    fn fail_message(&self, message: String) -> String {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(message.clone());
        }
        message
    }
}

/// RGBA8 of a premultiplied working pixel as today's compositor canvas
/// held it (`GL.render`'s present pass copies the accumulator into an
/// `alpha: false` canvas: the premultiplied display-encoded color over
/// black, opaque).
fn canvas_rgba8(rgba: &[f32], working: pm_export::WorkingEncoding) -> Vec<u8> {
    let mut out = vec![0u8; rgba.len()];
    for (px, o) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let (c, _) = pm_export::convert::output_rgba(
            [px[0], px[1], px[2], px[3]],
            working,
            pm_export::AlphaOutput::Premultiplied,
        );
        for k in 0..3 {
            let v = if c[k].is_nan() {
                0.0
            } else {
                c[k].clamp(0.0, 1.0)
            };
            o[k] = (v * 255.0).round() as u8;
        }
        o[3] = 255;
    }
    out
}

/// Working values of an opaque canvas frame (the inverse of [`canvas_rgba8`]).
fn working_of(rgba8: &[u8], straight: bool, working: pm_export::WorkingEncoding) -> Vec<f32> {
    let mut out = vec![0f32; rgba8.len()];
    for (px, o) in rgba8.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        // An `alpha: false` canvas reads back opaque; a straight answer is
        // composited over black as that canvas would show it.
        let a = if straight {
            f64::from(px[3]) / 255.0
        } else {
            1.0
        };
        for k in 0..3 {
            let e = f64::from(px[k]) / 255.0 * a;
            o[k] = match working {
                pm_export::WorkingEncoding::Standard => e,
                pm_export::WorkingEncoding::Linear => pm_export::convert::e_decode(e),
            } as f32;
        }
        o[3] = 1.0;
    }
    out
}

impl JobCompat {
    /// The composition whose export frames a window's compatibility
    /// context draws over (`CompatSource::CompatContext` among the
    /// producers): each frame goes to the window before it is encoded.
    pub(crate) fn context_frames(
        self: &Arc<Self>,
        composition: String,
    ) -> Option<Arc<dyn pm_export::render::FramePost>> {
        if !self.sources.contains(&CompatSource::CompatContext) {
            return None;
        }
        Some(Arc::new(ContextFrames {
            compat: self.clone(),
            composition,
        }))
    }
}

/// Export frames through the window's compatibility context.
struct ContextFrames {
    compat: Arc<JobCompat>,
    composition: String,
}

impl pm_export::render::FramePost for ContextFrames {
    fn frame(
        &self,
        _index: u64,
        time: pm_protocol::common::Ticks,
        width: u32,
        height: u32,
        working: pm_export::WorkingEncoding,
        rgba: Vec<f32>,
    ) -> Result<Vec<f32>, pm_export::render::JobError> {
        let fail = |message: String| {
            let message = self.compat.fail_message(message);
            pm_export::render::JobError::Render(format!(
                "An extension in the editor draws over this export, and its frame could not be drawn: {message}"
            ))
        };
        let bytes = canvas_rgba8(&rgba, working);
        let payload = if bytes.len() <= pm_protocol::snapshot::INLINE_PIXEL_BYTES {
            PixelPayload::Inline {
                bytes: pm_protocol::common::Bytes(bytes),
            }
        } else {
            let length = bytes.len() as u64;
            let name = pm_bridge::shm::publish(&bytes)
                .map_err(|e| fail(format!("the frame could not be shared: {e}")))?;
            PixelPayload::SharedMemory {
                name,
                length: pm_protocol::common::ByteSize::new(length)
                    .map_err(|_| fail("the frame is too large to share".into()))?,
            }
        };
        let request = CompatFrameRequest {
            document: self.compat.document.as_str().into(),
            composition: self.composition.as_str().into(),
            layer: self.composition.as_str().into(),
            time,
            source: CompatSource::CompatContext,
            scale: 1.0,
            revision: self.compat.revision,
            svg: None,
            blender: None,
            frame: Some(pm_protocol::snapshot::CompatFrameInput {
                width,
                height,
                payload,
            }),
        };
        match self
            .compat
            .host
            .call(Request::CompatFrame(request), self.compat.session)
        {
            // No handler drew: the frame as the engine rendered it.
            Ok(Response::CompatFrame(frame)) if frame.native => Ok(rgba),
            Ok(Response::CompatFrame(frame)) => {
                if frame.width != width || frame.height != height {
                    return Err(fail(format!(
                        "the window answered a {}×{} frame for a {width}×{height} export",
                        frame.width, frame.height
                    )));
                }
                let straight = match frame.format {
                    pm_protocol::snapshot::CompatFormat::Rgba8Premultiplied => false,
                    pm_protocol::snapshot::CompatFormat::Rgba8Straight => true,
                    other => return Err(fail(format!("the window answered {other:?} pixels"))),
                };
                let data = match frame.payload {
                    PixelPayload::Inline { bytes } => bytes.0,
                    PixelPayload::SharedMemory { name, length } => {
                        pm_bridge::shm::take(&name, length.get() as usize)
                            .map_err(|e| fail(format!("the frame could not be read: {e}")))?
                    }
                };
                if data.len() != rgba.len() {
                    return Err(fail("the window's frame does not match its size".into()));
                }
                Ok(working_of(&data, straight, working))
            }
            Ok(_) => Err(fail(
                "the host answered a compatibility frame with something else".into(),
            )),
            Err(message) => Err(fail(message)),
        }
    }
}

impl CompatRasters for JobCompat {
    fn begin_frame(&self) {
        self.ahead.lock().unwrap_or_else(|e| e.into_inner()).frame_times.clear();
    }

    fn producing(&self, source: CompatSource) -> bool {
        self.sources.contains(&source)
    }

    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<pm_protocol::common::PixelRect>,
    ) -> CompatAnswer {
        // Export frames never crop (today's `opt.exporting` turns preview
        // source clipping off; `pm_render::compat_crop`).
        debug_assert!(crop.is_none(), "an export frame asked for a crop");
        let ticks = ticks_of(time);
        let key = (
            composition.to_owned(),
            layer.to_owned(),
            ticks.get(),
            source,
            scale.to_bits(),
        );
        self.observe(composition, ticks.get());
        self.advance(composition);
        if let Some(answer) = self
            .answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return answer.clone();
        }
        // Not answered yet: the frame lists it, and the renderer fetches
        // the frame's answers in one batch ([`Self::fetch`]).
        CompatAnswer::Missing
    }

    fn fetch(&self, needs: &[pm_protocol::snapshot::CompatNeed]) -> bool {
        if self.failure().is_some() || needs.is_empty() {
            return false;
        }
        let key_of = |n: &pm_protocol::snapshot::CompatNeed, time: i64| -> AnswerKey {
            (
                n.composition.as_str().to_owned(),
                n.layer.as_str().to_owned(),
                time,
                n.source,
                n.scale.to_bits(),
            )
        };
        let pipelined = self.me.upgrade().is_some();
        // A fetch may include nested compositions at remapped local times.
        // Its needs identify new layers, but only rendered answers observe
        // the cadence: fetched gaps can skip frames already answered ahead.
        let mut earliest = HashMap::<String, i64>::new();
        for n in needs {
            earliest
                .entry(n.composition.as_str().to_owned())
                .and_modify(|time| *time = (*time).min(n.time.get()))
                .or_insert(n.time.get());
        }
        let steps = {
            let mut ahead = self.ahead.lock().unwrap_or_else(|e| e.into_inner());
            if pipelined {
                // A newly seen layer starts at its own local time; other
                // compositions' horizons stay where they were.
                for n in needs {
                    if ahead.cadences.get(n.composition.as_str()).is_some_and(|cadence| cadence.step > 0) {
                        let layer = (
                            n.composition.as_str().to_owned(),
                            n.layer.as_str().to_owned(),
                            n.source,
                            n.scale.to_bits(),
                        );
                        ahead.layers.entry(layer).or_insert(n.time.get());
                    }
                }
            }
            earliest.keys().map(|comp| (comp.clone(), ahead.cadences.get(comp).map_or(0, |cadence| cadence.step))).collect::<HashMap<_, _>>()
        };
        // The frame's needs first, then (unpipelined) the same layers at
        // the next frames, in time order (the order today's export ran the
        // chain in).
        let mut asks: Vec<(AnswerKey, bool)> = Vec::new();
        {
            let answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
            let mut seen = std::collections::HashSet::new();
            let count = if pipelined { 0 } else { FETCH_AHEAD };
            for k in 0..=count {
                for n in needs {
                    let step = steps[n.composition.as_str()];
                    if k > 0 && (pipelined || step <= 0) {
                        continue;
                    }
                    let key = key_of(n, n.time.get() + k * step);
                    if answers.contains_key(&key) || !seen.insert(key.clone()) {
                        continue;
                    }
                    if k > 0 && asks.len() >= MAX_BATCH {
                        break;
                    }
                    asks.push((key, k == 0));
                }
            }
        }
        // Those the background fetch is asking for: waited for, then asked
        // here if it could not get them.
        let needed: Vec<AnswerKey> = asks.iter().filter(|a| a.1).map(|a| a.0.clone()).collect();
        if !self.ask(asks, HOST_ASKS) {
            return false;
        }
        // Those the background fetch was asking for are waited for; any it
        // could not get are asked for here (and fail the export if the
        // host cannot answer them).
        loop {
            self.wait_for(&needed);
            if self.failure().is_some() {
                return false;
            }
            let missing: Vec<(AnswerKey, bool)> = {
                let answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
                needed
                    .iter()
                    .filter(|k| !answers.contains_key(*k))
                    .map(|k| (k.clone(), true))
                    .collect()
            };
            if missing.is_empty() {
                break;
            }
            if !self.ask(missing, HOST_ASKS) {
                return false;
            }
        }
        for n in needs {
            self.advance(n.composition.as_str());
        }
        self.failure().is_none()
    }
}

/// A host answer to a layer's compatibility request.
fn answer_of(result: Result<Response, String>) -> Result<CompatAnswer, String> {
    match result {
        Ok(Response::CompatFrame(frame)) if frame.native => Ok(CompatAnswer::Native),
        Ok(Response::CompatFrame(frame)) => {
            let data = match frame.payload {
                PixelPayload::Inline { bytes } => bytes.0,
                PixelPayload::SharedMemory { name, length } => {
                    pm_bridge::shm::take(&name, length.get() as usize)
                        .map_err(|e| format!("the layer's pixels could not be read: {e}"))?
                }
            };
            Ok(CompatAnswer::Raster(Arc::new(CompatRaster {
                width: frame.width,
                height: frame.height,
                format: frame.format,
                anchor: frame.origin,
                size: frame.size.unwrap_or([
                    f64::from(frame.width) / frame.scale,
                    f64::from(frame.height) / frame.scale,
                ]),
                uv: frame.uv.unwrap_or([0.0, 0.0, 1.0, 1.0]),
                data: Arc::new(data),
            })))
        }
        Ok(_) => Err("the host answered a compatibility frame with something else".into()),
        Err(message) => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};
    use super::*;
    use pm_protocol::common::Bytes;
    use pm_protocol::snapshot::{CompatFormat, CompatFrame};

    struct Host {
        calls: Mutex<Vec<CompatFrameRequest>>,
        answer: Box<dyn Fn(&CompatFrameRequest) -> Result<Response, String> + Send + Sync>,
    }

    impl CompatHost for Host {
        fn call(&self, request: Request, _session: SessionId) -> Result<Response, String> {
            let Request::CompatFrame(r) = request else {
                panic!("unexpected request");
            };
            let answer = (self.answer)(&r);
            self.calls.lock().unwrap().push(r);
            answer
        }
    }

    fn job(
        answer: impl Fn(&CompatFrameRequest) -> Result<Response, String> + Send + Sync + 'static,
    ) -> (Arc<Host>, JobCompat) {
        let host = Arc::new(Host {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        });
        let compat = JobCompat::new(
            host.clone(),
            SessionId::new(4).unwrap(),
            "P".into(),
            Revision::new(9).unwrap(),
            vec![CompatSource::RasterOverride],
        );
        (host, compat)
    }

    fn frame(native: bool) -> Response {
        Response::CompatFrame(Box::new(CompatFrame {
            key: "k".into(),
            width: if native { 1 } else { 2 },
            height: 1,
            format: CompatFormat::Rgba8Premultiplied,
            origin: [3.0, 4.0],
            scale: 2.0,
            payload: PixelPayload::Inline {
                bytes: Bytes(if native { Vec::new() } else { vec![9; 8] }),
            },
            native,
            size: Some([5.0, 6.0]),
            uv: None,
        }))
    }

    fn need(layer: &str, seconds: f64, scale: f64) -> pm_protocol::snapshot::CompatNeed {
        pm_protocol::snapshot::CompatNeed {
            composition: "root".into(),
            layer: layer.into(),
            time: ticks_of(seconds),
            source: CompatSource::RasterOverride,
            scale,
            crop: None,
        }
    }

    fn draw_and_fetch(compat: &JobCompat, needs: &[pm_protocol::snapshot::CompatNeed]) {
        compat.begin_frame();
        for n in needs {
            compat.answer(
                n.composition.as_str(), n.layer.as_str(),
                n.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
                n.scale, n.source, None,
            );
        }
        assert!(compat.fetch(needs));
    }

    #[test]
    fn asks_the_host_once_per_layer_frame_and_density_at_the_job_s_revision() {
        let (host, compat) = job(|r| Ok(frame(r.layer.as_str() == "plain")));
        assert!(compat.producing(CompatSource::RasterOverride));
        assert!(!compat.producing(CompatSource::CompatContext));
        // A frame lists what it lacks; the renderer fetches it in one batch.
        assert!(matches!(
            compat.answer("root", "typed", 0.5, 2.0, CompatSource::RasterOverride, None),
            CompatAnswer::Missing
        ));
        assert!(host.calls.lock().unwrap().is_empty());
        assert!(compat.fetch(&[need("typed", 0.5, 2.0), need("plain", 0.5, 2.0)]));
        let CompatAnswer::Raster(raster) = compat.answer(
            "root",
            "typed",
            0.5,
            2.0,
            CompatSource::RasterOverride,
            None,
        ) else {
            panic!("expected a raster");
        };
        assert_eq!(
            (
                raster.width,
                raster.height,
                raster.anchor,
                raster.size,
                &raster.data[..]
            ),
            (2, 1, [3.0, 4.0], [5.0, 6.0], &[9u8; 8][..])
        );
        assert!(matches!(
            compat.answer(
                "root",
                "typed",
                0.5,
                2.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Raster(_)
        ));
        assert!(matches!(
            compat.answer(
                "root",
                "plain",
                0.5,
                2.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Native
        ));
        let calls = host.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (
                calls[0].document.as_str(),
                calls[0].time.get(),
                calls[0].revision.get(),
                calls[0].scale
            ),
            ("P", 352_800_000, 9, 2.0)
        );
    }

    #[test]
    fn a_frame_the_host_cannot_answer_fails_with_its_reason() {
        let (_, compat) = job(|_| Err("open the project in a window to export it".into()));
        assert!(!compat.fetch(&[need("typed", 0.0, 1.0)]));
        assert!(matches!(
            compat.answer(
                "root",
                "typed",
                0.0,
                1.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Missing
        ));
        assert_eq!(
            compat.failure().as_deref(),
            Some("open the project in a window to export it")
        );
    }

    #[test]
    fn later_frames_of_the_same_layers_come_with_one_batch_in_time_order() {
        let (host, compat) = job(|r| Ok(frame(r.layer.as_str() == "plain")));
        let step = 1.0 / 30.0;
        // The first frame: its own needs only (no step known yet).
        draw_and_fetch(&compat, &[need("typed", 0.0, 2.0), need("plain", 0.0, 2.0)]);
        assert_eq!(host.calls.lock().unwrap().len(), 2);
        // The next frame gives the step: its needs and the same layers at
        // the next frames come in one batch, frame by frame.
        draw_and_fetch(&compat, &[need("typed", step, 2.0), need("plain", step, 2.0)]);
        let times: Vec<(String, i64)> = host.calls.lock().unwrap()[2..]
            .iter()
            .map(|c| (c.layer.as_str().to_owned(), c.time.get()))
            .collect();
        let at = |k: f64| ticks_of(step).get() + (k * ticks_of(step).get() as f64) as i64;
        assert_eq!(times.len(), 2 * (1 + FETCH_AHEAD as usize));
        assert_eq!(&times[..4], &[("typed".into(), at(0.0)), ("plain".into(), at(0.0)), ("typed".into(), at(1.0)), ("plain".into(), at(1.0))]);
        // Those frames are answered without asking again.
        let asked = host.calls.lock().unwrap().len();
        for k in 1..=FETCH_AHEAD {
            compat.begin_frame();
            let t = step * (1 + k) as f64;
            assert!(!matches!(
                compat.answer("root", "typed", t, 2.0, CompatSource::RasterOverride, None),
                CompatAnswer::Missing
            ), "frame {k} ahead");
        }
        assert_eq!(host.calls.lock().unwrap().len(), asked);
    }

    #[test]
    fn a_large_frame_is_fetched_in_turns_the_host_accepts() {
        struct Turns {
            sizes: Mutex<Vec<usize>>,
        }
        impl CompatHost for Turns {
            fn call(&self, _request: Request, _session: SessionId) -> Result<Response, String> {
                unreachable!("batched")
            }
            fn call_many(&self, requests: Vec<Request>, _session: SessionId) -> Vec<Result<Response, String>> {
                self.sizes.lock().unwrap().push(requests.len());
                assert!(requests.len() <= HOST_ASKS, "main refuses more than its in-flight bound");
                requests.iter().map(|_| Ok(frame(true))).collect()
            }
        }
        let host = Arc::new(Turns { sizes: Mutex::new(Vec::new()) });
        let compat = JobCompat::new(host.clone(), SessionId::new(4).unwrap(), "P".into(), Revision::new(9).unwrap(), vec![CompatSource::RasterOverride]);
        let needs: Vec<_> = (0..60).map(|i| need(&format!("L{i}"), 0.0, 1.0)).collect();
        assert!(compat.fetch(&needs));
        assert_eq!(host.sizes.lock().unwrap().iter().sum::<usize>(), 60);
        assert!(needs.iter().all(|n| !matches!(compat.answer("root", n.layer.as_str(), 0.0, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Missing)));
    }

    #[test]
    fn a_pipelined_export_finds_its_next_frames_answered_in_the_background() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Slow {
            asked: Mutex<Vec<i64>>,
            outstanding: AtomicUsize,
            most: AtomicUsize,
        }
        impl CompatHost for Slow {
            fn call(&self, _request: Request, _session: SessionId) -> Result<Response, String> {
                unreachable!("batched")
            }
            fn call_many(&self, requests: Vec<Request>, _session: SessionId) -> Vec<Result<Response, String>> {
                let now = self.outstanding.fetch_add(requests.len(), Ordering::SeqCst) + requests.len();
                self.most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                let times: Vec<i64> = requests.iter().map(|request| {
                    let Request::CompatFrame(request) = request else { panic!() };
                    request.time.get()
                }).collect();
                assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "near frames are asked first");
                for r in &requests {
                    let Request::CompatFrame(r) = r else { panic!() };
                    self.asked.lock().unwrap().push(r.time.get());
                }
                self.outstanding.fetch_sub(requests.len(), Ordering::SeqCst);
                requests.iter().map(|_| Ok(frame(false))).collect()
            }
        }
        let host = Arc::new(Slow { asked: Mutex::new(Vec::new()), outstanding: AtomicUsize::new(0), most: AtomicUsize::new(0) });
        let compat = JobCompat::new(host.clone(), SessionId::new(4).unwrap(), "P".into(), Revision::new(9).unwrap(), vec![CompatSource::RasterOverride]).pipelined();
        let step = 1.0 / 30.0;
        let layers: Vec<String> = (0..20).map(|i| format!("L{i}")).collect();
        let needs = |t: f64| layers.iter().map(|l| need(l, t, 1.0)).collect::<Vec<_>>();
        // The first two frames give the layers and the step.
        draw_and_fetch(&compat, &needs(0.0));
        draw_and_fetch(&compat, &needs(step));
        let mut fetched = 0;
        for k in 2..60 {
            compat.begin_frame();
            let t = step * k as f64;
            // A frame asks only for what is not answered (or on its way).
            let missing: Vec<_> = layers
                .iter()
                .filter(|l| matches!(compat.answer("root", l, t, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Missing))
                .map(|l| need(l, t, 1.0))
                .collect();
            if !missing.is_empty() {
                fetched += 1;
                assert!(compat.fetch(&missing));
            }
            for l in &layers {
                assert!(matches!(compat.answer("root", l, t, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Raster(_)), "frame {k}");
            }
            // The engine draws the frame meanwhile (longer than the host
            // takes for a frame's answers, as in an export).
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        // Most frames found their answers fetched ahead.
        assert!(fetched < 20, "{fetched} frames waited for a fetch of their own");
        // Never more at the host than main accepts, and never asked far ahead.
        assert!(host.most.load(Ordering::SeqCst) <= HOST_ASKS + AHEAD_ASKS);
        assert!(HOST_ASKS + AHEAD_ASKS <= 1024, "main's MAX_ASKS");
        let newest = ticks_of(step * 59.0).get();
        let furthest = *host.asked.lock().unwrap().iter().max().unwrap();
        assert!(furthest <= newest + (AHEAD_FRAMES + 1) * ticks_of(step).get());
        // Answers of frames already drawn are dropped.
        assert!(matches!(compat.answer("root", "L0", 0.0, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Missing));
    }

    #[test]
    fn a_new_layer_after_answered_frames_keeps_the_existing_horizon() {
        let (host, job) = job(|_| Ok(frame(true)));
        let compat = job.pipelined();
        let step = ticks_of(1.0 / 30.0).get();
        draw_and_fetch(&compat, &[need("old", 0.0, 1.0)]);
        draw_and_fetch(&compat, &[need("old", 1.0 / 30.0, 1.0)]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !host.calls.lock().unwrap().iter().any(|c| c.layer.as_str() == "old" && c.time.get() == 4 * step) {
            assert!(Instant::now() < deadline, "the original layer is answered ahead");
            std::thread::sleep(Duration::from_millis(1));
        }
        compat.wait_for(&[("root".into(), "old".into(), 4 * step, CompatSource::RasterOverride, 1.0f64.to_bits())]);
        // These rendered frames are cache hits, so there is no intervening
        // fetch batch from which to infer their cadence.
        for k in 2..=4 {
            compat.begin_frame();
            assert!(matches!(compat.answer("root", "old", k as f64 / 30.0, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Native));
        }
        draw_and_fetch(&compat, &[need("new", 5.0 / 30.0, 1.0)]);
        let target = (5 + AHEAD_FRAMES) * step;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !host.calls.lock().unwrap().iter().any(|c| c.layer.as_str() == "new" && c.time.get() == target) {
            assert!(Instant::now() < deadline, "new layer reaches its full frame-grid horizon");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(host.calls.lock().unwrap().iter().filter(|c| c.layer.as_str() == "new")
            .all(|c| (c.time.get() - 5 * step) % step == 0));
    }

    #[test]
    fn remapped_nested_time_does_not_change_root_export_cadence() {
        let (host, job) = job(|_| Ok(frame(true)));
        let compat = job.pipelined();
        let root_step = ticks_of(1.0 / 30.0).get();
        let nested_step = root_step * 9 / 10;
        // The nested clock starts just after root frame 1. Comparing
        // consecutive batches across compositions would observe 1,000
        // ticks and wrongly use that as both compositions' frame step.
        let nested_offset = root_step + 1_000;
        let at = |comp: &str, layer: &str, time: i64| {
            let mut n = need(layer, 0.0, 1.0);
            n.composition = comp.into();
            n.time = pm_protocol::common::Ticks::new(time).unwrap();
            n
        };
        draw_and_fetch(&compat, &[at("root", "root-old", 0), at("nested", "nested-old", nested_offset)]);
        draw_and_fetch(&compat, &[at("root", "root-old", root_step), at("nested", "nested-old", nested_offset + nested_step)]);
        draw_and_fetch(&compat, &[at("root", "root-mid", 2 * root_step), at("nested", "nested-old", nested_offset + 2 * nested_step)]);
        // Later frame plans observe both local clocks even when their old
        // layers have already been answered, with no fetch batch between.
        for k in 3..=4 {
            compat.begin_frame();
            compat.answer("root", "root-old", k as f64 / 30.0, 1.0, CompatSource::RasterOverride, None);
            compat.answer("nested", "nested-old", (nested_offset + k * nested_step) as f64 / pm_protocol::common::TICKS_PER_SECOND as f64, 1.0, CompatSource::RasterOverride, None);
        }
        draw_and_fetch(&compat, &[at("root", "root-new", 5 * root_step), at("nested", "nested-new", nested_offset + 5 * nested_step)]);
        let expected = [
            ("root-old", (1 + AHEAD_FRAMES) * root_step),
            ("nested-old", nested_offset + (1 + AHEAD_FRAMES) * nested_step),
            ("root-new", (5 + AHEAD_FRAMES) * root_step),
            ("nested-new", nested_offset + (5 + AHEAD_FRAMES) * nested_step),
        ];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let calls = host.calls.lock().unwrap();
            if expected.iter().all(|(layer, time)| calls.iter().any(|c| c.layer.as_str() == *layer && c.time.get() == *time)) {
                // Check the observable requests, not the scheduler's state:
                // every speculative answer is useful on a real local frame.
                for call in calls.iter() {
                    let (offset, step) = if call.composition.as_str() == "root" {
                        (0, root_step)
                    } else {
                        (nested_offset, nested_step)
                    };
                    assert_eq!((call.time.get() - offset) % step, 0);
                }
                break;
            }
            assert!(Instant::now() < deadline, "both local clocks reach their full answer horizons");
            drop(calls);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_speed_ramp_restores_the_normal_answer_horizon() {
        let (host, job) = job(|_| Ok(frame(true)));
        let compat = job.pipelined();
        let step = ticks_of(1.0 / 30.0).get();
        let times = [0, step, 2 * step, 2 * step + step / 2,
            2 * step + step / 2 + 3 * step / 4,
            2 * step + step / 2 + 3 * step / 4 + step,
            2 * step + step / 2 + 3 * step / 4 + 2 * step];
        let at = |layer: &str, time: i64| {
            let mut n = need(layer, 0.0, 1.0);
            n.time = pm_protocol::common::Ticks::new(time).unwrap();
            n
        };
        for time in times {
            draw_and_fetch(&compat, &[at("ramp", time)]);
        }
        let new_time = times[times.len() - 1] + step;
        draw_and_fetch(&compat, &[at("after-ramp", new_time)]);
        let target = new_time + AHEAD_FRAMES * step;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let calls = host.calls.lock().unwrap();
            if calls.iter().any(|c| c.layer.as_str() == "after-ramp" && c.time.get() == target) {
                assert!(calls.iter().filter(|c| c.layer.as_str() == "after-ramp")
                    .all(|c| (c.time.get() - new_time) % step == 0),
                    "recovered lookahead uses the normal local step");
                break;
            }
            assert!(Instant::now() < deadline, "normal cadence regains a full horizon");
            drop(calls);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn offset_instances_of_one_composition_do_not_keep_asking_false_future_frames() {
        use std::collections::HashSet;
        let (host, job) = job(|_| Ok(frame(true)));
        let compat = job.pipelined();
        let step = ticks_of(1.0 / 30.0).get();
        let offset = step * 3 / 5;
        let mut observed = HashSet::new();
        for frame_index in 0..40 {
            let needs: Vec<_> = [frame_index * step, frame_index * step + offset].into_iter().map(|time| {
                observed.insert(time);
                let mut n = need("shared", 0.0, 1.0);
                n.time = pm_protocol::common::Ticks::new(time).unwrap();
                n
            }).collect();
            draw_and_fetch(&compat, &needs);
        }
        // Both instances are seen within one output frame. Only the first
        // local clock sets the cadence; the second cannot make a 0.6/0.4
        // alternation or useless future asks on an off-frame grid.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let calls = host.calls.lock().unwrap();
            if calls.iter().any(|c| c.time.get() == 39 * step + offset) {
                let waste = calls.iter().filter(|c| !observed.contains(&c.time.get())).count();
                assert!(waste <= AHEAD_FRAMES as usize,
                    "offset instance clocks made {waste} unusable future asks");
                break;
            }
            assert!(Instant::now() < deadline, "the final observed frame is answered");
            drop(calls);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn shutter_samples_survive_the_compatibility_replan() {
        let step = ticks_of(1.0 / 30.0).get();
        for (shutter_denominator, pipelined) in [(16, false), (16, true), (32, false), (32, true)] {
            let (_, job) = job(|_| Ok(frame(true)));
            let compat = if pipelined { job.pipelined() } else { Arc::new(job) };
            for frame_index in 0..2 {
                let samples: Vec<_> = (0..16).map(|sample| {
                    let mut n = need("blurred", 0.0, 1.0);
                    n.time = pm_protocol::common::Ticks::new(
                        frame_index * step + sample * step / shutter_denominator,
                    ).unwrap();
                    n
                }).collect();
                // The first subsample was answered ahead. The remaining
                // samples become missing on the first plan, and the renderer
                // fetches those needs once before planning the entire frame
                // again. No cached first-pass answer may be evicted here.
                assert!(compat.fetch(&samples[..1]));
                compat.begin_frame();
                let missing: Vec<_> = samples.iter().filter(|n| {
                    matches!(compat.answer(
                        n.composition.as_str(), n.layer.as_str(),
                        n.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
                        n.scale, n.source, None,
                    ), CompatAnswer::Missing)
                }).cloned().collect();
                assert_eq!(missing.len(), 15, "only the first subsample was cached");
                assert!(compat.fetch(&missing));
                for n in &samples {
                    assert!(matches!(compat.answer(
                        n.composition.as_str(), n.layer.as_str(),
                        n.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
                        n.scale, n.source, None,
                    ), CompatAnswer::Native), "the retry has every {shutter_denominator}-shutter subsample at {}", n.time.get());
                }
            }
        }
    }

    #[test]
    fn a_compat_context_draws_over_export_frames_as_today_s_canvas() {
        use pm_export::WorkingEncoding;
        let host = Arc::new(Host {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(|r: &CompatFrameRequest| {
                let input = r.frame.as_ref().expect("the export frame goes along");
                let PixelPayload::Inline { bytes } = &input.payload else {
                    panic!("a small frame is inline");
                };
                // Pixel 0 is left as it was; pixel 1 is drawn over in red.
                if r.time.get() == 0 {
                    return Ok(frame(true));
                }
                let mut out = bytes.0.clone();
                out[4..8].copy_from_slice(&[255, 0, 0, 255]);
                Ok(Response::CompatFrame(Box::new(CompatFrame {
                    key: "context".into(),
                    width: input.width,
                    height: input.height,
                    format: CompatFormat::Rgba8Premultiplied,
                    origin: [0.0, 0.0],
                    scale: 1.0,
                    payload: PixelPayload::Inline { bytes: Bytes(out) },
                    native: false,
                    size: None,
                    uv: None,
                })))
            }),
        });
        let compat = Arc::new(JobCompat::new(
            host.clone(),
            SessionId::new(4).unwrap(),
            "P".into(),
            Revision::new(9).unwrap(),
            vec![CompatSource::CompatContext],
        ));
        let post = compat
            .context_frames("P-root".into())
            .expect("a context producer");
        // Premultiplied working values: half-transparent white, opaque blue.
        let rgba = vec![0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 1.0, 1.0];
        let t0 = pm_protocol::common::Ticks::ZERO;
        let same = post
            .frame(0, t0, 2, 1, WorkingEncoding::Standard, rgba.clone())
            .unwrap();
        assert_eq!(same, rgba, "no handler drew: the frame as rendered");
        {
            let calls = host.calls.lock().unwrap();
            let r = &calls[0];
            assert_eq!(
                (r.source, r.layer.as_str(), r.composition.as_str()),
                (CompatSource::CompatContext, "P-root", "P-root")
            );
            let PixelPayload::Inline { bytes } = &r.frame.as_ref().unwrap().payload else {
                panic!()
            };
            // Today's canvas: premultiplied color over black, opaque.
            assert_eq!(bytes.0, vec![128, 128, 128, 255, 0, 0, 255, 255]);
        }
        let t1 = pm_protocol::common::Ticks::new(23_520_000).unwrap();
        let drawn = post
            .frame(1, t1, 2, 1, WorkingEncoding::Standard, rgba.clone())
            .unwrap();
        assert_eq!(drawn[4..8], [1.0, 0.0, 0.0, 1.0]);
        assert!((drawn[0] - 128.0 / 255.0).abs() < 1e-6 && drawn[3] == 1.0);
        // Linear blending: E-encoded on the way out, decoded on the way back.
        let linear = post
            .frame(
                1,
                t1,
                2,
                1,
                WorkingEncoding::Linear,
                vec![0.2, 0.2, 0.2, 1.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap();
        assert!((f64::from(linear[0]) - 0.2).abs() < 0.004, "{linear:?}");
        // Without a context producer no frame is sent.
        let (_, raster_only) = job(|_| Ok(frame(true)));
        assert!(
            Arc::new(raster_only)
                .context_frames("P-root".into())
                .is_none()
        );
    }
}
